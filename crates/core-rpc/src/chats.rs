//! The assistant's chats, kept to be opened again.
//!
//! The window keeps a chat as two things: the turns, which are the model's
//! view and what going on with the conversation takes, and the log, which is
//! what the window showed — each question, its answer and the cards between
//! — and which only the window reads. The core keeps both whole, names the
//! chat after its first question, and lists chats by when they were last
//! added to.

use core_ai::Turn;
use core_store::model::AccountId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Core, Result, RpcError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatView {
    pub id: i64,
    pub account_id: AccountId,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A chat in full, for opening again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatFull {
    #[serde(flatten)]
    pub view: ChatView,
    pub turns: Vec<Turn>,
    pub log: Value,
}

/// How many chats are listed.
const MOST_LISTED: usize = 100;
const TITLE_CHARS: usize = 80;

/// A chat's name: its first question, without the mail that came attached
/// to it, cut short.
pub fn title_of(turns: &[Turn]) -> String {
    let first = turns.iter().find_map(|turn| match turn {
        Turn::User { content } => Some(content.as_str()),
        _ => None,
    });
    let Some(first) = first else {
        return "(empty)".into();
    };
    let asked = match first.rfind("</message>\n") {
        Some(at) => &first[at + "</message>\n".len()..],
        None => first,
    };
    let line = asked
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut title: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        title.push('…');
    }
    if title.is_empty() {
        "(empty)".into()
    } else {
        title
    }
}

impl Core {
    pub fn chats(&self, account: AccountId) -> Result<Vec<ChatView>> {
        Ok(self
            .store()
            .chats(account, MOST_LISTED)?
            .into_iter()
            .map(|c| ChatView {
                id: c.id,
                account_id: c.account_id,
                title: c.title,
                created_at: c.created_at,
                updated_at: c.updated_at,
            })
            .collect())
    }

    pub fn chat(&self, id: i64) -> Result<ChatFull> {
        let stored = self
            .store()
            .chat(id)?
            .ok_or_else(|| RpcError::Rejected(format!("there is no chat {id}")))?;
        Ok(ChatFull {
            view: ChatView {
                id: stored.summary.id,
                account_id: stored.summary.account_id,
                title: stored.summary.title,
                created_at: stored.summary.created_at,
                updated_at: stored.summary.updated_at,
            },
            turns: serde_json::from_str(&stored.turns).unwrap_or_default(),
            log: serde_json::from_str(&stored.log).unwrap_or(Value::Array(Vec::new())),
        })
    }

    /// Keeps a chat, new or continued, and returns it as listed.
    pub fn save_chat(
        &self,
        id: Option<i64>,
        account: AccountId,
        turns: &[Turn],
        log: &Value,
    ) -> Result<ChatView> {
        let title = title_of(turns);
        let turns = serde_json::to_string(turns)
            .map_err(|err| RpcError::Rejected(format!("the chat could not be kept: {err}")))?;
        let id = self
            .store()
            .save_chat(id, account, &title, &turns, &log.to_string())?;
        let stored = self.store().chat(id)?.map(|c| c.summary);
        let summary = stored.ok_or_else(|| RpcError::Rejected("the chat was not kept".into()))?;
        Ok(ChatView {
            id: summary.id,
            account_id: summary.account_id,
            title: summary.title,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        })
    }

    pub fn delete_chat(&self, id: i64) -> Result<()> {
        self.store().delete_chat(id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_is_named_after_its_first_question_without_the_mail() {
        let turns = vec![
            Turn::System { content: "x".into() },
            Turn::User {
                content: "This message is open in front of the person:\n<message id=1>\nFrom: a\n</message>\nReply and confirm.".into(),
            },
        ];
        assert_eq!(title_of(&turns), "Reply and confirm.");
        let long = Turn::User {
            content: "x".repeat(100),
        };
        assert_eq!(title_of(&[long]).chars().count(), 81);
        assert_eq!(title_of(&[]), "(empty)");
    }
}
