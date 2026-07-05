//! Profiles: accounts and postal addresses kept apart by what they are for —
//! private, one company, another.
//!
//! A profile holds nothing itself. Accounts and addresses point at one, or at
//! none, and the window shows one profile at a time or all of them. Deleting a
//! profile leaves what was in it where it is, in no profile.

use rusqlite::{params, OptionalExtension};

use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

impl Store {
    /// Every profile, in the order the person put them in.
    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, position FROM profile ORDER BY position, id")?;
        let rows = stmt.query_map([], |row| {
            Ok(Profile {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The profile with this name, ignoring case.
    pub fn profile_named(&self, name: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT id FROM profile WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Adds a profile at the end.
    pub fn add_profile(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO profile (name, position, created_at)
             VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM profile), ?2)",
            params![name, crate::now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_profile(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profile SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Deletes a profile; its accounts and addresses are in none afterwards.
    pub fn delete_profile(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE account SET profile_id = NULL WHERE profile_id = ?1",
            params![id],
        )?;
        tx.execute(
            "UPDATE paper_mailbox SET profile_id = NULL WHERE profile_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM profile WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Puts profiles in the order given; ids not named keep their place after.
    pub fn reorder_profiles(&self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE profile SET position = ?2 WHERE id = ?1",
                params![id, position as i64 + 1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_account_profile(&self, account_id: i64, profile: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE account SET profile_id = ?2 WHERE id = ?1",
            params![account_id, profile],
        )?;
        Ok(())
    }

    pub fn set_paper_profile(&self, mailbox_id: i64, profile: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE paper_mailbox SET profile_id = ?2 WHERE id = ?1",
            params![mailbox_id, profile],
        )?;
        Ok(())
    }

    /// Which profile each account is in, as `(account, profile)`.
    pub fn account_profiles(&self) -> Result<Vec<(i64, Option<i64>)>> {
        let mut stmt = self.conn.prepare("SELECT id, profile_id FROM account")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Which profile each postal address is in, as `(address, profile)`.
    pub fn paper_profiles(&self) -> Result<Vec<(i64, Option<i64>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, profile_id FROM paper_mailbox")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
