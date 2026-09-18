//! Smart mailboxes: saved questions asked of the store.
//!
//! A smart mailbox holds no mail. It is a set of rules — *from contains
//! "invoice"*, *older than 30 days* — that the list window answers the same way
//! it answers a folder or a category, so everything the list can do to a
//! folder it can do to one of these: page through it, count it, act on it.
//!
//! The rules are stored as JSON because the set of fields will grow and a
//! column per field would be a migration per field. They are compiled to SQL
//! here, with every value bound as a parameter rather than spliced in: a rule
//! is typed by a person, or imported from another program's file, and neither
//! is a source of trusted SQL.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::model::{AccountId, FolderId};
use crate::{Result, Store};

/// What a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartField {
    /// Sender name and address together, so "anna" and "@example.de" both match.
    From,
    /// To and Cc addresses.
    To,
    Subject,
    /// The full text, through the search index.
    Body,
    /// The category the list shows, e.g. `newsletter`.
    Category,
    /// `List-Id`.
    ListId,
    /// Any folder the message has a copy in, by name.
    Folder,
    /// No copy of it carries `\Seen`. The value is `true` or `false`.
    Unread,
    /// The value is `true` or `false`.
    HasAttachment,
    /// Carries a `List-Unsubscribe` header. The value is `true` or `false`.
    CanUnsubscribe,
    /// Sent more than this many days ago.
    OlderThanDays,
    /// Sent within this many days.
    NewerThanDays,
}

/// How a rule compares. Text comparisons ignore ASCII case, as a person
/// typing "Rechnung" means "rechnung" too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartOp {
    Contains,
    NotContains,
    Is,
    IsNot,
    BeginsWith,
    EndsWith,
}

/// One condition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SmartRule {
    pub field: SmartField,
    pub op: SmartOp,
    pub value: String,
}

/// A whole smart mailbox's question: its rules, and whether all of them must
/// hold or any one is enough.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct SmartQuery {
    pub match_all: bool,
    pub rules: Vec<SmartRule>,
}

/// A smart mailbox as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSmartMailbox {
    pub id: i64,
    pub account_id: AccountId,
    pub name: String,
    pub query: SmartQuery,
    /// Where it came from, when it was imported: `thunderbird` or `apple_mail`.
    pub source: Option<String>,
}

impl SmartRule {
    /// Checks a rule before it is saved, so a mailbox that can never match
    /// anything — a number field holding "soon" — is refused with a reason
    /// rather than stored and silently empty.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let value = self.value.trim();
        match self.field {
            SmartField::Unread | SmartField::HasAttachment | SmartField::CanUnsubscribe => {
                if parse_bool(value).is_none() {
                    return Err(format!("{:?} takes yes or no, not {value:?}", self.field));
                }
            }
            SmartField::OlderThanDays | SmartField::NewerThanDays => {
                if value.parse::<u32>().is_err() {
                    return Err(format!("a number of days, not {value:?}"));
                }
            }
            SmartField::Body => {
                if !matches!(self.op, SmartOp::Contains | SmartOp::NotContains) {
                    return Err("the text of a message can only be searched for words".into());
                }
                if value.is_empty() {
                    return Err("say which words to look for".into());
                }
            }
            _ => {
                if value.is_empty() && !matches!(self.op, SmartOp::Is | SmartOp::IsNot) {
                    return Err("a rule needs something to compare with".into());
                }
            }
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// A rule's SQL and the parameters it binds, in order.
pub(crate) struct Compiled {
    pub sql: String,
    pub args: Vec<Box<dyn rusqlite::ToSql>>,
}

/// Compiles a query to one parenthesised predicate over `m` (the message) and
/// `c` (its current category), as `message_window` joins them.
///
/// `now` is passed in so the age rules are testable and so one list, paged
/// over several calls, does not shift under itself as the clock moves.
pub(crate) fn compile(query: &SmartQuery, now: i64) -> Compiled {
    let mut parts = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    for rule in &query.rules {
        // A rule that cannot be read matches nothing. Skipping it instead
        // would widen the mailbox — an "all of these" box with one rule gone
        // shows more than it should, and one with every rule gone shows the
        // whole account, where select-all and Delete are two keys away.
        if rule.validate().is_err() {
            parts.push("0".to_string());
            continue;
        }
        parts.push(compile_rule(rule, now, &mut args));
    }

    let sql = if parts.is_empty() {
        "0".to_string()
    } else {
        let joiner = if query.match_all { " AND " } else { " OR " };
        format!("({})", parts.join(joiner))
    };
    Compiled { sql, args }
}

fn compile_rule(rule: &SmartRule, now: i64, args: &mut Vec<Box<dyn rusqlite::ToSql>>) -> String {
    let value = rule.value.trim().to_string();
    let text = |column: &str, args: &mut Vec<Box<dyn rusqlite::ToSql>>| -> String {
        let subject = format!("kuverta_lower(COALESCE({column}, ''))");
        text_predicate(&subject, rule.op, value.clone(), args)
    };

    match rule.field {
        // `is` and `ends with` mean the address, which is what a person
        // comparing a sender has in mind — "is anna@example.de", "ends with
        // @example.de". The other comparisons see the name as well, so
        // "contains Anna" finds her whichever way she writes her address.
        SmartField::From if matches!(rule.op, SmartOp::Is | SmartOp::IsNot | SmartOp::EndsWith) => {
            text("m.from_addr", args)
        }
        SmartField::From => text(
            "COALESCE(m.from_name, '') || ' <' || COALESCE(m.from_addr, '') || '>'",
            args,
        ),
        SmartField::To => text("m.recipients", args),
        SmartField::Subject => text("m.subject", args),
        SmartField::ListId => text("m.list_id", args),
        SmartField::Category => {
            let subject = "COALESCE(c.category, 'unknown')";
            text_predicate(subject, rule.op, value.to_ascii_lowercase(), args)
        }
        SmartField::Folder => {
            let inner = text_predicate("kuverta_lower(f.name)", positive(rule.op), value, args);
            let exists = format!(
                "EXISTS (SELECT 1 FROM message_location fl JOIN folder f ON f.id = fl.folder_id
                         WHERE fl.message_id = m.id AND {inner})"
            );
            if negated(rule.op) {
                format!("NOT {exists}")
            } else {
                exists
            }
        }
        SmartField::Body => {
            // Quoted, as `search` quotes, so what was typed is a phrase and
            // never FTS5 syntax.
            let phrase = format!("body : \"{}\"", value.replace('"', "\"\""));
            args.push(Box::new(phrase));
            let within = "m.id IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ?)";
            if negated(rule.op) {
                format!("NOT {within}")
            } else {
                within.to_string()
            }
        }
        SmartField::Unread => {
            let wanted = parse_bool(&value).unwrap_or(true) != negated(rule.op);
            if wanted {
                crate::UNREAD_PREDICATE.to_string()
            } else {
                format!("NOT {}", crate::UNREAD_PREDICATE)
            }
        }
        SmartField::HasAttachment => {
            let wanted = parse_bool(&value).unwrap_or(true) != negated(rule.op);
            format!("m.has_attachments = {}", i64::from(wanted))
        }
        SmartField::CanUnsubscribe => {
            let wanted = parse_bool(&value).unwrap_or(true) != negated(rule.op);
            if wanted {
                "m.list_unsubscribe IS NOT NULL".to_string()
            } else {
                "m.list_unsubscribe IS NULL".to_string()
            }
        }
        SmartField::OlderThanDays | SmartField::NewerThanDays => {
            let days: i64 = value.parse().unwrap_or(0);
            args.push(Box::new(now - days * 86_400));
            if rule.field == SmartField::OlderThanDays {
                "COALESCE(m.date_utc, 0) < ?".to_string()
            } else {
                "COALESCE(m.date_utc, 0) >= ?".to_string()
            }
        }
    }
}

/// `subject` compared with `value` by `op`. The value is lowercased here as
/// Rust lowercases; `subject` must be lowered by `kuverta_lower` to match.
/// Values are bound, never spliced.
fn text_predicate(
    subject: &str,
    op: SmartOp,
    value: String,
    args: &mut Vec<Box<dyn rusqlite::ToSql>>,
) -> String {
    let value = value.to_lowercase();
    match op {
        SmartOp::Contains => {
            args.push(Box::new(value));
            format!("instr({subject}, ?) > 0")
        }
        SmartOp::NotContains => {
            args.push(Box::new(value));
            format!("instr({subject}, ?) = 0")
        }
        SmartOp::Is => {
            args.push(Box::new(value));
            format!("{subject} = ?")
        }
        SmartOp::IsNot => {
            args.push(Box::new(value));
            format!("{subject} <> ?")
        }
        SmartOp::BeginsWith => {
            args.push(Box::new(value));
            format!("instr({subject}, ?) = 1")
        }
        SmartOp::EndsWith => {
            let length = value.chars().count() as i64;
            args.push(Box::new(length));
            args.push(Box::new(value));
            format!("substr({subject}, -?) = ?")
        }
    }
}

fn negated(op: SmartOp) -> bool {
    matches!(op, SmartOp::NotContains | SmartOp::IsNot)
}

/// The positive form of a negated operator, for rules that negate an EXISTS
/// rather than the comparison inside it: "in no folder named Archive" is not
/// "in some folder not named Archive".
fn positive(op: SmartOp) -> SmartOp {
    match op {
        SmartOp::NotContains => SmartOp::Contains,
        SmartOp::IsNot => SmartOp::Is,
        other => other,
    }
}

impl Store {
    pub fn smart_mailboxes(&self, account_id: AccountId) -> Result<Vec<StoredSmartMailbox>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, name, match_all, rules, source
             FROM smart_mailbox WHERE account_id = ?1 ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map(params![account_id], row_to_smart)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn smart_mailbox(&self, id: i64) -> Result<Option<StoredSmartMailbox>> {
        self.conn
            .query_row(
                "SELECT id, account_id, name, match_all, rules, source
                 FROM smart_mailbox WHERE id = ?1",
                params![id],
                row_to_smart,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Adds a smart mailbox, or replaces the one with `id`.
    pub fn save_smart_mailbox(
        &self,
        id: Option<i64>,
        account_id: AccountId,
        name: &str,
        query: &SmartQuery,
        source: Option<&str>,
    ) -> Result<i64> {
        let rules = serde_json::to_string(&query.rules).unwrap_or_else(|_| "[]".into());
        match id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE smart_mailbox SET name = ?2, match_all = ?3, rules = ?4
                     WHERE id = ?1",
                    params![id, name, query.match_all, rules],
                )?;
                Ok(id)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO smart_mailbox (account_id, name, match_all, rules, source, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![account_id, name, query.match_all, rules, source, crate::now()],
                )?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_smart_mailbox(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM smart_mailbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Whether an account already has a smart mailbox by this name — how an
    /// import avoids adding the same mailbox twice when it is run again.
    pub fn smart_mailbox_named(&self, account_id: AccountId, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM smart_mailbox WHERE account_id = ?1 AND name = ?2",
                params![account_id, name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// How many messages a filter selects, and how many of those are unread,
    /// without fetching any of them — for a sidebar badge.
    pub fn count_matching(
        &self,
        account_id: AccountId,
        filter: &crate::model::ListFilter,
    ) -> Result<(usize, usize)> {
        let total = self.message_window(account_id, 0, 0, filter)?.total;
        let unread_filter = crate::model::ListFilter {
            unread_only: true,
            ..filter.clone()
        };
        let unread = self.message_window(account_id, 0, 0, &unread_filter)?.total;
        Ok((total, unread))
    }

    /// Folder ids by name, for rules that name a folder a server may spell
    /// differently from the program the rule was imported from.
    pub fn folder_named(&self, account_id: AccountId, name: &str) -> Result<Option<FolderId>> {
        self.conn
            .query_row(
                "SELECT id FROM folder WHERE account_id = ?1 AND name = ?2 COLLATE NOCASE",
                params![account_id, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}

fn row_to_smart(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSmartMailbox> {
    let rules: String = row.get(4)?;
    Ok(StoredSmartMailbox {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        query: SmartQuery {
            match_all: row.get(3)?,
            // A row this build cannot read is shown as an empty mailbox rather
            // than failing the sidebar: no rules compile to nothing.
            rules: serde_json::from_str(&rules).unwrap_or_default(),
        },
        source: row.get(5)?,
    })
}
