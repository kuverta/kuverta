//! Smart mailboxes: saved searches that live in the sidebar.
//!
//! The rules are `core_store::smart`'s; this adds what a window needs around
//! them — counts for the sidebar, validation with reasons, and bringing over
//! the ones a person already built in another program.
//!
//! ## Importing
//!
//! Thunderbird keeps its saved searches ("virtual folders") in each profile's
//! `virtualFolders.dat`, a plain-text file of `uri=`/`scope=`/`terms=` lines;
//! Apple Mail keeps its smart mailboxes in `SmartMailboxes.plist`. Neither
//! program's rules are a subset of ours or ours of theirs, so an import
//! translates what it can and **says what it could not**: a smart mailbox that
//! silently lost its "is flagged" rule would show more mail than the original
//! and look right while doing it.

use std::path::{Path, PathBuf};

use core_store::model::{AccountId, ListFilter};
use core_store::{SmartField, SmartOp, SmartQuery, SmartRule};
use serde::{Deserialize, Serialize};

use crate::{Core, Result, RpcError};

/// A smart mailbox as the sidebar draws it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartMailboxView {
    pub id: i64,
    pub account_id: AccountId,
    pub name: String,
    pub query: SmartQuery,
    pub source: Option<String>,
    pub total: usize,
    pub unread: usize,
}

/// A smart mailbox as the editor sends it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartMailboxInput {
    pub id: Option<i64>,
    pub account_id: AccountId,
    pub name: String,
    pub query: SmartQuery,
}

/// How many messages a query selects, and the first few, for the editor to
/// show while the rules are being written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartPreview {
    pub total: usize,
    pub rows: Vec<crate::MessageRow>,
}

/// A smart mailbox found in another program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundSmartMailbox {
    pub name: String,
    /// `thunderbird` or `apple_mail`.
    pub source: String,
    /// The kuverta account it belongs to, when that could be told from the
    /// file; `None` means it will go to whichever account the person chooses.
    pub account_id: Option<AccountId>,
    pub query: SmartQuery,
    /// Rules that could not be carried over, each in words.
    pub skipped: Vec<String>,
    /// Already imported once: a mailbox by this name exists on the account.
    pub exists: bool,
}

/// What looking for smart mailboxes elsewhere found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SmartImportScan {
    pub found: Vec<FoundSmartMailbox>,
    /// Where it looked and what went wrong, for a program that is installed
    /// but could not be read.
    pub notes: Vec<String>,
}

impl Core {
    pub fn smart_mailboxes(&self, account: AccountId) -> Result<Vec<SmartMailboxView>> {
        self.store()
            .smart_mailboxes(account)?
            .into_iter()
            .map(|mailbox| {
                let (total, unread) = self.store().count_matching(
                    account,
                    &ListFilter {
                        smart: Some(mailbox.query.clone()),
                        ..Default::default()
                    },
                )?;
                Ok(SmartMailboxView {
                    id: mailbox.id,
                    account_id: mailbox.account_id,
                    name: mailbox.name,
                    query: mailbox.query,
                    source: mailbox.source,
                    total,
                    unread,
                })
            })
            .collect()
    }

    /// The rules of one smart mailbox, for the list to filter by.
    pub fn smart_query(&self, id: i64) -> Result<SmartQuery> {
        Ok(self
            .store()
            .smart_mailbox(id)?
            .ok_or_else(|| RpcError::Rejected(format!("no smart mailbox {id}")))?
            .query)
    }

    pub fn save_smart_mailbox(&self, input: &SmartMailboxInput) -> Result<i64> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(RpcError::Rejected("a smart mailbox needs a name".into()));
        }
        if input.query.rules.is_empty() {
            return Err(RpcError::Rejected(
                "add at least one rule — a smart mailbox with none would be all mail".into(),
            ));
        }
        for (at, rule) in input.query.rules.iter().enumerate() {
            rule.validate()
                .map_err(|why| RpcError::Rejected(format!("rule {}: {why}", at + 1)))?;
        }
        Ok(self
            .store()
            .save_smart_mailbox(input.id, input.account_id, name, &input.query, None)?)
    }

    pub fn delete_smart_mailbox(&self, id: i64) -> Result<()> {
        Ok(self.store().delete_smart_mailbox(id)?)
    }

    /// What a query would select, before it is saved.
    pub fn preview_smart(
        &self,
        account: AccountId,
        query: &SmartQuery,
        limit: usize,
    ) -> Result<SmartPreview> {
        let page = self.messages(
            account,
            0,
            limit,
            &ListFilter {
                smart: Some(query.clone()),
                ..Default::default()
            },
        )?;
        Ok(SmartPreview {
            total: page.total,
            rows: page.rows,
        })
    }

    /// Looks for smart mailboxes in Thunderbird and Apple Mail.
    pub fn scan_smart_imports(&self) -> Result<SmartImportScan> {
        let accounts = self.store().accounts()?;
        let mut scan = SmartImportScan::default();

        for root in crate::setup::thunderbird::roots() {
            let Ok(profiles) = crate::setup::thunderbird::profiles(&root) else {
                continue;
            };
            for profile in profiles {
                let path = profile.join("virtualFolders.dat");
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        for mut found in thunderbird_virtual_folders(&text) {
                            found.account_id = found
                                .account_hint
                                .as_ref()
                                .and_then(|(user, host)| match_account(&accounts, user, host));
                            scan.found.push(found.into_found());
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => scan
                        .notes
                        .push(format!("could not read {}: {err}", path.display())),
                }
            }
        }

        if let Some(path) = apple_mail_smart_mailboxes() {
            match std::fs::read(&path) {
                Ok(bytes) => match plist::Value::from_reader(std::io::Cursor::new(bytes)) {
                    Ok(value) => scan.found.extend(apple_mail_mailboxes(&value)),
                    Err(err) => scan.notes.push(format!(
                        "could not read Apple Mail's smart mailboxes: {err}"
                    )),
                },
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => scan.notes.push(
                    "Apple Mail's smart mailboxes need Full Disk Access for kuverta \
                     (System Settings → Privacy & Security)"
                        .into(),
                ),
                Err(err) => scan
                    .notes
                    .push(format!("could not read {}: {err}", path.display())),
            }
        }

        for found in &mut scan.found {
            if let Some(account) = found.account_id {
                found.exists = self.store().smart_mailbox_named(account, &found.name)?;
            }
        }
        Ok(scan)
    }

    /// Saves imported smart mailboxes; those with no account of their own go
    /// to `fallback`. Returns how many were added. One that already exists by
    /// name is left alone, so importing twice adds nothing twice.
    pub fn import_smart_mailboxes(
        &self,
        chosen: &[FoundSmartMailbox],
        fallback: AccountId,
    ) -> Result<usize> {
        let mut added = 0;
        for found in chosen {
            let account = found.account_id.unwrap_or(fallback);
            if found.query.rules.is_empty()
                || found
                    .query
                    .rules
                    .iter()
                    .any(|rule| rule.validate().is_err())
                || self.store().smart_mailbox_named(account, &found.name)?
            {
                continue;
            }
            self.store().save_smart_mailbox(
                None,
                account,
                &found.name,
                &found.query,
                Some(&found.source),
            )?;
            added += 1;
        }
        Ok(added)
    }
}

fn match_account(
    accounts: &[core_store::model::Account],
    user: &str,
    host: &str,
) -> Option<AccountId> {
    accounts
        .iter()
        .find(|a| a.username.eq_ignore_ascii_case(user) && a.imap_host.eq_ignore_ascii_case(host))
        .or_else(|| {
            accounts.iter().find(|a| {
                a.username.eq_ignore_ascii_case(user) || a.email.eq_ignore_ascii_case(user)
            })
        })
        .map(|a| a.id)
}

/// A Thunderbird virtual folder, before it is matched to an account.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TbVirtualFolder {
    name: String,
    /// The server user and host its URI names.
    account_hint: Option<(String, String)>,
    account_id: Option<AccountId>,
    query: SmartQuery,
    skipped: Vec<String>,
}

impl TbVirtualFolder {
    fn into_found(self) -> FoundSmartMailbox {
        FoundSmartMailbox {
            name: self.name,
            source: "thunderbird".into(),
            account_id: self.account_id,
            query: self.query,
            skipped: self.skipped,
            exists: false,
        }
    }
}

/// Reads `virtualFolders.dat`.
pub(crate) fn thunderbird_virtual_folders(text: &str) -> Vec<TbVirtualFolder> {
    let mut folders = Vec::new();
    let mut uri: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut terms: Option<String> = None;

    let mut flush =
        |uri: &mut Option<String>, scope: &mut Option<String>, terms: &mut Option<String>| {
            if let (Some(uri), Some(terms)) = (uri.take(), terms.take()) {
                folders.push(virtual_folder(&uri, scope.take().as_deref(), &terms));
            }
            *scope = None;
        };

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "uri" => {
                flush(&mut uri, &mut scope, &mut terms);
                uri = Some(value.trim().to_string());
            }
            "scope" => scope = Some(value.trim().to_string()),
            "terms" => terms = Some(value.trim().to_string()),
            _ => {}
        }
    }
    flush(&mut uri, &mut scope, &mut terms);
    folders
}

fn virtual_folder(uri: &str, scope: Option<&str>, terms: &str) -> TbVirtualFolder {
    let (account_hint, path) = split_folder_uri(uri);
    let name = path
        .rsplit('/')
        .next()
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or("Imported")
        .to_string();

    let mut skipped = Vec::new();
    let (match_all, raw_terms) = parse_terms(terms);
    let mut rules: Vec<SmartRule> = Vec::new();
    for (attribute, op, value) in raw_terms {
        match tb_rule(&attribute, &op, &value) {
            Some(rule) => rules.push(rule),
            None => skipped.push(format!("{attribute} {op} {value}").trim().to_string()),
        }
    }

    // Where it searched. One folder is a rule; several are only expressible
    // when any rule may match, which would change what the others mean.
    if let Some(scope) = scope {
        let folders: Vec<String> = scope
            .split('|')
            .map(|uri| split_folder_uri(uri).1)
            .filter(|path| !path.is_empty())
            .collect();
        match folders.as_slice() {
            [one] if match_all => rules.push(SmartRule {
                field: SmartField::Folder,
                op: SmartOp::Is,
                value: one.clone(),
            }),
            [] => {}
            many => skipped.push(format!("searched only in {}", many.join(", "))),
        }
    }

    let rules = keep_valid(rules, &mut skipped);
    TbVirtualFolder {
        name,
        account_hint,
        account_id: None,
        query: SmartQuery { match_all, rules },
        skipped,
    }
}

/// The rules that would be accepted if typed here; the rest are said, like
/// the ones with no equivalent. An empty "subject contains" from another
/// program is no rule at all.
fn keep_valid(rules: Vec<SmartRule>, skipped: &mut Vec<String>) -> Vec<SmartRule> {
    rules
        .into_iter()
        .filter(|rule| match rule.validate() {
            Ok(()) => true,
            Err(why) => {
                skipped.push(format!(
                    "{:?} {:?} {:?}: {why}",
                    rule.field, rule.op, rule.value
                ));
                false
            }
        })
        .collect()
}

/// `imap://erika%40example.de@imap.example.de/INBOX/Bills` →
/// (`("erika@example.de", "imap.example.de")`, `"INBOX/Bills"`).
fn split_folder_uri(uri: &str) -> (Option<(String, String)>, String) {
    let Some((_, rest)) = uri.split_once("://") else {
        return (None, String::new());
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = decode(path);
    let hint = authority.rsplit_once('@').and_then(|(user, host)| {
        let user = decode(user);
        // Local Folders is `nobody@Local Folders`, which is no account.
        (user != "nobody").then(|| (user, decode(host)))
    });
    (hint, path)
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `AND (subject,contains,Rechnung) AND (from,is,"a, b")` → whether all must
/// match, and each `(attribute, op, value)`. `ALL` is no terms at all.
fn parse_terms(terms: &str) -> (bool, Vec<(String, String, String)>) {
    let mut parsed = Vec::new();
    let mut match_all = true;
    let mut first = true;
    let mut rest = terms.trim();

    while !rest.is_empty() {
        let (joiner, after) = if let Some(after) = rest.strip_prefix("AND") {
            ("AND", after)
        } else if let Some(after) = rest.strip_prefix("OR") {
            ("OR", after)
        } else {
            break;
        };
        if first {
            match_all = joiner == "AND";
            first = false;
        }
        let Some(after) = after.trim_start().strip_prefix('(') else {
            break;
        };
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut chars = after.char_indices().peekable();
        let mut quoted = false;
        let mut end = after.len();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' if field.is_empty() && !quoted => quoted = true,
                '\\' if quoted => {
                    if let Some((_, next)) = chars.next() {
                        field.push(next);
                    }
                }
                '"' if quoted => quoted = false,
                ',' if !quoted && fields.len() < 2 => fields.push(std::mem::take(&mut field)),
                ')' if !quoted => {
                    end = at + 1;
                    break;
                }
                _ => field.push(c),
            }
        }
        fields.push(field);
        if fields.len() == 3 {
            parsed.push((fields[0].clone(), fields[1].clone(), fields[2].clone()));
        }
        rest = after[end.min(after.len())..].trim_start();
    }
    (match_all, parsed)
}

/// One Thunderbird search term as a rule, when it has an equivalent.
fn tb_rule(attribute: &str, op: &str, value: &str) -> Option<SmartRule> {
    let text_op = |op: &str| -> Option<SmartOp> {
        Some(match op {
            "contains" => SmartOp::Contains,
            "doesn't contain" => SmartOp::NotContains,
            "is" => SmartOp::Is,
            "isn't" => SmartOp::IsNot,
            "begins with" => SmartOp::BeginsWith,
            "ends with" => SmartOp::EndsWith,
            _ => return None,
        })
    };
    let rule = |field, op| {
        Some(SmartRule {
            field,
            op,
            value: value.to_string(),
        })
    };
    match attribute {
        "subject" => rule(SmartField::Subject, text_op(op)?),
        "from" => rule(SmartField::From, text_op(op)?),
        "to" | "cc" | "to or cc" => rule(SmartField::To, text_op(op)?),
        "body" => match op {
            "contains" => rule(SmartField::Body, SmartOp::Contains),
            "doesn't contain" => rule(SmartField::Body, SmartOp::NotContains),
            _ => None,
        },
        "status" => {
            let unread = match (value, op) {
                ("read", "is") | ("new", "isn't") => "no",
                ("read", "isn't") | ("new", "is") => "yes",
                _ => return None,
            };
            Some(SmartRule {
                field: SmartField::Unread,
                op: SmartOp::Is,
                value: unread.into(),
            })
        }
        "has attachment status" => Some(SmartRule {
            field: SmartField::HasAttachment,
            op: if op == "isn't" {
                SmartOp::IsNot
            } else {
                SmartOp::Is
            },
            value: if value == "true" { "yes" } else { "no" }.into(),
        }),
        "age in days" => {
            let field = match op {
                "is greater than" => SmartField::OlderThanDays,
                "is less than" => SmartField::NewerThanDays,
                _ => return None,
            };
            value.parse::<u32>().ok()?;
            rule(field, SmartOp::Is)
        }
        _ => None,
    }
}

/// Apple Mail's smart mailboxes file, in the newest `V*` directory there is.
fn apple_mail_smart_mailboxes() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let mail = home.join("Library/Mail");
    let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir(&mail)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix('V')?.parse().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|(_, dir)| dir.join("MailData/SmartMailboxes.plist"))
        .find(|path| path_exists(path))
}

fn path_exists(path: &Path) -> bool {
    // A file this program may not read still exists, and saying why it could
    // not be read is better than saying there was nothing.
    std::fs::symlink_metadata(path).is_ok()
}

/// Every smart mailbox in Apple Mail's file, folders of them flattened.
pub(crate) fn apple_mail_mailboxes(value: &plist::Value) -> Vec<FoundSmartMailbox> {
    let mut found = Vec::new();
    let items = match value {
        plist::Value::Array(items) => items.clone(),
        plist::Value::Dictionary(dict) => dict
            .get("mailboxes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    for item in &items {
        collect_apple(item, &mut found);
    }
    found
}

fn collect_apple(item: &plist::Value, found: &mut Vec<FoundSmartMailbox>) {
    let Some(dict) = item.as_dictionary() else {
        return;
    };
    if let Some(children) = dict.get("MailboxChildren").and_then(|v| v.as_array()) {
        for child in children {
            collect_apple(child, found);
        }
    }
    let Some(criteria) = dict.get("MailboxCriteria").and_then(|v| v.as_array()) else {
        return;
    };
    let name = dict
        .get("MailboxName")
        .and_then(|v| v.as_string())
        .unwrap_or("Imported")
        .to_string();
    let match_all = match dict.get("MailboxAllCriteriaMustBeSatisfied") {
        Some(plist::Value::Boolean(all)) => *all,
        Some(plist::Value::String(all)) => all.eq_ignore_ascii_case("yes"),
        _ => true,
    };

    let mut rules = Vec::new();
    let mut skipped = Vec::new();
    for criterion in criteria {
        let Some(criterion) = criterion.as_dictionary() else {
            continue;
        };
        let text = |key: &str| {
            criterion
                .get(key)
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        let header = text("Header");
        let qualifier = text("Qualifier");
        let expression = text("Expression");
        match apple_rule(header, qualifier, expression) {
            Some(rule) => rules.push(rule),
            None => skipped.push(
                format!("{header} {qualifier} {expression}")
                    .trim()
                    .to_string(),
            ),
        }
    }
    let rules = keep_valid(rules, &mut skipped);
    found.push(FoundSmartMailbox {
        name,
        source: "apple_mail".into(),
        account_id: None,
        query: SmartQuery { match_all, rules },
        skipped,
        exists: false,
    });
}

fn apple_rule(header: &str, qualifier: &str, expression: &str) -> Option<SmartRule> {
    let op = match qualifier {
        "Contains" => SmartOp::Contains,
        "DoesNotContain" => SmartOp::NotContains,
        "BeginsWith" => SmartOp::BeginsWith,
        "EndsWith" => SmartOp::EndsWith,
        "IsEqualTo" => SmartOp::Is,
        "IsNotEqualTo" => SmartOp::IsNot,
        _ => return None,
    };
    let field = match header {
        "From" | "Sender" => SmartField::From,
        "To" | "Cc" | "AnyRecipient" => SmartField::To,
        "Subject" => SmartField::Subject,
        "Body" | "EntireMessage" => match op {
            SmartOp::Contains | SmartOp::NotContains => SmartField::Body,
            _ => return None,
        },
        _ => return None,
    };
    Some(SmartRule {
        field,
        op,
        value: expression.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: &str = "version=1
uri=imap://erika%40example.de@imap.example.de/Rechnungen
scope=imap://erika%40example.de@imap.example.de/INBOX
terms=AND (subject,contains,Rechnung) AND (status,isn't,read) AND (date,is before,01-Jan-2026)
searchOnline=false
uri=mailbox://nobody@Local%20Folders/Big%20senders
scope=imap://erika%40example.de@imap.example.de/INBOX|imap://erika%40example.de@imap.example.de/Archive
terms=OR (from,ends with,@shop.example) OR (subject,contains,\"sale, today\")
searchOnline=false
";

    #[test]
    fn thunderbird_virtual_folders_become_rules_and_say_what_they_lost() {
        let folders = thunderbird_virtual_folders(TB);
        assert_eq!(folders.len(), 2);

        let bills = &folders[0];
        assert_eq!(bills.name, "Rechnungen");
        assert_eq!(
            bills.account_hint,
            Some(("erika@example.de".into(), "imap.example.de".into()))
        );
        assert!(bills.query.match_all);
        let fields: Vec<_> = bills.query.rules.iter().map(|r| r.field).collect();
        assert_eq!(
            fields,
            [SmartField::Subject, SmartField::Unread, SmartField::Folder]
        );
        assert_eq!(bills.query.rules[1].value, "yes");
        assert_eq!(bills.skipped, ["date is before 01-Jan-2026"]);

        let big = &folders[1];
        assert_eq!(big.name, "Big senders");
        assert_eq!(big.account_hint, None);
        assert!(!big.query.match_all);
        assert_eq!(big.query.rules[1].value, "sale, today");
        assert_eq!(big.skipped, ["searched only in INBOX, Archive"]);
    }

    #[test]
    fn apple_mail_smart_mailboxes_are_read_folders_and_all() {
        use plist::{Dictionary, Value};
        let criterion = |header: &str, qualifier: &str, expression: &str| {
            let mut d = Dictionary::new();
            d.insert("Header".into(), Value::String(header.into()));
            d.insert("Qualifier".into(), Value::String(qualifier.into()));
            d.insert("Expression".into(), Value::String(expression.into()));
            Value::Dictionary(d)
        };
        let mut mailbox = Dictionary::new();
        mailbox.insert("MailboxName".into(), Value::String("Receipts".into()));
        mailbox.insert(
            "MailboxAllCriteriaMustBeSatisfied".into(),
            Value::String("NO".into()),
        );
        mailbox.insert(
            "MailboxCriteria".into(),
            Value::Array(vec![
                criterion("Subject", "Contains", "receipt"),
                criterion("From", "EndsWith", "@shop.example"),
                criterion("MessageIsFlagged", "", ""),
            ]),
        );
        let mut folder = Dictionary::new();
        folder.insert(
            "MailboxChildren".into(),
            Value::Array(vec![Value::Dictionary(mailbox)]),
        );

        let found = apple_mail_mailboxes(&Value::Array(vec![Value::Dictionary(folder)]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Receipts");
        assert!(!found[0].query.match_all);
        assert_eq!(found[0].query.rules.len(), 2);
        assert_eq!(found[0].skipped, ["MessageIsFlagged"]);
    }
}
