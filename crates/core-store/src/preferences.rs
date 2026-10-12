//! Settings that are one value for the whole store.
//!
//! A preference is a named value: whether the assistant may research online,
//! whether it uses skills, where its searches go. Kept here, not in the
//! window, because the core enforces them — a tool the person has not
//! switched on is never offered to a model, whichever window or command line
//! asks. What is not set reads as `None`, and the caller knows the default.

use rusqlite::{params, OptionalExtension};

use crate::{Result, Store};

impl Store {
    pub fn preference(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM preference WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO preference (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, crate::now()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preference_is_absent_until_set_and_then_replaced() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.preference("web.research").unwrap(), None);
        store.set_preference("web.research", "yes").unwrap();
        assert_eq!(
            store.preference("web.research").unwrap().as_deref(),
            Some("yes")
        );
        store.set_preference("web.research", "no").unwrap();
        assert_eq!(
            store.preference("web.research").unwrap().as_deref(),
            Some("no")
        );
    }
}
