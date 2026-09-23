//! What the Cleanup view asks of the store: who can be unsubscribed from, how
//! much there is to clear out, and which messages look like one another.
//!
//! Also the backfill those answers depend on. Mail synced before the store
//! kept recipients and `List-Unsubscribe` has neither column filled; the raw
//! message is on disk, so the headers are read back from it once, in batches,
//! rather than the answers being wrong for every message older than the
//! upgrade.

use rusqlite::{params, OptionalExtension};

use crate::model::{AccountId, MessageId};
use crate::{Result, Store};

/// Everything one sender has sent that can be unsubscribed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubscribeSender {
    /// What identifies the sender: the `List-Id` when there is one — a list
    /// can change its From address and stay the same list — else the address.
    pub key: String,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub list_id: Option<String>,
    pub messages: usize,
    pub unread: usize,
    pub latest_utc: Option<i64>,
    /// From the most recent message, since a list's links change and the
    /// newest are the ones that still work.
    pub list_unsubscribe: String,
    pub list_unsubscribe_post: Option<String>,
    /// The most recent message, for showing what is being unsubscribed from.
    pub latest_message: MessageId,
    pub latest_subject: Option<String>,
}

/// A recorded unsubscribe attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsubscription {
    pub sender: String,
    /// `one_click`, `mailto` or `browser`.
    pub method: String,
    pub target: String,
    /// `done`, `opened` (handed to the browser, which is as far as this can
    /// see) or `failed`.
    pub state: String,
    pub detail: Option<String>,
    pub created_at: i64,
}

/// A message, as the similarity check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarityRow {
    pub id: MessageId,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub list_id: Option<String>,
    pub date_utc: Option<i64>,
    /// The category the list shows it under.
    pub category: Option<String>,
    /// Set when it answers another message — a conversation, not a run.
    pub in_reply_to: Option<String>,
}

/// The headers read back from a stored message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfilledHeaders {
    pub recipients: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
}

/// Categories whose mail is what a cleanup clears out: sent in bulk, read
/// once if at all.
pub const CLEANUP_CATEGORIES: &[&str] = &["newsletter", "marketing", "notification"];

impl Store {
    /// Messages whose headers have not been read back yet, with where their
    /// raw message is.
    pub fn messages_missing_headers(
        &self,
        account_id: AccountId,
        limit: usize,
    ) -> Result<Vec<(MessageId, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, body_path FROM message
             WHERE account_id = ?1 AND headers_read = 0
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Records what was read back; `None` for a message whose body is gone,
    /// so it is not read again every time.
    pub fn set_backfilled_headers(
        &self,
        updates: &[(MessageId, Option<BackfilledHeaders>)],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE message SET recipients = COALESCE(?2, recipients),
                        list_unsubscribe = COALESCE(?3, list_unsubscribe),
                        list_unsubscribe_post = COALESCE(?4, list_unsubscribe_post),
                        headers_read = 1
                 WHERE id = ?1",
            )?;
            for (id, headers) in updates {
                let headers = headers.clone().unwrap_or_default();
                stmt.execute(params![
                    id,
                    headers.recipients,
                    headers.list_unsubscribe,
                    headers.list_unsubscribe_post
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Everyone who can be unsubscribed from, most prolific first.
    ///
    /// Grouped here rather than in SQL because the grouping key falls back
    /// from list to address, and the row to take links from is the newest in
    /// each group — both easy in Rust and awkward in one statement.
    pub fn unsubscribe_senders(&self, account_id: AccountId) -> Result<Vec<UnsubscribeSender>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.from_name, m.from_addr, m.list_id, m.date_utc, m.list_unsubscribe,
                    m.list_unsubscribe_post, m.subject, {unread}
             FROM message m
             WHERE m.account_id = ?1 AND m.list_unsubscribe IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM operation o
                               WHERE o.message_id = m.id AND o.state = 'pending'
                                 AND o.kind = 'move')
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC",
            unread = crate::UNREAD_PREDICATE
        ))?;

        let mut senders: Vec<UnsubscribeSender> = Vec::new();
        let mut index: std::collections::HashMap<String, usize> = Default::default();
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((
                row.get::<_, MessageId>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })?;
        for row in rows {
            let (id, from_name, from_addr, list_id, date, unsubscribe, post, subject, unread) =
                row?;
            let key = sender_key(list_id.as_deref(), from_addr.as_deref());
            match index.get(&key) {
                Some(&at) => {
                    let sender = &mut senders[at];
                    sender.messages += 1;
                    sender.unread += usize::from(unread);
                }
                None => {
                    index.insert(key.clone(), senders.len());
                    senders.push(UnsubscribeSender {
                        key,
                        from_name,
                        from_addr,
                        list_id,
                        messages: 1,
                        unread: usize::from(unread),
                        latest_utc: date,
                        list_unsubscribe: unsubscribe,
                        list_unsubscribe_post: post,
                        latest_message: id,
                        latest_subject: subject,
                    });
                }
            }
        }
        senders.sort_by(|a, b| b.messages.cmp(&a.messages).then(a.key.cmp(&b.key)));
        Ok(senders)
    }

    /// The sender of one message, as something to unsubscribe from — for the
    /// offer made when that message is deleted. `None` when the message
    /// carries no `List-Unsubscribe`, which is most of them.
    ///
    /// The counts are the sender's whole run, not this one message: the offer
    /// says how much mail stops coming.
    pub fn unsubscribe_for_message(
        &self,
        account_id: AccountId,
        id: MessageId,
    ) -> Result<Option<UnsubscribeSender>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_name, from_addr, list_id, date_utc, list_unsubscribe,
                    list_unsubscribe_post, subject
             FROM message
             WHERE account_id = ?1 AND id = ?2 AND list_unsubscribe IS NOT NULL",
        )?;
        let found = stmt
            .query_row(params![account_id, id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .optional()?;
        let Some((from_name, from_addr, list_id, date, unsubscribe, post, subject)) = found else {
            return Ok(None);
        };

        let key = sender_key(list_id.as_deref(), from_addr.as_deref());
        let (messages, unread) = self.count_from_sender(account_id, &key)?;
        Ok(Some(UnsubscribeSender {
            key,
            from_name,
            from_addr,
            list_id,
            messages,
            unread,
            latest_utc: date,
            list_unsubscribe: unsubscribe,
            list_unsubscribe_post: post,
            latest_message: id,
            latest_subject: subject,
        }))
    }

    /// How much mail one sender has here, and how much of it is unread —
    /// counted the same way `unsubscribe_senders` counts it, the messages
    /// carrying the header included and no others.
    fn count_from_sender(&self, account_id: AccountId, key: &str) -> Result<(usize, usize)> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT COUNT(*), COALESCE(SUM({unread}), 0)
             FROM message m
             WHERE m.account_id = ?1 AND m.list_unsubscribe IS NOT NULL
               AND (m.list_id = ?2 OR (m.list_id IS NULL AND lower(m.from_addr) = ?2))",
            unread = crate::UNREAD_PREDICATE
        ))?;
        let counted = stmt.query_row(params![account_id, key], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok((counted.0.max(0) as usize, counted.1.max(0) as usize))
    }

    /// The messages `unsubscribe_senders` counted for one sender, and no
    /// others: those carrying `List-Unsubscribe`. A person whose mail tool
    /// added the header to one newsletter shows as one message, and must not
    /// lose the forty they wrote by hand when that one is cleared out.
    pub fn messages_from_sender(&self, account_id: AccountId, key: &str) -> Result<Vec<MessageId>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM message
             WHERE account_id = ?1 AND list_unsubscribe IS NOT NULL
               AND (list_id = ?2 OR (list_id IS NULL AND lower(from_addr) = ?2))
             ORDER BY COALESCE(date_utc, 0) DESC",
        )?;
        let rows = stmt.query_map(params![account_id, key], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn record_unsubscription(
        &self,
        account_id: AccountId,
        attempt: &Unsubscription,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO unsubscription (account_id, sender, method, target, state, detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account_id,
                attempt.sender,
                attempt.method,
                attempt.target,
                attempt.state,
                attempt.detail,
                attempt.created_at
            ],
        )?;
        Ok(())
    }

    /// The latest attempt for each sender on an account.
    pub fn unsubscriptions(&self, account_id: AccountId) -> Result<Vec<Unsubscription>> {
        let mut stmt = self.conn.prepare(
            "SELECT sender, method, target, state, detail, created_at FROM unsubscription u
             WHERE account_id = ?1
               AND id = (SELECT MAX(id) FROM unsubscription v
                         WHERE v.account_id = u.account_id AND v.sender = u.sender)
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(Unsubscription {
                sender: row.get(0)?,
                method: row.get(1)?,
                target: row.get(2)?,
                state: row.get(3)?,
                detail: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// How many messages in the Inbox are bulk mail to be cleared out.
    ///
    /// The Inbox only: a newsletter already archived has been dealt with,
    /// and counting it would make a number that never goes down.
    pub fn cleanup_count(&self, account_id: AccountId) -> Result<usize> {
        let placeholders = CLEANUP_CATEGORIES
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let count: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM message m
                 {join}
                 WHERE m.account_id = ?1 AND c.category IN ({placeholders})
                   AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                               WHERE l.message_id = m.id AND upper(f.name) = 'INBOX')
                   AND NOT EXISTS (SELECT 1 FROM operation o
                                   WHERE o.message_id = m.id AND o.state = 'pending'
                                     AND o.kind = 'move')",
                join = crate::CURRENT_CATEGORY_JOIN
            ),
            params![account_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// The messages a similarity check compares against: everything on the
    /// account with a copy outside `skip` — the Trash, Sent, Drafts — and not
    /// on its way somewhere, newest first.
    pub fn similarity_candidates(
        &self,
        account_id: AccountId,
        skip: &[String],
        limit: usize,
    ) -> Result<Vec<SimilarityRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SIMILARITY_COLUMNS}
             FROM message m
             {join}
             WHERE m.account_id = ?1
               AND EXISTS (SELECT 1 FROM message_location l JOIN folder f ON f.id = l.folder_id
                           WHERE l.message_id = m.id
                             AND f.name NOT IN (SELECT value FROM json_each(?2)))
               AND NOT EXISTS (SELECT 1 FROM operation o
                               WHERE o.message_id = m.id AND o.state = 'pending'
                                 AND o.kind = 'move')
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT ?3",
            join = crate::CURRENT_CATEGORY_JOIN
        ))?;
        let skip = serde_json::to_string(skip).unwrap_or_else(|_| "[]".into());
        let rows = stmt.query_map(params![account_id, skip, limit as i64], row_to_similarity)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// One message as the similarity check reads it, wherever it is.
    pub fn similarity_row(
        &self,
        account_id: AccountId,
        id: MessageId,
    ) -> Result<Option<SimilarityRow>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {SIMILARITY_COLUMNS} FROM message m {join}
                     WHERE m.account_id = ?1 AND m.id = ?2",
                    join = crate::CURRENT_CATEGORY_JOIN
                ),
                params![account_id, id],
                row_to_similarity,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Forgets a folder the server no longer has, with the copies in it.
    /// Messages left in no folder are collected by the next sync, as ever.
    pub fn delete_folder(&self, folder_id: crate::model::FolderId) -> Result<()> {
        self.conn
            .execute("DELETE FROM folder WHERE id = ?1", params![folder_id])?;
        Ok(())
    }
}

const SIMILARITY_COLUMNS: &str = "m.id, m.from_name, m.from_addr, m.subject, m.snippet, \
     m.list_id, m.date_utc, c.category, m.in_reply_to";

fn row_to_similarity(row: &rusqlite::Row<'_>) -> rusqlite::Result<SimilarityRow> {
    Ok(SimilarityRow {
        id: row.get(0)?,
        from_name: row.get(1)?,
        from_addr: row.get(2)?,
        subject: row.get(3)?,
        snippet: row.get(4)?,
        list_id: row.get(5)?,
        date_utc: row.get(6)?,
        category: row.get(7)?,
        in_reply_to: row.get(8)?,
    })
}

/// How `unsubscribe_senders` groups: by list, else by lowercased address.
pub fn sender_key(list_id: Option<&str>, from_addr: Option<&str>) -> String {
    match list_id {
        Some(list) if !list.is_empty() => list.to_string(),
        _ => from_addr.unwrap_or_default().to_lowercase(),
    }
}
