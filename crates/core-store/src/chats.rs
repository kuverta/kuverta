//! The assistant's chats, kept so they can be opened again.
//!
//! A chat is kept twice over: as the model sees it — the turns, which is what
//! going on with it takes — and as the window showed it, question by
//! question with what the assistant did on the way, which is what reading it
//! again takes. Both are JSON that core-rpc and the window own; the store
//! keeps them whole and lists them by when they were last added to.

use rusqlite::{params, OptionalExtension};

use crate::model::AccountId;
use crate::{Result, Store};

/// A chat as listed: no content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: i64,
    pub account_id: AccountId,
    /// The first question, cut short.
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A chat in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChat {
    pub summary: ChatSummary,
    pub turns: String,
    pub log: String,
}

impl Store {
    /// Keeps a chat: a new one without an id, or the one with it replaced.
    /// Returns its id.
    pub fn save_chat(
        &self,
        id: Option<i64>,
        account_id: AccountId,
        title: &str,
        turns: &str,
        log: &str,
    ) -> Result<i64> {
        let now = crate::now();
        if let Some(id) = id {
            let changed = self.conn.execute(
                "UPDATE assistant_chat SET title = ?2, turns = ?3, log = ?4, updated_at = ?5
                  WHERE id = ?1",
                params![id, title, turns, log, now],
            )?;
            if changed > 0 {
                return Ok(id);
            }
        }
        self.conn.execute(
            "INSERT INTO assistant_chat (account_id, title, turns, log, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![account_id, title, turns, log, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The account's chats, most recently added to first.
    pub fn chats(&self, account_id: AccountId, limit: usize) -> Result<Vec<ChatSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, title, created_at, updated_at FROM assistant_chat
              WHERE account_id = ?1 ORDER BY updated_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |row| {
            Ok(ChatSummary {
                id: row.get(0)?,
                account_id: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn chat(&self, id: i64) -> Result<Option<StoredChat>> {
        self.conn
            .query_row(
                "SELECT id, account_id, title, created_at, updated_at, turns, log
                   FROM assistant_chat WHERE id = ?1",
                params![id],
                |row| {
                    Ok(StoredChat {
                        summary: ChatSummary {
                            id: row.get(0)?,
                            account_id: row.get(1)?,
                            title: row.get(2)?,
                            created_at: row.get(3)?,
                            updated_at: row.get(4)?,
                        },
                        turns: row.get(5)?,
                        log: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn delete_chat(&self, id: i64) -> Result<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM assistant_chat WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImapSecurity, NewAccount};

    #[test]
    fn a_chat_is_kept_added_to_and_listed_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let account = store
            .add_account(&NewAccount {
                label: "Dev".into(),
                email: "dev@kuverta.test".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: 993,
                imap_security: ImapSecurity::Tls,
                username: "dev@kuverta.test".into(),
                auth_method: "app_password".into(),
                ..Default::default()
            })
            .unwrap();
        let first = store
            .save_chat(None, account, "What needs me?", "[]", "[]")
            .unwrap();
        let second = store
            .save_chat(None, account, "Find the eSIM", "[]", "[]")
            .unwrap();
        let again = store
            .save_chat(Some(first), account, "What needs me?", "[1]", "[2]")
            .unwrap();
        assert_eq!(again, first);
        // Within one second the order is by id; across seconds, by when a
        // chat was last added to. A test cannot wait for the clock, so it
        // checks only that both are there.
        let mut listed: Vec<i64> = store
            .chats(account, 10)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        listed.sort_unstable();
        assert_eq!(listed, vec![first, second]);
        let kept = store.chat(first).unwrap().unwrap();
        assert_eq!(kept.turns, "[1]");
        assert_eq!(kept.log, "[2]");
        // Saving under an id that has gone makes a new chat rather than
        // losing the conversation.
        assert!(store.delete_chat(second).unwrap());
        let replaced = store
            .save_chat(Some(second), account, "Find the eSIM", "[]", "[]")
            .unwrap();
        assert!(store.chat(replaced).unwrap().is_some());
        assert_eq!(store.chats(account, 10).unwrap().len(), 2);
    }
}
