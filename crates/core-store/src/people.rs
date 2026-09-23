//! Mail seen by who it is with, and by how soon it needs an answer.
//!
//! **Conversations** group every message on an account by the other person:
//! the sender of what arrived, the first recipient of what was sent. That is
//! what a messenger shows — a list of people, and under each the exchange in
//! order — and the store can answer it without keeping anything new, because
//! mail sent from any client comes back through the Sent folder with the
//! account's own address as its sender.
//!
//! **Urgency** is a verdict per message: how soon it needs acting on, what
//! the action is, and why, from the rules or from a model. Like the
//! classifier's verdicts it is advice — it orders one view and moves nothing.

use rusqlite::{params, OptionalExtension};

use crate::model::{AccountId, MessageId};
use crate::{Result, Store};

/// One person, as the conversation list shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// The other person's address, lowercased.
    pub key: String,
    /// Their name as they last wrote it, when they did.
    pub name: Option<String>,
    pub messages: usize,
    pub unread: usize,
    pub last_utc: Option<i64>,
    pub last_subject: Option<String>,
    pub last_snippet: Option<String>,
    /// Whether the last word was the account's own.
    pub last_from_me: bool,
}

/// One page of conversations.
#[derive(Debug, Clone)]
pub struct ConversationWindow {
    pub total: usize,
    pub offset: usize,
    pub conversations: Vec<Conversation>,
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub id: MessageId,
    pub from_me: bool,
    pub from_name: Option<String>,
    pub date_utc: Option<i64>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub unread: bool,
    pub body_path: Option<String>,
    pub rfc822_message_id: Option<String>,
}

/// What is known about one correspondent, for the card that summarises them.
///
/// Counts are of the mail that the conversation view counts — the Trash and
/// the Junk left out — so the card and the conversation list never disagree
/// about how much mail there is with somebody.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Correspondent {
    /// Their address, lowercased.
    pub address: String,
    /// Their name as they last wrote it.
    pub name: Option<String>,
    /// Messages from them, and messages the account sent them.
    pub received: usize,
    pub sent: usize,
    /// Of theirs, how many are unread.
    pub unread: usize,
    /// When the first message either way is dated, and the last in each
    /// direction: together these are "since when" and "who spoke last".
    pub first_utc: Option<i64>,
    pub last_from_them_utc: Option<i64>,
    pub last_to_them_utc: Option<i64>,
    /// How many of their messages carry an attachment.
    pub with_attachments: usize,
    /// What their mail is most often classified as, and how much of it is.
    pub usual_category: Option<(String, usize)>,
    /// Where their mail sits, the fullest folder first.
    pub folders: Vec<(String, usize)>,
    /// The most pressing verdict on their Inbox mail, when there is one.
    pub waiting: Option<Urgency>,
}

/// How soon a message needs acting on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urgency {
    /// 0 nothing to do, 1 can wait, 2 this week, 3 today.
    pub score: i64,
    pub reason: String,
    /// `reply`, `pay`, `attend`, `decide`, `read` or `none`.
    pub action: Option<String>,
    /// `YYYY-MM-DD`, when the message names one.
    pub deadline: Option<String>,
    /// `rules` or `model`.
    pub source: String,
    pub model: Option<String>,
}

/// A message waiting for an urgency verdict, with what the rules need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrgencyCandidate {
    pub id: MessageId,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub date_utc: Option<i64>,
    pub category: Option<String>,
    pub recipients: Option<String>,
    pub rfc822_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub body_path: Option<String>,
    pub unread: bool,
}

/// A message with its urgency, for the view that ranks them.
#[derive(Debug, Clone)]
pub struct UrgentMessage {
    pub summary: crate::MessageSummary,
    pub category: Option<String>,
    pub unread: bool,
    pub urgency: Urgency,
}

/// The other person in a message: its sender, or when the account sent it,
/// its first recipient. `?1` is the account's own address, lowercased.
const COUNTERPART: &str = "CASE WHEN lower(m.from_addr) = ?1
          THEN CASE WHEN instr(m.recipients, ',') > 0
                    THEN trim(substr(m.recipients, 1, instr(m.recipients, ',') - 1))
                    ELSE trim(m.recipients) END
          ELSE lower(m.from_addr) END";

/// Messages that belong in the conversation view: in some folder that is not
/// one of `?3` (JSON: the Trash, Junk…), and, unless `?4`, not bulk mail —
/// a newsletter is not somebody to talk to.
fn conversation_filter() -> String {
    format!(
        "m.account_id = ?2
         AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                     WHERE l.message_id = m.id
                       AND f.name NOT IN (SELECT value FROM json_each(?3)))
         AND NOT EXISTS (SELECT 1 FROM operation o
                         WHERE o.message_id = m.id AND o.state = 'pending'
                           AND o.kind = 'move'
                           AND o.target_folder IN (SELECT value FROM json_each(?3)))
         AND (?4 OR lower(m.from_addr) = ?1
              OR COALESCE(c.category, 'unknown') NOT IN ({bulk}))",
        bulk = crate::CLEANUP_CATEGORIES
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

impl Store {
    /// People the account has mail with, most recent first. `query`, when
    /// given, narrows to names and addresses containing it.
    #[allow(clippy::too_many_arguments)]
    pub fn conversations(
        &self,
        account_id: AccountId,
        me: &str,
        skip_folders: &[String],
        include_bulk: bool,
        query: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<ConversationWindow> {
        let skip = serde_json::to_string(skip_folders).unwrap_or_else(|_| "[]".into());
        let me = me.to_lowercase();
        let needle = query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        let grouped = format!(
            "SELECT {COUNTERPART} AS who,
                    MAX(CASE WHEN lower(m.from_addr) <> ?1 THEN m.from_name END) AS name,
                    COUNT(*) AS messages,
                    SUM(CASE WHEN lower(m.from_addr) <> ?1 AND {unread} THEN 1 ELSE 0 END) AS unread,
                    MAX(COALESCE(m.date_utc, 0)) AS last_utc
             FROM message m
             {join}
             WHERE {filter}
             GROUP BY who
             HAVING who IS NOT NULL AND who <> ''
                AND (?5 IS NULL OR instr(kuverta_lower(who || ' ' || COALESCE(name, '')), ?5) > 0)",
            unread = crate::UNREAD_PREDICATE,
            join = crate::CURRENT_CATEGORY_JOIN,
            filter = conversation_filter(),
        );

        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM ({grouped})"),
            params![me, account_id, skip, include_bulk, needle],
            |row| row.get(0),
        )?;

        let mut stmt = self.conn.prepare(&format!(
            "{grouped}
             ORDER BY last_utc DESC, who
             LIMIT ?6 OFFSET ?7"
        ))?;
        let rows = stmt.query_map(
            params![
                me,
                account_id,
                skip,
                include_bulk,
                needle,
                limit as i64,
                offset as i64
            ],
            |row| {
                Ok(Conversation {
                    key: row.get(0)?,
                    name: row.get(1)?,
                    messages: row.get::<_, i64>(2)? as usize,
                    unread: row.get::<_, i64>(3)? as usize,
                    last_utc: row.get::<_, Option<i64>>(4)?.filter(|t| *t > 0),
                    last_subject: None,
                    last_snippet: None,
                    last_from_me: false,
                })
            },
        )?;
        let mut conversations = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        // The last message of each, for the line under the name. One query
        // per row of a page — a page is a screenful.
        let mut last = self.conn.prepare(&format!(
            "SELECT m.subject, m.snippet, lower(m.from_addr) = ?1
             FROM message m
             {join}
             WHERE {filter} AND {COUNTERPART} = ?5
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT 1",
            join = crate::CURRENT_CATEGORY_JOIN,
            filter = conversation_filter(),
        ))?;
        for conversation in &mut conversations {
            if let Some((subject, snippet, from_me)) = last
                .query_row(
                    params![me, account_id, skip, include_bulk, conversation.key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
            {
                conversation.last_subject = subject;
                conversation.last_snippet = snippet;
                conversation.last_from_me = from_me;
            }
        }

        Ok(ConversationWindow {
            total: total as usize,
            offset,
            conversations,
        })
    }

    /// The last `limit` messages with one person, oldest first.
    pub fn conversation(
        &self,
        account_id: AccountId,
        me: &str,
        key: &str,
        skip_folders: &[String],
        limit: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let skip = serde_json::to_string(skip_folders).unwrap_or_else(|_| "[]".into());
        let me = me.to_lowercase();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT * FROM (
                 SELECT m.id, lower(m.from_addr) = ?1, m.from_name, m.date_utc, m.subject,
                        m.snippet, {unread}, m.body_path, m.rfc822_message_id
                 FROM message m
                 {join}
                 WHERE {filter} AND {COUNTERPART} = ?5
                 ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
                 LIMIT ?6)
             ORDER BY COALESCE(date_utc, 0), id",
            unread = crate::UNREAD_PREDICATE,
            join = crate::CURRENT_CATEGORY_JOIN,
            filter = conversation_filter(),
        ))?;
        let rows = stmt.query_map(
            params![me, account_id, skip, true, key.to_lowercase(), limit as i64],
            |row| {
                Ok(ConversationMessage {
                    id: row.get(0)?,
                    from_me: row.get(1)?,
                    from_name: row.get(2)?,
                    date_utc: row.get(3)?,
                    subject: row.get(4)?,
                    snippet: row.get(5)?,
                    unread: row.get(6)?,
                    body_path: row.get(7)?,
                    rfc822_message_id: row.get(8)?,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -- urgency -----------------------------------------------------------

    /// Inbox mail from others since `since_utc`, not bulk, that has no
    /// verdict yet — or only the rules' when `model` asks for a model's.
    pub fn urgency_candidates(
        &self,
        account_id: AccountId,
        me: &str,
        since_utc: i64,
        want_model: bool,
        limit: usize,
    ) -> Result<Vec<UrgencyCandidate>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.from_name, m.from_addr, m.subject, m.snippet, m.date_utc,
                    c.category, m.recipients, m.rfc822_message_id, m.in_reply_to, m.body_path,
                    {unread}
             FROM message m
             {join}
             LEFT JOIN urgency u ON u.message_id = m.id
             WHERE m.account_id = ?1 AND lower(COALESCE(m.from_addr, '')) <> ?2
               AND COALESCE(m.date_utc, 0) >= ?3
               AND COALESCE(c.category, 'unknown') NOT IN ({bulk})
               AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                           WHERE l.message_id = m.id AND upper(f.name) = 'INBOX')
               AND (u.message_id IS NULL OR (?4 AND u.source = 'rules'))
             ORDER BY COALESCE(m.date_utc, 0) DESC
             LIMIT ?5",
            unread = crate::UNREAD_PREDICATE,
            join = crate::CURRENT_CATEGORY_JOIN,
            bulk = crate::CLEANUP_CATEGORIES
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let rows = stmt.query_map(
            params![
                account_id,
                me.to_lowercase(),
                since_utc,
                want_model,
                limit as i64
            ],
            |row| {
                Ok(UrgencyCandidate {
                    id: row.get(0)?,
                    from_name: row.get(1)?,
                    from_addr: row.get(2)?,
                    subject: row.get(3)?,
                    snippet: row.get(4)?,
                    date_utc: row.get(5)?,
                    category: row.get(6)?,
                    recipients: row.get(7)?,
                    rfc822_message_id: row.get(8)?,
                    in_reply_to: row.get(9)?,
                    body_path: row.get(10)?,
                    unread: row.get(11)?,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Records a verdict, replacing any earlier one: a model's answer is
    /// the better-informed of the two and is asked for after the rules'.
    pub fn record_urgency(&self, message_id: MessageId, urgency: &Urgency) -> Result<()> {
        self.conn.execute(
            "INSERT INTO urgency (message_id, score, reason, action, deadline, source, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (message_id) DO UPDATE SET
                 score = excluded.score, reason = excluded.reason, action = excluded.action,
                 deadline = excluded.deadline, source = excluded.source, model = excluded.model,
                 created_at = excluded.created_at",
            params![
                message_id,
                urgency.score.clamp(0, 3),
                urgency.reason,
                urgency.action,
                urgency.deadline,
                urgency.source,
                urgency.model,
                crate::now()
            ],
        )?;
        Ok(())
    }

    /// Inbox mail rated at least `min_score`, most pressing first: by score,
    /// then the nearest deadline, then the newest.
    pub fn urgent_messages(
        &self,
        account_id: AccountId,
        min_score: i64,
        limit: usize,
    ) -> Result<Vec<UrgentMessage>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, m.snippet, c.category, {unread},
                    u.score, u.reason, u.action, u.deadline, u.source, u.model
             FROM message m
             {join}
             JOIN urgency u ON u.message_id = m.id
             WHERE m.account_id = ?1 AND u.score >= ?2
               AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                           WHERE l.message_id = m.id AND upper(f.name) = 'INBOX')
               AND NOT EXISTS (SELECT 1 FROM operation o
                               WHERE o.message_id = m.id AND o.state = 'pending'
                                 AND o.kind = 'move')
             ORDER BY u.score DESC, u.deadline IS NULL, u.deadline,
                      COALESCE(m.date_utc, 0) DESC
             LIMIT ?3",
            unread = crate::UNREAD_PREDICATE,
            join = crate::CURRENT_CATEGORY_JOIN,
        ))?;
        let rows = stmt.query_map(params![account_id, min_score, limit as i64], |row| {
            Ok(UrgentMessage {
                summary: crate::MessageSummary {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    from_name: row.get(2)?,
                    from_addr: row.get(3)?,
                    date_utc: row.get(4)?,
                    list_id: row.get(5)?,
                    has_attachments: row.get(6)?,
                    snippet: row.get(7)?,
                },
                category: row.get(8)?,
                unread: row.get(9)?,
                urgency: Urgency {
                    score: row.get(10)?,
                    reason: row.get(11)?,
                    action: row.get(12)?,
                    deadline: row.get(13)?,
                    source: row.get(14)?,
                    model: row.get(15)?,
                },
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// One message's verdict, for the reading pane.
    pub fn urgency_of(&self, message_id: MessageId) -> Result<Option<Urgency>> {
        self.conn
            .query_row(
                "SELECT score, reason, action, deadline, source, model FROM urgency
                 WHERE message_id = ?1",
                params![message_id],
                |row| {
                    Ok(Urgency {
                        score: row.get(0)?,
                        reason: row.get(1)?,
                        action: row.get(2)?,
                        deadline: row.get(3)?,
                        source: row.get(4)?,
                        model: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// How often the account has written to `address`: someone written to is
    /// someone whose mail is expected to matter.
    pub fn times_written_to(
        &self,
        account_id: AccountId,
        me: &str,
        address: &str,
    ) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM message
             WHERE account_id = ?1 AND lower(from_addr) = ?2
               AND instr(COALESCE(recipients, ''), ?3) > 0",
            params![account_id, me.to_lowercase(), address.to_lowercase()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Whether the account has answered the message with this Message-ID.
    pub fn has_replied(&self, account_id: AccountId, me: &str, message_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM message
                 WHERE account_id = ?1 AND lower(from_addr) = ?2 AND in_reply_to = ?3
                 LIMIT 1",
                params![account_id, me.to_lowercase(), message_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The other person in one message: its sender, or, when the account
    /// sent it, its first recipient — the same rule the conversation list
    /// groups by, so a card opened from a message and the conversation it
    /// belongs to are about the same person.
    pub fn counterpart_of(
        &self,
        account_id: AccountId,
        me: &str,
        id: MessageId,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {COUNTERPART} FROM message m WHERE m.account_id = ?2 AND m.id = ?3"
                ),
                params![me.to_lowercase(), account_id, id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|found| {
                found
                    .flatten()
                    .map(|who| who.trim().to_lowercase())
                    .filter(|who| !who.is_empty())
            })
            .map_err(Into::into)
    }

    /// Everything the store knows about one correspondent.
    ///
    /// Four small queries rather than one wide one: the counts, where the
    /// mail is filed, what it usually is, and whether any of it is waiting.
    /// Each is a different grain, and joining them into a single statement
    /// would multiply the rows of one by the rows of another.
    pub fn correspondent(
        &self,
        account_id: AccountId,
        me: &str,
        address: &str,
        skip_folders: &[String],
    ) -> Result<Correspondent> {
        let skip = serde_json::to_string(skip_folders).unwrap_or_else(|_| "[]".into());
        let me = me.to_lowercase();
        let who = address.trim().to_lowercase();
        let filter = conversation_filter();
        let join = crate::CURRENT_CATEGORY_JOIN;
        let unread = crate::UNREAD_PREDICATE;
        // Bulk mail is counted here: the card is asked about whoever sent the
        // message on screen, and refusing to describe a newsletter because it
        // is a newsletter would leave the card blank exactly where the
        // question "who is this?" is most often asked.
        let args = params![me, account_id, skip, true, who];

        let mut summary = self.conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(CASE WHEN lower(m.from_addr) <> ?1 THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN lower(m.from_addr) =  ?1 THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN lower(m.from_addr) <> ?1 AND {unread}
                                          THEN 1 ELSE 0 END), 0),
                        MIN(NULLIF(COALESCE(m.date_utc, 0), 0)),
                        MAX(CASE WHEN lower(m.from_addr) <> ?1 THEN m.date_utc END),
                        MAX(CASE WHEN lower(m.from_addr) =  ?1 THEN m.date_utc END),
                        COALESCE(SUM(CASE WHEN lower(m.from_addr) <> ?1 AND m.has_attachments
                                          THEN 1 ELSE 0 END), 0),
                        MAX(CASE WHEN lower(m.from_addr) <> ?1 THEN m.from_name END)
                 FROM message m
                 {join}
                 WHERE {filter} AND {COUNTERPART} = ?5"
            ),
            args,
            |row| {
                Ok(Correspondent {
                    address: who.clone(),
                    received: row.get::<_, i64>(0)? as usize,
                    sent: row.get::<_, i64>(1)? as usize,
                    unread: row.get::<_, i64>(2)? as usize,
                    first_utc: row.get(3)?,
                    last_from_them_utc: row.get(4)?,
                    last_to_them_utc: row.get(5)?,
                    with_attachments: row.get::<_, i64>(6)? as usize,
                    name: row
                        .get::<_, Option<String>>(7)?
                        .filter(|name| !name.trim().is_empty()),
                    ..Default::default()
                })
            },
        )?;

        let mut folders = self.conn.prepare(&format!(
            "SELECT f.name, COUNT(DISTINCT m.id)
             FROM message m
             {join}
             JOIN message_location l ON l.message_id = m.id
             JOIN folder f ON f.id = l.folder_id
             WHERE {filter} AND {COUNTERPART} = ?5
               AND f.name NOT IN (SELECT value FROM json_each(?3))
             GROUP BY f.name
             ORDER BY 2 DESC, 1
             LIMIT 4"
        ))?;
        summary.folders = folders
            .query_map(args, |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
            })?
            .collect::<rusqlite::Result<Vec<(String, usize)>>>()?;

        summary.usual_category = self
            .conn
            .query_row(
                &format!(
                    "SELECT c.category, COUNT(*)
                     FROM message m
                     {join}
                     WHERE {filter} AND {COUNTERPART} = ?5
                       AND lower(m.from_addr) <> ?1 AND c.category IS NOT NULL
                     GROUP BY c.category
                     ORDER BY 2 DESC, 1
                     LIMIT 1"
                ),
                args,
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as usize)),
            )
            .optional()?;

        // What of theirs is still in the Inbox and rated at least "this week",
        // which is the line the reading pane shows for a single message.
        summary.waiting = self
            .conn
            .query_row(
                "SELECT u.score, u.reason, u.action, u.deadline, u.source, u.model
                 FROM message m
                 JOIN urgency u ON u.message_id = m.id
                 WHERE m.account_id = ?1 AND lower(COALESCE(m.from_addr, '')) = ?2
                   AND u.score >= 2
                   AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                               WHERE l.message_id = m.id AND upper(f.name) = 'INBOX')
                   AND NOT EXISTS (SELECT 1 FROM operation o
                                   WHERE o.message_id = m.id AND o.state = 'pending'
                                     AND o.kind = 'move')
                 ORDER BY u.score DESC, u.deadline IS NULL, u.deadline,
                          COALESCE(m.date_utc, 0) DESC
                 LIMIT 1",
                params![account_id, who],
                |row| {
                    Ok(Urgency {
                        score: row.get(0)?,
                        reason: row.get(1)?,
                        action: row.get(2)?,
                        deadline: row.get(3)?,
                        source: row.get(4)?,
                        model: row.get(5)?,
                    })
                },
            )
            .optional()?;

        Ok(summary)
    }

    /// How many messages have come from `address` before `before_utc`.
    pub fn earlier_from(
        &self,
        account_id: AccountId,
        address: &str,
        before_utc: i64,
    ) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM message
             WHERE account_id = ?1 AND lower(from_addr) = ?2 AND COALESCE(date_utc, 0) < ?3",
            params![account_id, address.to_lowercase(), before_utc],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}
