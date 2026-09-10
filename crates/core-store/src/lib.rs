//! Local message store: SQLite for metadata and search, blobs on disk.
//!
//! The store is deliberately synchronous. Sync work is I/O bound on the network
//! side, not on SQLite, and a blocking connection avoids an async SQLite layer
//! that would buy nothing at personal-mailbox scale. Async callers should wrap
//! store calls in `spawn_blocking`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

pub mod blobs;
pub mod dedup;
pub mod model;
mod schema;

pub use blobs::Blobs;
pub use dedup::{dedup_key, normalize_message_id};
pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("blob store: {0}")]
    Io(#[from] std::io::Error),

    #[error(
        "database schema is version {found}, but this build only understands {supported}; \
         it was written by a newer version of fuckmail"
    )]
    SchemaTooNew { found: usize, supported: usize },

    #[error("SQLite was built without FTS5 support, which the message store requires")]
    MissingFts5,

    #[error("no account with id {0}")]
    UnknownAccount(AccountId),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// In-memory store, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL keeps the UI readable while a sync writes. It is a no-op for
        // in-memory databases, which return "memory" instead of "wal".
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        ensure_fts5(&conn)?;
        schema::migrate(&conn)?;

        Ok(Self { conn })
    }

    /// Escape hatch for callers that need raw SQL (reporting, one-off queries).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // -- accounts ---------------------------------------------------------

    pub fn add_account(&self, account: &NewAccount) -> Result<AccountId> {
        self.conn.execute(
            "INSERT INTO account
                 (label, email, imap_host, imap_port, imap_security, username, auth_method,
                  oauth_client_id, oauth_tenant, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                account.label,
                account.email,
                account.imap_host,
                account.imap_port,
                account.imap_security.as_str(),
                account.username,
                account.auth_method,
                account.oauth_client_id,
                account.oauth_tenant,
                now(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn account_by_email(&self, email: &str) -> Result<Option<Account>> {
        self.conn
            .query_row(
                "SELECT id, label, email, imap_host, imap_port, imap_security, username,
                        auth_method, oauth_client_id, oauth_tenant
                 FROM account WHERE email = ?1",
                params![email],
                row_to_account,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, email, imap_host, imap_port, imap_security, username,
                    auth_method, oauth_client_id, oauth_tenant
             FROM account ORDER BY id",
        )?;
        let rows = stmt.query_map([], row_to_account)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -- folders ----------------------------------------------------------

    /// Inserts the folder, or returns the existing id if the account already
    /// has one by that name. Sync state is left untouched on an existing row.
    pub fn upsert_folder(
        &self,
        account_id: AccountId,
        name: &str,
        special_use: Option<&str>,
    ) -> Result<FolderId> {
        self.conn.execute(
            "INSERT INTO folder (account_id, name, special_use)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, name)
             DO UPDATE SET special_use = COALESCE(excluded.special_use, folder.special_use)",
            params![account_id, name, special_use],
        )?;

        self.conn
            .query_row(
                "SELECT id FROM folder WHERE account_id = ?1 AND name = ?2",
                params![account_id, name],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn folders(&self, account_id: AccountId) -> Result<Vec<Folder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, name, special_use, uid_validity, uid_next, highest_modseq
             FROM folder WHERE account_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(Folder {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                special_use: row.get(3)?,
                uid_validity: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                uid_next: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                highest_modseq: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Records the IMAP sync state after a successful pass over a folder.
    pub fn set_folder_sync_state(
        &self,
        folder_id: FolderId,
        uid_validity: Option<u32>,
        uid_next: Option<u32>,
        highest_modseq: Option<u64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE folder
             SET uid_validity = ?2, uid_next = ?3, highest_modseq = ?4, last_synced_at = ?5
             WHERE id = ?1",
            params![
                folder_id,
                uid_validity.map(|v| v as i64),
                uid_next.map(|v| v as i64),
                highest_modseq.map(|v| v as i64),
                now(),
            ],
        )?;
        Ok(())
    }

    /// Drops every cached UID for a folder. Called when `UIDVALIDITY` changes,
    /// which means the server has renumbered and nothing cached can be trusted.
    ///
    /// Messages themselves survive if they are still referenced from another
    /// folder; those left with no location at all are removed.
    pub fn invalidate_folder(&self, folder_id: FolderId) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let removed = tx.execute(
            "DELETE FROM message_location WHERE folder_id = ?1",
            params![folder_id],
        )?;
        tx.execute(
            "DELETE FROM message
             WHERE id NOT IN (SELECT message_id FROM message_location)",
            [],
        )?;
        tx.execute(
            "UPDATE folder SET uid_next = NULL, highest_modseq = NULL WHERE id = ?1",
            params![folder_id],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    // -- messages ---------------------------------------------------------

    /// Stores a message and, optionally, the folder location it was seen at.
    ///
    /// If the account already holds a message with the same identity (see
    /// [`dedup`]) no new message row is created — only the location is added.
    /// This is the path Gmail's labels take.
    pub fn upsert_message(
        &self,
        account_id: AccountId,
        message: &NewMessage,
        location: Option<&Location>,
    ) -> Result<(MessageId, Upsert)> {
        let key = dedup_key(message);
        let tx = self.conn.unchecked_transaction()?;

        let existing: Option<MessageId> = tx
            .query_row(
                "SELECT id FROM message WHERE account_id = ?1 AND dedup_key = ?2",
                params![account_id, key],
                |row| row.get(0),
            )
            .optional()?;

        let (id, outcome) = match existing {
            Some(id) => (id, Upsert::Deduplicated),
            None => {
                tx.execute(
                    "INSERT INTO message
                         (account_id, dedup_key, rfc822_message_id, subject, from_name, from_addr,
                          date_utc, size_bytes, snippet, has_attachments, list_id, in_reply_to,
                          body_path, first_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        account_id,
                        key,
                        message.rfc822_message_id,
                        message.subject,
                        message.from_name,
                        message.from_addr,
                        message.date_utc,
                        message.size_bytes,
                        message.snippet,
                        message.has_attachments,
                        message.list_id,
                        message.in_reply_to,
                        message.body_path,
                        now(),
                    ],
                )?;
                let id = tx.last_insert_rowid();

                tx.execute(
                    "INSERT INTO message_fts (rowid, subject, sender, body)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        id,
                        message.subject.as_deref().unwrap_or(""),
                        message.from_addr.as_deref().unwrap_or(""),
                        message.search_text.as_deref().unwrap_or(""),
                    ],
                )?;

                (id, Upsert::Inserted)
            }
        };

        if let Some(loc) = location {
            // A re-sync legitimately re-reports the same (folder, uid); take the
            // newer flags rather than failing.
            tx.execute(
                "INSERT INTO message_location (message_id, folder_id, uid, flags)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (folder_id, uid)
                 DO UPDATE SET flags = excluded.flags, message_id = excluded.message_id",
                params![id, loc.folder_id, loc.uid, loc.flags],
            )?;
        }

        tx.commit()?;
        Ok((id, outcome))
    }

    pub fn locations_of(&self, message_id: MessageId) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, uid, flags FROM message_location
             WHERE message_id = ?1 ORDER BY folder_id, uid",
        )?;
        let rows = stmt.query_map(params![message_id], |row| {
            Ok(Location {
                folder_id: row.get(0)?,
                uid: row.get::<_, i64>(1)? as u32,
                flags: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Highest UID already stored for a folder, or `None` when it is empty.
    /// Used to fetch only what is new.
    pub fn max_uid(&self, folder_id: FolderId) -> Result<Option<u32>> {
        let max: Option<i64> = self.conn.query_row(
            "SELECT MAX(uid) FROM message_location WHERE folder_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )?;
        Ok(max.map(|v| v as u32))
    }

    /// Looks up a single folder.
    pub fn folder(&self, folder_id: FolderId) -> Result<Option<Folder>> {
        self.conn
            .query_row(
                "SELECT id, account_id, name, special_use, uid_validity, uid_next, highest_modseq
                 FROM folder WHERE id = ?1",
                params![folder_id],
                |row| {
                    Ok(Folder {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        name: row.get(2)?,
                        special_use: row.get(3)?,
                        uid_validity: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                        uid_next: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                        highest_modseq: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Every UID currently cached for a folder.
    ///
    /// Used to work out what the server has expunged: anything here that the
    /// server no longer lists is gone.
    pub fn folder_uids(&self, folder_id: FolderId) -> Result<Vec<u32>> {
        let mut stmt = self
            .conn
            .prepare("SELECT uid FROM message_location WHERE folder_id = ?1")?;
        let rows = stmt.query_map(params![folder_id], |row| Ok(row.get::<_, i64>(0)? as u32))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Updates the flags on a stored location.
    ///
    /// Returns whether anything actually changed, so a sync can report real
    /// flag churn rather than counting every message the server re-reported.
    pub fn set_location_flags(&self, folder_id: FolderId, uid: u32, flags: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE message_location SET flags = ?3
             WHERE folder_id = ?1 AND uid = ?2 AND flags <> ?3",
            params![folder_id, uid, flags],
        )?;
        Ok(changed > 0)
    }

    /// Removes locations the server no longer has, then deletes any message
    /// left with no location at all.
    ///
    /// A message expunged from one folder but still present in another keeps
    /// its remaining location and stays in the store — the same property that
    /// makes Gmail labels work.
    ///
    /// Note: the message's body blob is not removed here. Blob pruning is a
    /// separate sweep, so a partial sync cannot delete a body that a surviving
    /// location still points at.
    pub fn remove_locations(&self, folder_id: FolderId, uids: &[u32]) -> Result<usize> {
        if uids.is_empty() {
            return Ok(0);
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut removed = 0;
        {
            let mut stmt =
                tx.prepare("DELETE FROM message_location WHERE folder_id = ?1 AND uid = ?2")?;
            for uid in uids {
                removed += stmt.execute(params![folder_id, *uid])?;
            }
        }
        tx.execute(
            "DELETE FROM message
             WHERE id NOT IN (SELECT message_id FROM message_location)",
            [],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn message_count(&self, account_id: AccountId) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM message WHERE account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn location_count(&self, account_id: AccountId) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM message_location l
                 JOIN folder f ON f.id = l.folder_id
                 WHERE f.account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Most recent messages for an account, newest first.
    pub fn recent(&self, account_id: AccountId, limit: usize) -> Result<Vec<MessageSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, subject, from_name, from_addr, date_utc, list_id, has_attachments
             FROM message WHERE account_id = ?1
             ORDER BY COALESCE(date_utc, 0) DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], row_to_summary)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Recent messages, each with the most recent rules verdict recorded for
    /// it. Messages never classified come back with `category: None`.
    pub fn recent_with_category(
        &self,
        account_id: AccountId,
        limit: usize,
    ) -> Result<Vec<CategorizedMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, c.category, c.confidence
             FROM message m
             LEFT JOIN classification c
               ON c.message_id = m.id
              AND c.source = 'rules'
              AND c.id = (
                  SELECT MAX(c2.id) FROM classification c2
                  WHERE c2.message_id = m.id AND c2.source = 'rules'
              )
             WHERE m.account_id = ?1
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |row| {
            Ok(CategorizedMessage {
                summary: row_to_summary(row)?,
                category: row.get(7)?,
                confidence: row.get(8)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Full-text search over subject, sender and body.
    pub fn search(
        &self,
        account_id: AccountId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        // FTS5 treats a bare string as query syntax; quoting makes user input a
        // literal phrase, so a stray `"` or `*` cannot become an operator.
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));

        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id, m.has_attachments
             FROM message_fts f
             JOIN message m ON m.id = f.rowid
             WHERE message_fts MATCH ?1 AND m.account_id = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![phrase, account_id, limit as i64], row_to_summary)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -- classification ---------------------------------------------------

    pub fn record_verdict(&self, message_id: MessageId, verdict: &Verdict) -> Result<()> {
        self.conn.execute(
            "INSERT INTO classification
                 (message_id, category, confidence, source, model, latency_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                message_id,
                verdict.category,
                verdict.confidence,
                verdict.source.as_str(),
                verdict.model,
                verdict.latency_ms,
                now(),
            ],
        )?;
        Ok(())
    }

    pub fn record_correction(
        &self,
        message_id: MessageId,
        from_category: Option<&str>,
        to_category: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO correction (message_id, from_category, to_category, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![message_id, from_category, to_category, now()],
        )?;
        Ok(())
    }

    /// The categories the user has assigned by correcting messages, expressed
    /// as (sender, list_id, category) so the classifier can key on either.
    ///
    /// Only the most recent correction per message counts — a later correction
    /// supersedes an earlier one. Where several messages from one sender were
    /// corrected differently, the most recent wins, which matches the
    /// intuition that the latest decision is the current one.
    pub fn learned_categories(&self, account_id: AccountId) -> Result<Vec<LearnedCategory>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.from_addr, m.list_id, c.to_category
             FROM correction c
             JOIN message m ON m.id = c.message_id
             WHERE m.account_id = ?1
               AND c.id = (
                   SELECT MAX(c2.id) FROM correction c2 WHERE c2.message_id = c.message_id
               )
             ORDER BY c.id",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(LearnedCategory {
                sender: row.get(0)?,
                list_id: row.get(1)?,
                category: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    /// Messages where the rules baseline and the model disagreed.
    ///
    /// This is the query that answers "is the model worth its latency"; it
    /// exists from the start so the question is always answerable.
    pub fn disagreements(&self, account_id: AccountId) -> Result<Vec<Disagreement>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.subject, r.category, k.category
             FROM message m
             JOIN classification r ON r.message_id = m.id AND r.source = 'rules'
             JOIN classification k ON k.message_id = m.id AND k.source = 'model'
             WHERE m.account_id = ?1 AND r.category <> k.category
             ORDER BY m.id",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(Disagreement {
                message_id: row.get(0)?,
                subject: row.get(1)?,
                rules_category: row.get(2)?,
                model_category: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone)]
pub struct MessageSummary {
    pub id: MessageId,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub date_utc: Option<i64>,
    pub list_id: Option<String>,
    pub has_attachments: bool,
}

/// A category the user assigned by correcting a message. Either key may be set.
#[derive(Debug, Clone)]
pub struct LearnedCategory {
    pub sender: Option<String>,
    pub list_id: Option<String>,
    pub category: String,
}

#[derive(Debug, Clone)]
pub struct CategorizedMessage {
    pub summary: MessageSummary,
    pub category: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Disagreement {
    pub message_id: MessageId,
    pub subject: Option<String>,
    pub rules_category: String,
    pub model_category: String,
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let security: String = row.get(5)?;
    Ok(Account {
        id: row.get(0)?,
        label: row.get(1)?,
        email: row.get(2)?,
        imap_host: row.get(3)?,
        imap_port: row.get::<_, i64>(4)? as u16,
        imap_security: ImapSecurity::parse(&security).unwrap_or(ImapSecurity::Tls),
        username: row.get(6)?,
        auth_method: row.get(7)?,
        oauth_client_id: row.get(8)?,
        oauth_tenant: row.get(9)?,
    })
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get(0)?,
        subject: row.get(1)?,
        from_name: row.get(2)?,
        from_addr: row.get(3)?,
        date_utc: row.get(4)?,
        list_id: row.get(5)?,
        has_attachments: row.get(6)?,
    })
}

/// Fails loudly at open time rather than at the first search: a build without
/// FTS5 would otherwise look healthy until someone tried to use it.
fn ensure_fts5(conn: &Connection) -> Result<()> {
    match conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(x);
         DROP TABLE temp.fts5_probe;",
    ) {
        Ok(()) => Ok(()),
        Err(_) => Err(StoreError::MissingFts5),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
