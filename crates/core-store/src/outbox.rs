//! Mail waiting to be sent later.
//!
//! "Send tomorrow at 8" is a promise made now and kept later, so the whole
//! message is written down now: the draft as the compose window described it,
//! and when it is due. The store does not know what a draft is — core-rpc
//! stores it as JSON and reads it back — so this is only the ledger: what is
//! due, what went, what failed and why.
//!
//! A message moves through `scheduled` → `sending` → `sent`, or to `failed`
//! with the reason, or to `cancelled`. `sending` is claimed before the server
//! is touched, for the reason the operation queue records intent first: a
//! crash in the middle must leave a row that says a send may have happened,
//! never one that quietly sends it twice.

use rusqlite::{params, OptionalExtension};

use crate::model::AccountId;
use crate::{Result, Store};

/// A message in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub id: i64,
    pub account_id: AccountId,
    /// The draft, as JSON core-rpc wrote.
    pub draft: String,
    /// For listing without reading the draft back.
    pub subject: String,
    pub recipients: String,
    /// Unix seconds.
    pub send_at: i64,
    /// `scheduled`, `sending`, `sent`, `failed` or `cancelled`.
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub sent_at: Option<i64>,
}

const COLUMNS: &str = "id, account_id, draft, subject, recipients, send_at, state, attempts, \
                       last_error, created_at, sent_at";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxEntry> {
    Ok(OutboxEntry {
        id: row.get(0)?,
        account_id: row.get(1)?,
        draft: row.get(2)?,
        subject: row.get(3)?,
        recipients: row.get(4)?,
        send_at: row.get(5)?,
        state: row.get(6)?,
        attempts: row.get(7)?,
        last_error: row.get(8)?,
        created_at: row.get(9)?,
        sent_at: row.get(10)?,
    })
}

impl Store {
    pub fn schedule_send(
        &self,
        account_id: AccountId,
        draft: &str,
        subject: &str,
        recipients: &str,
        send_at: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO outbox (account_id, draft, subject, recipients, send_at, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'scheduled', ?6)",
            params![account_id, draft, subject, recipients, send_at, crate::now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn outbox_entry(&self, id: i64) -> Result<Option<OutboxEntry>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM outbox WHERE id = ?1"),
                params![id],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }

    /// What is waiting, and what failed and still needs a decision, soonest
    /// first. Sent and cancelled mail has left the outbox.
    pub fn outbox(&self) -> Result<Vec<OutboxEntry>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox
             WHERE state IN ('scheduled', 'sending', 'failed')
             ORDER BY send_at, id"
        ))?;
        let rows = stmt.query_map([], row_to_entry)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Scheduled messages whose time has come.
    pub fn due_outbox(&self, now: i64) -> Result<Vec<OutboxEntry>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox
             WHERE state = 'scheduled' AND send_at <= ?1
             ORDER BY send_at, id"
        ))?;
        let rows = stmt.query_map(params![now], row_to_entry)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Takes a due message for sending. False when something else took it
    /// first, it was cancelled, or it was moved to later in the meantime —
    /// then it is not ours to send, or not yet.
    pub fn claim_outbox(&self, id: i64, now: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE outbox SET state = 'sending', attempts = attempts + 1
             WHERE id = ?1 AND state = 'scheduled' AND send_at <= ?2",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn outbox_sent(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET state = 'sent', sent_at = ?2, last_error = NULL WHERE id = ?1",
            params![id, crate::now()],
        )?;
        Ok(())
    }

    pub fn outbox_failed(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET state = 'failed', last_error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Cancels a message that has not gone. False when it already has, or is
    /// going now: a send in flight cannot be called back.
    pub fn cancel_outbox(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE outbox SET state = 'cancelled'
             WHERE id = ?1 AND state IN ('scheduled', 'failed')",
            params![id],
        )?;
        Ok(changed == 1)
    }

    /// Moves a waiting or failed message to a new time, which also retries a
    /// failed one.
    pub fn reschedule_outbox(&self, id: i64, send_at: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE outbox SET state = 'scheduled', send_at = ?2, last_error = NULL
             WHERE id = ?1 AND state IN ('scheduled', 'failed')",
            params![id, send_at],
        )?;
        Ok(changed == 1)
    }

    /// Messages left `sending` by a window that closed mid-send. Whether they
    /// went is unknowable from here, so they are marked failed with that said,
    /// for a person to decide — never re-sent on a guess.
    pub fn recover_interrupted_sends(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE outbox SET state = 'failed',
                    last_error = 'kuverta closed while this was being sent, so it may or may \
not have gone. Check Sent before sending it again.'
             WHERE state = 'sending'",
            [],
        )?)
    }
}
