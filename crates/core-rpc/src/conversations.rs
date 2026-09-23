//! Mail as conversations: a list of people, and with each the exchange as a
//! messenger shows it — theirs on one side, yours on the other.
//!
//! Mail quotes everything that came before, so an exchange of ten messages
//! shown whole is the first message ten times. Each bubble shows what that
//! message added: quoted lines, the "On … wrote:" line that introduces them,
//! a forwarded original and the signature are taken off. The whole message is
//! a click away in the ordinary view; this is for following who said what.

use core_store::model::AccountId;
use serde::{Deserialize, Serialize};

use crate::{Core, Result};

/// A person in the conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationView {
    /// Their address, which is also how the conversation is asked for.
    pub key: String,
    pub name: String,
    pub messages: usize,
    pub unread: usize,
    pub last_utc: Option<i64>,
    pub last_subject: Option<String>,
    pub last_snippet: Option<String>,
    pub last_from_me: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPage {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<ConversationView>,
}

/// One bubble.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bubble {
    pub id: i64,
    pub from_me: bool,
    pub date_utc: Option<i64>,
    pub subject: Option<String>,
    /// What this message added, quotes and signature taken off.
    pub text: String,
    pub unread: bool,
}

/// The exchange with one person.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub key: String,
    pub name: String,
    pub bubbles: Vec<Bubble>,
    /// Their latest message, which a reply from the bubble view answers.
    pub reply_to: Option<i64>,
}

/// How many messages a conversation opens with.
const THREAD_LIMIT: usize = 200;

impl Core {
    /// People this account has mail with, most recent first.
    pub fn conversations(
        &self,
        account: AccountId,
        include_bulk: bool,
        query: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<ConversationPage> {
        let me = self.account_email(account)?;
        let skip = self.folders_to_leave_out(account)?;
        let window =
            self.store()
                .conversations(account, &me, &skip, include_bulk, query, offset, limit)?;
        Ok(ConversationPage {
            total: window.total,
            offset: window.offset,
            rows: window
                .conversations
                .into_iter()
                .map(|c| ConversationView {
                    name: c
                        .name
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| c.key.clone()),
                    key: c.key,
                    messages: c.messages,
                    unread: c.unread,
                    last_utc: c.last_utc,
                    last_subject: c.last_subject,
                    last_snippet: c.last_snippet,
                    last_from_me: c.last_from_me,
                })
                .collect(),
        })
    }

    /// Everything with one person, oldest first, as bubbles.
    pub fn conversation(&self, account: AccountId, key: &str) -> Result<Thread> {
        let me = self.account_email(account)?;
        let skip = self.folders_to_leave_out(account)?;
        let messages = self
            .store()
            .conversation(account, &me, key, &skip, THREAD_LIMIT)?;
        let name = messages
            .iter()
            .rev()
            .filter(|m| !m.from_me)
            .find_map(|m| m.from_name.clone())
            .unwrap_or_else(|| key.to_string());
        let reply_to = messages.iter().rev().find(|m| !m.from_me).map(|m| m.id);
        let bubbles = messages
            .into_iter()
            .map(|m| {
                let text = m
                    .body_path
                    .as_deref()
                    .and_then(|path| self.blobs().get(path).ok())
                    .and_then(|raw| {
                        mail_parser::MessageParser::default()
                            .parse(&raw)
                            .and_then(|p| p.body_text(0).map(|t| t.into_owned()))
                    })
                    .map(|text| strip_quoted(&text))
                    .filter(|text| !text.is_empty())
                    .or(m.snippet)
                    .unwrap_or_default();
                Bubble {
                    id: m.id,
                    from_me: m.from_me,
                    date_utc: m.date_utc,
                    subject: m.subject,
                    text,
                    unread: m.unread,
                }
            })
            .collect();
        Ok(Thread {
            key: key.to_string(),
            name,
            bubbles,
            reply_to,
        })
    }

    /// Folders whose mail is not part of anyone's conversation: the Trash
    /// and Junk.
    pub(crate) fn folders_to_leave_out(&self, account: AccountId) -> Result<Vec<String>> {
        Ok(self
            .store()
            .folder_summaries(account)?
            .into_iter()
            .filter(|folder| matches!(folder.rank(), 5 | 6))
            .map(|folder| folder.name)
            .collect())
    }
}

/// What a message added: its text with the quoted original, the line that
/// introduces it, a forwarded message and the signature taken off.
///
/// Conservative where it cannot tell: a line is only taken as the start of a
/// quote when it looks like one of the usual introductions, and when stripping
/// would leave nothing the text is kept whole — an empty bubble says less
/// than a long one.
pub fn strip_quoted(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::new();

    for (at, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // A signature: "-- " on a line of its own, by the old convention.
        if *line == "-- " || trimmed == "--" {
            break;
        }
        if starts_quote(trimmed) {
            break;
        }
        // Outlook's block: "From: … / Sent: …" after a blank line or a rule.
        if (trimmed.starts_with("From:") || trimmed.starts_with("Von:"))
            && lines.get(at + 1).is_some_and(|next| {
                let next = next.trim();
                ["Sent:", "Gesendet:", "Date:", "Datum:", "To:", "An:"]
                    .iter()
                    .any(|h| next.starts_with(h))
            })
        {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        kept.push(line.trim_end());
    }

    // Blank lines at the ends go, and runs of them inside collapse to one.
    let mut out: Vec<&str> = Vec::new();
    for line in kept {
        if line.is_empty() && out.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|last| last.is_empty()) {
        out.pop();
    }
    let stripped = out.join("\n");
    if stripped.trim().is_empty() {
        text.trim().to_string()
    } else {
        stripped
    }
}

fn starts_quote(line: &str) -> bool {
    let lower = line.to_lowercase();
    if lower.contains("original message")
        || lower.contains("ursprüngliche nachricht")
        || lower.contains("forwarded message")
        || lower.contains("weitergeleitete nachricht")
    {
        return lower.starts_with('-') || lower.starts_with("begin");
    }
    // "On Tue, 3 Sep 2026, Erika <…> wrote:" / "Am 03.09.2026 schrieb Erika:"
    (lower.starts_with("on ") && lower.ends_with("wrote:"))
        || (lower.starts_with("am ") && lower.ends_with(':') && lower.contains("schrieb"))
        || (lower.starts_with("le ") && lower.ends_with("a écrit :"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_keeps_what_it_added() {
        let text = "Passt mir gut.\n\nViele Grüße\nErika\n\nAm 03.09.2026 um 10:12 schrieb Max <max@example.de>:\n> Dienstag um 10?\n> Max\n";
        assert_eq!(strip_quoted(text), "Passt mir gut.\n\nViele Grüße\nErika");
    }

    #[test]
    fn signatures_outlook_blocks_and_inline_quotes_go() {
        assert_eq!(
            strip_quoted("Yes.\n-- \nErika Mustermann\nExample GmbH"),
            "Yes."
        );
        assert_eq!(
            strip_quoted("Done.\n\nFrom: Max\nSent: Monday\nTo: Erika\nSubject: x\n\nold"),
            "Done."
        );
        assert_eq!(strip_quoted("> earlier\nmy answer\n> more"), "my answer");
        assert_eq!(
            strip_quoted("See below\n\n-------- Forwarded Message --------\nSubject: x"),
            "See below"
        );
    }

    #[test]
    fn a_message_that_is_all_quote_is_kept_whole() {
        assert_eq!(strip_quoted("> only a quote"), "> only a quote");
    }
}
