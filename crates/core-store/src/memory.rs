//! What the assistant remembers about an account.
//!
//! Short notes, kept so that a fact found once — who the tax adviser is,
//! where invoices go — is told to the model with each question instead of
//! searched for again. Each has a random key that is the same on every
//! device the notes are synced to, and a time it last changed: two devices
//! that both changed a note keep the later change. A forgotten note stays as
//! an empty row, so that the forgetting travels too.

use rusqlite::{params, OptionalExtension};

use crate::model::AccountId;
use crate::{Result, Store};

/// One note, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMemory {
    pub id: i64,
    pub account_id: AccountId,
    pub key: String,
    /// Empty when forgotten.
    pub text: String,
    pub forgotten: bool,
    /// Unix seconds.
    pub updated_at: i64,
}

/// Whether, and how it went, the notes travel through the mail server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySync {
    pub enabled: bool,
    /// Changed here since the last upload.
    pub pending: bool,
    pub synced_at: Option<i64>,
    pub last_error: Option<String>,
}

fn row_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMemory> {
    Ok(StoredMemory {
        id: row.get(0)?,
        account_id: row.get(1)?,
        key: row.get(2)?,
        text: row.get(3)?,
        forgotten: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

const COLUMNS: &str = "id, account_id, key, text, forgotten, updated_at";

impl Store {
    /// The account's notes, oldest first; forgotten ones too when asked for,
    /// which only syncing needs.
    pub fn memories(
        &self,
        account_id: AccountId,
        with_forgotten: bool,
    ) -> Result<Vec<StoredMemory>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM assistant_memory
              WHERE account_id = ?1 AND (?2 OR forgotten = 0)
              ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![account_id, with_forgotten], row_memory)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn memory(&self, id: i64) -> Result<Option<StoredMemory>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM assistant_memory WHERE id = ?1"),
                params![id],
                row_memory,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Keeps a new note, returning its id.
    pub fn add_memory(&self, account_id: AccountId, key: &str, text: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO assistant_memory (account_id, key, text, forgotten, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4)",
            params![account_id, key, text, crate::now()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.memory_changed(account_id)?;
        Ok(id)
    }

    /// Rewrites a note. False when there is no such note.
    pub fn update_memory(&self, id: i64, text: &str) -> Result<bool> {
        let Some(memory) = self.memory(id)? else {
            return Ok(false);
        };
        self.conn.execute(
            "UPDATE assistant_memory SET text = ?2, forgotten = 0, updated_at = ?3 WHERE id = ?1",
            params![id, text, crate::now().max(memory.updated_at + 1)],
        )?;
        self.memory_changed(memory.account_id)?;
        Ok(true)
    }

    /// Forgets a note: its text goes, its row stays for syncing.
    pub fn forget_memory(&self, id: i64) -> Result<bool> {
        let Some(memory) = self.memory(id)? else {
            return Ok(false);
        };
        if memory.forgotten {
            return Ok(true);
        }
        self.conn.execute(
            "UPDATE assistant_memory SET text = '', forgotten = 1, updated_at = ?2 WHERE id = ?1",
            params![id, crate::now().max(memory.updated_at + 1)],
        )?;
        self.memory_changed(memory.account_id)?;
        Ok(true)
    }

    /// Takes a note as another device has it, if it is newer than the one
    /// here — or the same age and greater, so every device settles on the
    /// same one. Returns whether anything changed. Does not mark the notes
    /// as changed here: this is the change coming in, not going out.
    pub fn merge_memory(
        &self,
        account_id: AccountId,
        key: &str,
        text: &str,
        forgotten: bool,
        updated_at: i64,
    ) -> Result<bool> {
        let text = if forgotten { "" } else { text };
        let mine: Option<(String, bool, i64)> = self
            .conn
            .query_row(
                "SELECT text, forgotten, updated_at FROM assistant_memory
                  WHERE account_id = ?1 AND key = ?2",
                params![account_id, key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        match mine {
            None => {
                self.conn.execute(
                    "INSERT INTO assistant_memory (account_id, key, text, forgotten, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![account_id, key, text, forgotten, updated_at],
                )?;
                Ok(true)
            }
            Some(mine) if wins((text, forgotten, updated_at), (&mine.0, mine.1, mine.2)) => {
                self.conn.execute(
                    "UPDATE assistant_memory SET text = ?3, forgotten = ?4, updated_at = ?5
                      WHERE account_id = ?1 AND key = ?2",
                    params![account_id, key, text, forgotten, updated_at],
                )?;
                Ok(true)
            }
            Some(_) => Ok(false),
        }
    }

    pub fn memory_sync(&self, account_id: AccountId) -> Result<MemorySync> {
        Ok(self
            .conn
            .query_row(
                "SELECT enabled, pending, synced_at, last_error FROM memory_sync WHERE account_id = ?1",
                params![account_id],
                |row| {
                    Ok(MemorySync {
                        enabled: row.get(0)?,
                        pending: row.get(1)?,
                        synced_at: row.get(2)?,
                        last_error: row.get(3)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Turns syncing on or off. Turned on, everything here counts as not yet
    /// uploaded.
    pub fn set_memory_sync(&self, account_id: AccountId, enabled: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memory_sync (account_id, enabled, pending) VALUES (?1, ?2, ?2)
             ON CONFLICT (account_id) DO UPDATE SET enabled = excluded.enabled,
                 pending = excluded.pending OR pending, last_error = NULL",
            params![account_id, enabled],
        )?;
        Ok(())
    }

    /// How a sync went: `None` for an error means it worked, and what was
    /// pending is uploaded.
    pub fn memory_synced(&self, account_id: AccountId, error: Option<&str>) -> Result<()> {
        match error {
            None => self.conn.execute(
                "UPDATE memory_sync SET pending = 0, synced_at = ?2, last_error = NULL
                  WHERE account_id = ?1",
                params![account_id, crate::now()],
            )?,
            Some(error) => self.conn.execute(
                "UPDATE memory_sync SET last_error = ?2 WHERE account_id = ?1",
                params![account_id, error],
            )?,
        };
        Ok(())
    }

    fn memory_changed(&self, account_id: AccountId) -> Result<()> {
        self.conn.execute(
            "UPDATE memory_sync SET pending = 1 WHERE account_id = ?1",
            params![account_id],
        )?;
        Ok(())
    }
}

/// Whether a note `theirs` replaces `mine`: later wins, and at the same
/// second a forgetting wins, then the greater text — any rule will do, as long
/// as every device applies the same one.
fn wins(theirs: (&str, bool, i64), mine: (&str, bool, i64)) -> bool {
    (theirs.2, theirs.1, theirs.0) > (mine.2, mine.1, mine.0)
}

#[cfg(test)]
mod tests {
    use super::wins;

    #[test]
    fn the_later_note_wins_and_ties_settle_the_same_way_everywhere() {
        assert!(wins(("b", false, 2), ("a", false, 1)));
        assert!(!wins(("b", false, 1), ("a", false, 2)));
        assert!(wins(("", true, 5), ("a", false, 5)));
        assert!(wins(("b", false, 5), ("a", false, 5)));
        assert!(!wins(("a", false, 5), ("b", false, 5)));
        assert!(!wins(("a", false, 5), ("a", false, 5)));
    }
}
