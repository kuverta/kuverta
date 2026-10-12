//! Skills: instructions for one kind of job, fetched from somewhere, waiting
//! to be read.
//!
//! A skill is a text file — Markdown, usually — that tells the assistant how
//! to do one kind of job: how a cancellation letter for a German contract is
//! worded, what to check on an invoice from a particular supplier. It comes
//! from an address the person gave, and it is kept here with where it came
//! from and a digest of its text.
//!
//! `accepted_at` is the whole point of the table. It is NULL when the skill
//! arrives and stays NULL until the person has read the text in the settings
//! and accepted it there; until then no model sees a word of it. A model can
//! fetch a skill; only a person can accept one.

use rusqlite::{params, OptionalExtension};

use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSkill {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    /// The address it was fetched from.
    pub source: String,
    pub text: String,
    /// SHA-256 of the text, hex: the same file again is told from a changed
    /// one.
    pub digest: String,
    pub fetched_at: i64,
    /// When the person accepted it in the settings; `None` while it waits.
    pub accepted_at: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct NewSkill {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub text: String,
    pub digest: String,
}

const COLUMNS: &str =
    "id, name, description, source, text, digest, fetched_at, accepted_at, enabled";

fn row_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSkill> {
    Ok(StoredSkill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        source: row.get(3)?,
        text: row.get(4)?,
        digest: row.get(5)?,
        fetched_at: row.get(6)?,
        accepted_at: row.get(7)?,
        enabled: row.get(8)?,
    })
}

impl Store {
    /// Every skill, the ones waiting to be read first, then by name.
    pub fn skills(&self) -> Result<Vec<StoredSkill>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM skill ORDER BY accepted_at IS NOT NULL, name COLLATE NOCASE, id"
        ))?;
        let rows = stmt.query_map([], row_skill)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn skill(&self, id: i64) -> Result<Option<StoredSkill>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM skill WHERE id = ?1"),
                params![id],
                row_skill,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Keeps a fetched skill, waiting to be read. A skill from the same
    /// address that was never accepted is rewritten in place, keeping its
    /// id: it was only ever the previous download of this one. An accepted
    /// one is left as it is and the new text sits beside it, so that an
    /// update is read and accepted like the first version was.
    pub fn add_skill(&self, skill: &NewSkill) -> Result<i64> {
        let waiting: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM skill WHERE source = ?1 AND accepted_at IS NULL ORDER BY id LIMIT 1",
                params![skill.source],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = waiting {
            self.conn.execute(
                "UPDATE skill SET name = ?2, description = ?3, text = ?4, digest = ?5, fetched_at = ?6
                  WHERE id = ?1",
                params![
                    id,
                    skill.name,
                    skill.description,
                    skill.text,
                    skill.digest,
                    crate::now()
                ],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO skill (name, description, source, text, digest, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                skill.name,
                skill.description,
                skill.source,
                skill.text,
                skill.digest,
                crate::now()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The person has read it and accepts it. Returns whether there was a
    /// skill to accept.
    pub fn accept_skill(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE skill SET accepted_at = ?2 WHERE id = ?1 AND accepted_at IS NULL",
            params![id, crate::now()],
        )?;
        Ok(changed > 0)
    }

    pub fn set_skill_enabled(&self, id: i64, enabled: bool) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE skill SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(changed > 0)
    }

    pub fn delete_skill(&self, id: i64) -> Result<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM skill WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(source: &str, text: &str) -> NewSkill {
        NewSkill {
            name: "Cancellations".into(),
            description: None,
            source: source.into(),
            text: text.into(),
            digest: format!("digest-of-{}", text.len()),
        }
    }

    #[test]
    fn a_skill_waits_until_accepted_and_a_redownload_replaces_only_a_waiting_one() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .add_skill(&skill("https://example.com/SKILL.md", "one"))
            .unwrap();
        assert!(store.skill(first).unwrap().unwrap().accepted_at.is_none());

        // Fetched again before anyone read it: the earlier download is
        // rewritten, under the same id.
        let second = store
            .add_skill(&skill("https://example.com/SKILL.md", "two"))
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(store.skill(first).unwrap().unwrap().text, "two");
        assert_eq!(store.skills().unwrap().len(), 1);
        assert!(store.accept_skill(second).unwrap());
        assert!(!store.accept_skill(second).unwrap(), "accepted once");

        // Fetched again after acceptance: the accepted one stays, the new
        // text waits beside it.
        let third = store
            .add_skill(&skill("https://example.com/SKILL.md", "three"))
            .unwrap();
        let all = store.skills().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, third, "the one waiting to be read comes first");
        assert!(all[1].accepted_at.is_some());

        assert!(store.set_skill_enabled(second, false).unwrap());
        assert!(!store.skill(second).unwrap().unwrap().enabled);
        assert!(store.delete_skill(second).unwrap());
        assert!(!store.delete_skill(second).unwrap());
    }
}
