//! Plain data types crossing the store boundary.
//!
//! Deliberately free of `mail-parser` types: the store should not care which
//! parser produced a message, and keeping the boundary plain makes it possible
//! to construct fixtures in tests without building a MIME document.

pub type AccountId = i64;
pub type FolderId = i64;
pub type MessageId = i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImapSecurity {
    /// Implicit TLS, normally port 993.
    Tls,
    /// Cleartext connection upgraded via STARTTLS, normally port 143.
    StartTls,
    /// No transport security. Only ever valid against the local dev server.
    Plaintext,
}

impl ImapSecurity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::Plaintext => "plaintext",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::StartTls),
            "plaintext" => Some(Self::Plaintext),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub label: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ImapSecurity,
    pub username: String,
    /// Matches the auth strategy in `core-accounts`: "app_password" | "oauth2".
    pub auth_method: String,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: AccountId,
    pub label: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ImapSecurity,
    pub username: String,
    pub auth_method: String,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub name: String,
    pub special_use: Option<String>,
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub highest_modseq: Option<u64>,
}

/// A message as handed to the store by the sync layer.
#[derive(Debug, Clone, Default)]
pub struct NewMessage {
    /// RFC 5322 Message-ID with angle brackets stripped. `None` is legal and
    /// does happen: see the `06-no-messageid` fixture.
    pub rfc822_message_id: Option<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    /// Unix seconds, UTC.
    pub date_utc: Option<i64>,
    pub size_bytes: Option<i64>,
    pub snippet: Option<String>,
    pub has_attachments: bool,
    /// `List-Id`, kept as a first-class column because it is the single most
    /// useful signal the rules baseline has.
    pub list_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// Path to the raw body blob on disk, relative to the blob root.
    pub body_path: Option<String>,
    /// Plain-text body used for full-text search. Not persisted as a column.
    pub search_text: Option<String>,
}

/// Where a message lives on the server. One message can have many locations —
/// this is what makes Gmail's labels-as-folders behaviour representable.
#[derive(Debug, Clone)]
pub struct Location {
    pub folder_id: FolderId,
    pub uid: u32,
    pub flags: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    /// The message was not in the store and has been created.
    Inserted,
    /// The message was already present under this account; only a new location
    /// (if any) was recorded.
    Deduplicated,
}

/// Which classifier produced a verdict. Both run on every message so their
/// disagreement can be measured — see docs/implementation-plan.md §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierSource {
    Rules,
    Model,
}

impl ClassifierSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Model => "model",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub category: String,
    pub confidence: Option<f64>,
    pub source: ClassifierSource,
    /// Model name for `Model` verdicts, `None` for rules.
    pub model: Option<String>,
    pub latency_ms: Option<i64>,
}
