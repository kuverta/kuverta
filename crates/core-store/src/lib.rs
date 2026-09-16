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
         it was written by a newer version of kuverta"
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
                  oauth_client_id, oauth_tenant, smtp_host, smtp_port, smtp_security,
                  oauth_provider, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                account.smtp.as_ref().map(|s| s.host.as_str()),
                account.smtp.as_ref().map(|s| s.port),
                account.smtp.as_ref().map(|s| s.security.as_str()),
                account.oauth_provider,
                now(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Changes an existing account's settings.
    ///
    /// Everything except the submission endpoint, which has its own call
    /// because it is the one part that can be absent entirely. The email
    /// address is part of the identity the keychain and the message rows hang
    /// off, so it is settable but a caller changing it has to move the
    /// credential too.
    pub fn update_account(&self, id: AccountId, account: &NewAccount) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE account
             SET label = ?2, email = ?3, imap_host = ?4, imap_port = ?5, imap_security = ?6,
                 username = ?7, auth_method = ?8, oauth_client_id = ?9, oauth_tenant = ?10,
                 oauth_provider = ?11
             WHERE id = ?1",
            params![
                id,
                account.label,
                account.email,
                account.imap_host,
                account.imap_port,
                account.imap_security.as_str(),
                account.username,
                account.auth_method,
                account.oauth_client_id,
                account.oauth_tenant,
                account.oauth_provider,
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::UnknownAccount(id));
        }
        Ok(())
    }

    /// Removes an account and everything hanging off it.
    ///
    /// Folders, messages, locations, classifications, corrections and queued
    /// operations all cascade. The blobs on disk do not — they are addressed
    /// by content and shared, so removing them is a sweep of its own rather
    /// than something to do while deleting a row.
    pub fn delete_account(&self, id: AccountId) -> Result<()> {
        let changed = self
            .conn
            .execute("DELETE FROM account WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StoreError::UnknownAccount(id));
        }
        Ok(())
    }

    /// Attaches (or replaces) the submission endpoint for an existing account.
    ///
    /// Separate from `add_account` because accounts registered before send
    /// existed need a way to gain one without being re-created.
    pub fn set_smtp(&self, account_id: AccountId, smtp: Option<&SmtpConfig>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE account SET smtp_host = ?2, smtp_port = ?3, smtp_security = ?4
             WHERE id = ?1",
            params![
                account_id,
                smtp.map(|s| s.host.as_str()),
                smtp.map(|s| s.port),
                smtp.map(|s| s.security.as_str()),
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::UnknownAccount(account_id));
        }
        Ok(())
    }

    pub fn account_by_email(&self, email: &str) -> Result<Option<Account>> {
        self.conn
            .query_row(
                "SELECT id, label, email, imap_host, imap_port, imap_security, username,
                        auth_method, oauth_client_id, oauth_tenant,
                        smtp_host, smtp_port, smtp_security, oauth_provider
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
                    auth_method, oauth_client_id, oauth_tenant,
                    smtp_host, smtp_port, smtp_security, oauth_provider
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

    // -- folder exclusions -------------------------------------------------

    /// Stops a sync touching a folder. Returns false if it was already excluded.
    pub fn exclude_folder(&self, account_id: AccountId, pattern: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO folder_exclusion (account_id, pattern, created_at)
             VALUES (?1, ?2, ?3)",
            params![account_id, pattern, now()],
        )?;
        Ok(changed > 0)
    }

    /// Undoes [`Self::exclude_folder`]. Returns false if it was not excluded.
    ///
    /// The folder's messages are not fetched here; the next sync does that,
    /// as it would for a folder that had just appeared.
    pub fn include_folder(&self, account_id: AccountId, pattern: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM folder_exclusion WHERE account_id = ?1 AND pattern = ?2",
            params![account_id, pattern],
        )?;
        Ok(changed > 0)
    }

    pub fn folder_exclusions(&self, account_id: AccountId) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT pattern FROM folder_exclusion WHERE account_id = ?1 ORDER BY pattern",
        )?;
        let rows = stmt.query_map(params![account_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -- the mutation queue ------------------------------------------------

    /// Queues a mutation, returning its id.
    ///
    /// A still-pending operation on the same message and the same flag is
    /// cancelled first: while the undo window is open the user's last word
    /// wins, and marking a message read then unread should leave one
    /// operation rather than two that fight.
    pub fn enqueue_operation(&self, op: &NewOperation) -> Result<OperationId> {
        if let OperationKind::Flag { flag, .. } = &op.kind {
            self.conn.execute(
                "UPDATE operation SET state = 'cancelled', settled_at = ?3,
                        last_error = 'superseded by a later change'
                 WHERE message_id = ?1 AND state = 'pending' AND kind = 'flag' AND flag = ?2",
                params![op.message_id, flag, now()],
            )?;
        }

        let (target_folder, flag, flag_set) = match &op.kind {
            OperationKind::Move { target_folder } => (Some(target_folder.as_str()), None, None),
            OperationKind::Flag { flag, set } => (None, Some(flag.as_str()), Some(*set as i64)),
        };

        self.conn.execute(
            "INSERT INTO operation
                 (account_id, message_id, kind, source_folder_id, source_uid,
                  source_uid_validity, expect_message_id, target_folder, flag, flag_set,
                  state, execute_after, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, ?12)",
            params![
                op.account_id,
                op.message_id,
                op.kind.as_str(),
                op.source_folder_id,
                op.source_uid,
                op.source_uid_validity,
                op.expect_message_id,
                target_folder,
                flag,
                flag_set,
                op.execute_after,
                now(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Every operation still waiting, oldest first, whether or not its undo
    /// window has elapsed. This is what a "what is queued" view shows.
    pub fn pending_operations(&self, account_id: AccountId) -> Result<Vec<Operation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {OPERATION_COLUMNS} FROM operation
             WHERE account_id = ?1 AND state = 'pending' ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![account_id], row_to_operation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Operations whose undo window has elapsed, oldest first.
    ///
    /// Order matters and is not cosmetic: two operations on the same message
    /// must reach the server in the order the user made them, or a move
    /// followed by a flag change would apply the flag in the folder the
    /// message has just left.
    pub fn due_operations(&self, account_id: AccountId, now_utc: i64) -> Result<Vec<Operation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {OPERATION_COLUMNS} FROM operation
             WHERE account_id = ?1 AND state = 'pending' AND execute_after <= ?2
             ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![account_id, now_utc], row_to_operation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Cancels a pending operation. Returns false if it had already settled —
    /// undo cannot reach something the server has been told.
    pub fn cancel_operation(&self, id: OperationId) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE operation SET state = 'cancelled', settled_at = ?2,
                    last_error = 'cancelled'
             WHERE id = ?1 AND state = 'pending'",
            params![id, now()],
        )?;
        Ok(changed > 0)
    }

    /// Cancels the most recently queued pending operation, and returns it.
    ///
    /// This is `undo`: the last thing you did that has not yet left the
    /// machine.
    pub fn cancel_latest_operation(&self, account_id: AccountId) -> Result<Option<Operation>> {
        let latest = self
            .conn
            .query_row(
                &format!(
                    "SELECT {OPERATION_COLUMNS} FROM operation
                     WHERE account_id = ?1 AND state = 'pending' ORDER BY id DESC LIMIT 1"
                ),
                params![account_id],
                row_to_operation,
            )
            .optional()?;

        match latest {
            Some(op) if self.cancel_operation(op.id)? => Ok(Some(op)),
            _ => Ok(None),
        }
    }

    /// Records the outcome of an attempt.
    ///
    /// `Pending` is a legal outcome: it means the attempt failed in a way that
    /// is worth retrying, and only the attempt count and error move.
    pub fn settle_operation(
        &self,
        id: OperationId,
        state: OperationState,
        error: Option<&str>,
    ) -> Result<()> {
        let settled_at = (state != OperationState::Pending).then(now);
        self.conn.execute(
            "UPDATE operation
             SET state = ?2, last_error = ?3, settled_at = ?4, attempts = attempts + 1
             WHERE id = ?1",
            params![id, state.as_str(), error, settled_at],
        )?;
        Ok(())
    }

    /// Looks a message up by its RFC 5322 Message-ID.
    ///
    /// Angle brackets are optional: callers get the id from wherever the user
    /// copied it, which may or may not include them.
    pub fn message_by_rfc822_id(
        &self,
        account_id: AccountId,
        message_id: &str,
    ) -> Result<Option<StoredMessage>> {
        let normalized = crate::dedup::normalize_message_id(message_id);
        self.conn
            .query_row(
                &format!("{MESSAGE_COLUMNS} WHERE account_id = ?1 AND rfc822_message_id = ?2"),
                params![account_id, normalized],
                row_to_stored_message,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Looks a message up by its row id, scoped to the account.
    ///
    /// Scoped rather than global so a stale id from another account cannot
    /// silently address someone else's mail.
    pub fn message_by_id(
        &self,
        account_id: AccountId,
        id: MessageId,
    ) -> Result<Option<StoredMessage>> {
        self.conn
            .query_row(
                &format!("{MESSAGE_COLUMNS} WHERE account_id = ?1 AND id = ?2"),
                params![account_id, id],
                row_to_stored_message,
            )
            .optional()
            .map_err(Into::into)
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
        tx.commit()?;
        Ok(removed)
    }

    /// Deletes messages that are no longer in any folder.
    ///
    /// Separate from [`Self::remove_locations`], and this is load-bearing
    /// rather than tidy. A message being moved is out of its old folder before
    /// it is seen in the new one, so a sync that collected orphans as it went
    /// would destroy the row in between — and with it the classifier verdict,
    /// the corrections the user made, and any queued operation, all of which
    /// cascade. Whether that happened came down to the order the server
    /// happened to list the two folders in.
    ///
    /// Call once, after every folder in a sync has been fetched, so a message
    /// that merely moved has already been seen in its new home.
    pub fn delete_orphaned_messages(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM message
             WHERE id NOT IN (SELECT message_id FROM message_location)",
            [],
        )?)
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
            "SELECT id, subject, from_name, from_addr, date_utc, list_id, has_attachments,
                    snippet
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
    /// One window of the message list, newest first, with a total to size the
    /// scrollbar against.
    ///
    /// Windowed because the list has to be: the Tauri spike measured the
    /// Rust/JS bridge at roughly 78 MiB/s of JSON, so handing the whole
    /// mailbox across it is the one thing that would undo the 59.8 fps it
    /// otherwise reaches. See docs/spike-tauri-list.md.
    ///
    /// `unread` is derived rather than stored: a message is unread when no
    /// copy of it anywhere carries `\Seen`. Deriving it keeps one answer for a
    /// message that sits in several folders, which on Gmail is most of them.
    pub fn message_window(
        &self,
        account_id: AccountId,
        offset: usize,
        limit: usize,
        filter: &ListFilter,
    ) -> Result<MessageWindow> {
        // Built as a list rather than a fixed template because the filters are
        // independent and combine: folder, category and unread are three
        // questions, not one enum. Positional `?` placeholders are numbered in
        // the order they are pushed, so the clause and its parameter are
        // written together and cannot drift apart.
        let mut clauses = vec!["m.account_id = ?".to_string()];
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id)];

        if let Some(category) = &filter.category {
            clauses.push("c.category = ?".into());
            args.push(Box::new(category.clone()));
        }
        if let Some(folder_id) = filter.folder {
            // A message is "in" a folder if any of its copies is. On Gmail a
            // message is routinely in several at once.
            clauses.push(
                "EXISTS (SELECT 1 FROM message_location fl
                         WHERE fl.message_id = m.id AND fl.folder_id = ?)"
                    .into(),
            );
            args.push(Box::new(folder_id));
        }
        if filter.unread_only {
            clauses.push(UNREAD_PREDICATE.into());
        }

        let from = format!(
            "FROM message m
             {CURRENT_CATEGORY_JOIN}
             WHERE {}",
            clauses.join(" AND ")
        );

        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) {from}"),
            rusqlite::params_from_iter(args.iter()),
            |row| row.get(0),
        )?;

        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, m.snippet, c.category, c.confidence,
                    {UNREAD_PREDICATE}
             {from}
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT ? OFFSET ?"
        ))?;

        args.push(Box::new(limit as i64));
        args.push(Box::new(offset as i64));

        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(ListedMessage {
                summary: row_to_summary(row)?,
                category: row.get(8)?,
                confidence: row.get(9)?,
                unread: row.get(10)?,
            })
        })?;

        Ok(MessageWindow {
            total: total as usize,
            offset,
            messages: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        })
    }

    /// Every folder on the account, with what is in it.
    ///
    /// Counts are of messages, not locations: one message carrying three Gmail
    /// labels is one message in each of those folders, and a sidebar that said
    /// otherwise would be adding up to more than the mailbox holds.
    pub fn folder_summaries(&self, account_id: AccountId) -> Result<Vec<FolderSummary>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT f.id, f.name, f.special_use,
                    COUNT(DISTINCT l.message_id),
                    COUNT(DISTINCT CASE WHEN {UNREAD_PREDICATE} THEN m.id END)
             FROM folder f
             LEFT JOIN message_location l ON l.folder_id = f.id
             LEFT JOIN message m ON m.id = l.message_id
             WHERE f.account_id = ?1
             GROUP BY f.id, f.name, f.special_use
             ORDER BY f.name"
        ))?;

        let rows = stmt.query_map(params![account_id], |row| {
            Ok(FolderSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                special_use: row.get(2)?,
                total: row.get::<_, i64>(3)? as usize,
                unread: row.get::<_, i64>(4)? as usize,
            })
        })?;

        let mut folders = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        folders.sort_by_key(|folder| (folder.rank(), folder.name.to_lowercase()));
        Ok(folders)
    }

    /// How many messages fall in each category, commonest first.
    ///
    /// Counts each message once, under the one category it is shown as. The
    /// join is what makes that true: a message reclassified on a later sync
    /// has several rows, and joining them all counted it under every category
    /// it had ever been given — which `DISTINCT` hid within a category but not
    /// across them.
    pub fn category_counts(&self, account_id: AccountId) -> Result<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT c.category, COUNT(m.id)
             FROM message m
             {CURRENT_CATEGORY_JOIN}
             WHERE m.account_id = ?1 AND c.category IS NOT NULL
             GROUP BY c.category
             ORDER BY 2 DESC, 1"
        ))?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Messages on an account that this model has not classified yet, newest
    /// first.
    ///
    /// Keyed on the model's name as well as the source, so trying a second
    /// model is a fresh pass rather than a no-op — and so comparing two models
    /// is possible at all.
    pub fn awaiting_model_verdict(
        &self,
        account_id: AccountId,
        model: &str,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, m.snippet
             FROM message m
             WHERE m.account_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM classification c
                   WHERE c.message_id = m.id AND c.source = 'model' AND c.model = ?2
               )
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, model, limit as i64], row_to_summary)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn recent_with_category(
        &self,
        account_id: AccountId,
        limit: usize,
    ) -> Result<Vec<CategorizedMessage>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, m.snippet, c.category, c.confidence
             FROM message m
             {CURRENT_CATEGORY_JOIN}
             WHERE m.account_id = ?1
             ORDER BY COALESCE(m.date_utc, 0) DESC, m.id DESC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![account_id, limit as i64], |row| {
            Ok(CategorizedMessage {
                summary: row_to_summary(row)?,
                category: row.get(8)?,
                confidence: row.get(9)?,
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
            "SELECT m.id, m.subject, m.from_name, m.from_addr, m.date_utc, m.list_id,
                    m.has_attachments, m.snippet
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

    // -- postal addresses --------------------------------------------------

    // -- where models run ----------------------------------------------------

    /// Every model provider, the local Ollama first.
    pub fn ai_providers(&self) -> Result<Vec<StoredAiProvider>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, label, base_url, key_name FROM ai_provider ORDER BY id")?;
        let rows = stmt.query_map([], row_to_ai_provider)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn ai_provider(&self, id: i64) -> Result<Option<StoredAiProvider>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, kind, label, base_url, key_name FROM ai_provider WHERE id = ?1",
                params![id],
                row_to_ai_provider,
            )
            .optional()?)
    }

    pub fn add_ai_provider(&self, provider: &NewAiProvider) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO ai_provider (kind, label, base_url, key_name, created_at)
             VALUES (?1, ?2, ?3, lower(hex(randomblob(16))), ?4)",
            params![
                provider.kind,
                provider.label,
                provider.base_url.trim().trim_end_matches('/'),
                now()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Changes a provider in place, keeping its key where it is filed.
    ///
    /// Returns whether there was such a provider.
    pub fn update_ai_provider(&self, id: i64, provider: &NewAiProvider) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE ai_provider SET kind = ?2, label = ?3, base_url = ?4 WHERE id = ?1",
            params![
                id,
                provider.kind,
                provider.label,
                provider.base_url.trim().trim_end_matches('/')
            ],
        )?;
        Ok(changed > 0)
    }

    /// Removes a provider and every job given one of its models, so those jobs
    /// go back to their defaults rather than pointing at nothing.
    pub fn delete_ai_provider(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM ai_task WHERE provider_id = ?1", params![id])?;
        tx.execute("DELETE FROM ai_provider WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// The jobs a model has been chosen for. A job not listed uses its default.
    pub fn ai_tasks(&self) -> Result<Vec<StoredAiTask>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task, provider_id, model FROM ai_task ORDER BY task")?;
        let rows = stmt.query_map([], |row| {
            Ok(StoredAiTask {
                task: row.get(0)?,
                provider_id: row.get(1)?,
                model: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn set_ai_task(&self, task: &str, provider_id: i64, model: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_task (task, provider_id, model) VALUES (?1, ?2, ?3)
             ON CONFLICT (task) DO UPDATE
             SET provider_id = excluded.provider_id, model = excluded.model",
            params![task, provider_id, model],
        )?;
        Ok(())
    }

    /// Sends a job back to its default.
    pub fn clear_ai_task(&self, task: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM ai_task WHERE task = ?1", params![task])?;
        Ok(())
    }

    // -- postal addresses, continued -----------------------------------------

    /// Every configured physical address, oldest first.
    pub fn paper_mailboxes(&self) -> Result<Vec<StoredPaperMailbox>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, base_url, selector_kind, selector_value, token_key
             FROM paper_mailbox ORDER BY id",
        )?;
        let rows = stmt.query_map([], row_to_paper_mailbox)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn paper_mailbox(&self, id: i64) -> Result<Option<StoredPaperMailbox>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, label, base_url, selector_kind, selector_value, token_key
                 FROM paper_mailbox WHERE id = ?1",
                params![id],
                row_to_paper_mailbox,
            )
            .optional()?)
    }

    /// Adds an address, or updates the one already pointing at those documents.
    pub fn upsert_paper_mailbox(&self, mailbox: &NewPaperMailbox) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO paper_mailbox
                 (label, base_url, selector_kind, selector_value, created_at, token_key)
             VALUES (?1, ?2, ?3, ?4, ?5, lower(hex(randomblob(16))))
             ON CONFLICT (base_url, selector_kind, IFNULL(selector_value, ''))
             DO UPDATE SET label = excluded.label",
            params![
                mailbox.label,
                mailbox.base_url.trim_end_matches('/'),
                mailbox.selector_kind,
                mailbox.selector_value,
                now(),
            ],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM paper_mailbox
             WHERE base_url = ?1 AND selector_kind = ?2
               AND IFNULL(selector_value, '') = IFNULL(?3, '')",
            params![
                mailbox.base_url.trim_end_matches('/'),
                mailbox.selector_kind,
                mailbox.selector_value
            ],
            |row| row.get(0),
        )?)
    }

    /// Changes an existing address in place.
    ///
    /// Separate from the upsert because the upsert is keyed on where the
    /// documents are, and an edit is exactly a change to where they are: saving
    /// a new URL through the upsert would add a second address and strand the
    /// first, together with the token filed under its id.
    ///
    /// Returns whether there was such an address.
    pub fn update_paper_mailbox(&self, id: i64, mailbox: &NewPaperMailbox) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE paper_mailbox
             SET label = ?2, base_url = ?3, selector_kind = ?4, selector_value = ?5
             WHERE id = ?1",
            params![
                id,
                mailbox.label,
                mailbox.base_url.trim_end_matches('/'),
                mailbox.selector_kind,
                mailbox.selector_value,
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn delete_paper_mailbox(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM paper_mailbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    // -- what Paperless does not keep ---------------------------------------

    /// Marks a letter read or unread here.
    ///
    /// Unread is the absence of a row: a letter nobody has opened in kuverta
    /// is unread, however long it has been in Paperless — which is what a
    /// freshly scanned letter should be.
    pub fn set_paper_read(&self, base_url: &str, document_id: i64, read: bool) -> Result<()> {
        let base = base_url.trim_end_matches('/');
        if read {
            self.conn.execute(
                "INSERT INTO paper_read (base_url, document_id, read_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (base_url, document_id) DO NOTHING",
                params![base, document_id, now()],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM paper_read WHERE base_url = ?1 AND document_id = ?2",
                params![base, document_id],
            )?;
        }
        Ok(())
    }

    /// The letters on an instance that have been read here.
    pub fn paper_read_ids(&self, base_url: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT document_id FROM paper_read WHERE base_url = ?1 ORDER BY document_id",
        )?;
        let rows = stmt.query_map(params![base_url.trim_end_matches('/')], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Keeps a transcript of a scan, replacing any earlier one.
    pub fn save_paper_transcript(
        &self,
        base_url: &str,
        document_id: i64,
        model: &str,
        text: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO paper_transcript (base_url, document_id, model, text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (base_url, document_id)
             DO UPDATE SET model = excluded.model, text = excluded.text,
                           created_at = excluded.created_at",
            params![
                base_url.trim_end_matches('/'),
                document_id,
                model,
                text,
                now()
            ],
        )?;
        Ok(())
    }

    /// Every transcript kept for an instance, as `(document, model, text)`.
    pub fn paper_transcripts(&self, base_url: &str) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT document_id, model, text FROM paper_transcript
             WHERE base_url = ?1 ORDER BY document_id",
        )?;
        let rows = stmt.query_map(params![base_url.trim_end_matches('/')], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -- corrections to post ------------------------------------------------

    /// Records that the user filed a piece of post by hand.
    pub fn record_paper_correction(
        &self,
        mailbox_id: i64,
        document_id: i64,
        correspondent: Option<&str>,
        from_category: Option<&str>,
        to_category: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO paper_correction
                 (mailbox_id, document_id, correspondent, from_category, to_category, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mailbox_id,
                document_id,
                correspondent,
                from_category,
                to_category,
                now()
            ],
        )?;
        Ok(())
    }

    /// The category each corrected document of one address was last filed under.
    ///
    /// Only the most recent correction per document counts, so filing a letter
    /// twice leaves the second answer standing and both events in the table.
    pub fn paper_overrides(&self, mailbox_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.document_id, p.to_category
             FROM paper_correction p
             WHERE p.mailbox_id = ?1
               AND p.id = (
                   SELECT MAX(p2.id) FROM paper_correction p2
                   WHERE p2.mailbox_id = p.mailbox_id AND p2.document_id = p.document_id
               )
             ORDER BY p.document_id",
        )?;
        let rows = stmt.query_map(params![mailbox_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The category each correspondent was last filed under, across every address.
    ///
    /// Across addresses rather than per address, the same way mail learned from
    /// one account is used for post: the Stadtwerke are the Stadtwerke whichever
    /// letterbox their bill arrived through.
    pub fn paper_learned(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.correspondent, p.to_category
             FROM paper_correction p
             WHERE p.correspondent IS NOT NULL
               AND p.id = (
                   SELECT MAX(p2.id) FROM paper_correction p2
                   WHERE p2.correspondent = p.correspondent
               )
             ORDER BY p.id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
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

    /// The category a message is currently shown under, if any.
    ///
    /// The same preference `CURRENT_CATEGORY_JOIN` applies, expressed for one
    /// message. Kept beside it so the two cannot drift: a correction recorded
    /// against a different "before" than the list was showing would make the
    /// log disagree with what the user saw.
    pub fn current_category(&self, message_id: MessageId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT c.category FROM classification c
                 WHERE c.message_id = ?1 AND c.source IN ('rules', 'agent', 'user')
                 ORDER BY CASE c.source WHEN 'user' THEN 2 WHEN 'agent' THEN 1 ELSE 0 END DESC,
                   c.id DESC
                 LIMIT 1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The category the user themselves most recently filed a message under.
    ///
    /// Separate from [`Store::current_category`] because the two answer
    /// different questions once an assistant can file mail. "Is this already
    /// where it is shown?" includes the assistant's filing; "has the user
    /// already said so?" must not — otherwise a user confirming an assistant's
    /// suggestion would record nothing, and the one filing that teaches the
    /// classifier would be silently dropped.
    pub fn user_category(&self, message_id: MessageId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT category FROM classification
                 WHERE message_id = ?1 AND source = 'user'
                 ORDER BY id DESC LIMIT 1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?)
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
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
    /// First line or so of the body, recorded at parse time. A list that shows
    /// only sender and subject makes you open mail to find out what it is.
    pub snippet: Option<String>,
}

/// Somewhere models run. A hosted service's key lives in the keychain.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAiProvider {
    pub id: i64,
    /// `ollama`, or `openai` for a service that speaks OpenAI's API.
    pub kind: String,
    pub label: String,
    pub base_url: String,
    /// What the key is filed under in the keychain: random, fixed when the
    /// provider is added, and unique across stores.
    pub key_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewAiProvider {
    pub kind: String,
    pub label: String,
    pub base_url: String,
}

/// The model chosen for one job, and where it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAiTask {
    pub task: String,
    pub provider_id: i64,
    pub model: String,
}

fn row_to_ai_provider(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAiProvider> {
    Ok(StoredAiProvider {
        id: row.get(0)?,
        kind: row.get(1)?,
        label: row.get(2)?,
        base_url: row.get(3)?,
        key_name: row.get(4)?,
    })
}

/// A physical address, as stored. The API token lives in the keychain.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPaperMailbox {
    pub id: i64,
    pub label: String,
    pub base_url: String,
    pub selector_kind: String,
    pub selector_value: Option<String>,
    /// What the token is filed under in the keychain. Random, fixed when the
    /// address is added, and unique across stores — unlike `id`.
    pub token_key: String,
}

/// A physical address, as configured.
#[derive(Debug, Clone, Default)]
pub struct NewPaperMailbox {
    pub label: String,
    pub base_url: String,
    pub selector_kind: String,
    pub selector_value: Option<String>,
}

fn row_to_paper_mailbox(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPaperMailbox> {
    Ok(StoredPaperMailbox {
        id: row.get(0)?,
        label: row.get(1)?,
        base_url: row.get(2)?,
        selector_kind: row.get(3)?,
        selector_value: row.get(4)?,
        token_key: row.get(5)?,
    })
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

/// Whether a message is unread: no copy of it anywhere carries `\\Seen`.
///
/// Written once because the list, the counts and the filter all have to agree.
/// Two spellings of this would show as a badge that never matches the rows.
/// The category a message is *shown* under.
///
/// A correction the user made wins; then something an assistant filed through
/// the agent surface; then the most recent rules verdict. Model verdicts are deliberately not consulted: they are recorded
/// beside the rules ones so `disagreements` can compare the two, and a model
/// that started changing what the list shows would take that measurement with
/// it.
///
/// Written once and shared, because three queries have to agree about this and
/// a fourth — `disagreements` — has to deliberately not.
const CURRENT_CATEGORY_JOIN: &str = "\
    LEFT JOIN classification c
      ON c.id = (
          SELECT c2.id FROM classification c2
          WHERE c2.message_id = m.id AND c2.source IN ('rules', 'agent', 'user')
          ORDER BY CASE c2.source WHEN 'user' THEN 2 WHEN 'agent' THEN 1 ELSE 0 END DESC,
                   c2.id DESC
          LIMIT 1
      )";

const UNREAD_PREDICATE: &str = "NOT EXISTS (SELECT 1 FROM message_location ul
                                            WHERE ul.message_id = m.id
                                              AND ul.flags LIKE '%\\Seen%')";

const MESSAGE_COLUMNS: &str =
    "SELECT id, rfc822_message_id, subject, from_addr, date_utc, body_path FROM message";

fn row_to_stored_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        id: row.get(0)?,
        rfc822_message_id: row.get(1)?,
        subject: row.get(2)?,
        from_addr: row.get(3)?,
        date_utc: row.get(4)?,
        body_path: row.get(5)?,
    })
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let kind: String = row.get(3)?;
    let kind = match kind.as_str() {
        "move" => OperationKind::Move {
            target_folder: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        },
        _ => OperationKind::Flag {
            flag: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
            set: row.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
        },
    };

    Ok(Operation {
        id: row.get(0)?,
        account_id: row.get(1)?,
        message_id: row.get(2)?,
        kind,
        source_folder_id: row.get(4)?,
        source_uid: row.get::<_, i64>(5)? as u32,
        source_uid_validity: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
        expect_message_id: row.get(7)?,
        state: OperationState::parse(&row.get::<_, String>(11)?).unwrap_or(OperationState::Failed),
        execute_after: row.get(12)?,
        attempts: row.get(13)?,
        last_error: row.get(14)?,
        created_at: row.get(15)?,
    })
}

/// Column list for [`row_to_operation`], in the order it reads them.
const OPERATION_COLUMNS: &str = "id, account_id, message_id, kind, source_folder_id, source_uid, \
     source_uid_validity, expect_message_id, target_folder, flag, flag_set, state, \
     execute_after, attempts, last_error, created_at";

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let security: String = row.get(5)?;
    // The three SMTP columns are written as a unit, but a hand-edited database
    // could still hold a partial row; treat anything incomplete as "cannot
    // send" rather than inventing a default port.
    let smtp = match (
        row.get::<_, Option<String>>(10)?,
        row.get::<_, Option<i64>>(11)?,
        row.get::<_, Option<String>>(12)?,
    ) {
        (Some(host), Some(port), Some(security)) => Some(SmtpConfig {
            host,
            port: port as u16,
            security: SmtpSecurity::parse(&security).unwrap_or(SmtpSecurity::Tls),
        }),
        _ => None,
    };

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
        smtp,
        oauth_provider: row.get(13)?,
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
        snippet: row.get(7)?,
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
