//! Plain data types crossing the store boundary.
//!
//! Deliberately free of `mail-parser` types: the store should not care which
//! parser produced a message, and keeping the boundary plain makes it possible
//! to construct fixtures in tests without building a MIME document.

pub type AccountId = i64;
pub type FolderId = i64;
pub type MessageId = i64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ImapSecurity {
    /// Implicit TLS, normally port 993.
    #[default]
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

/// Transport security for SMTP submission.
///
/// Deliberately a separate type from [`ImapSecurity`] rather than a shared one:
/// the two protocols have different conventional ports, and a value that means
/// "993" must never be silently usable where one meaning "465" is expected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// Implicit TLS, normally port 465.
    #[default]
    Tls,
    /// Cleartext connection upgraded via STARTTLS, normally port 587.
    StartTls,
    /// No transport security. Only ever valid against the local dev sink.
    Plaintext,
}

impl SmtpSecurity {
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

/// Where an account submits outgoing mail.
///
/// All-or-nothing: an account either has a complete submission endpoint or
/// cannot send at all, which is why `Account::smtp` is one `Option` rather
/// than three independently-nullable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
}

#[derive(Debug, Clone, Default)]
pub struct NewAccount {
    pub label: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ImapSecurity,
    pub username: String,
    /// Matches the auth strategy in `core-accounts`: "app_password" | "oauth2".
    pub auth_method: String,
    /// Azure AD (or equivalent) application id, for `oauth2` accounts.
    pub oauth_client_id: Option<String>,
    /// Directory id, or "common" for personal Microsoft accounts.
    pub oauth_tenant: Option<String>,
    /// Submission endpoint. `None` leaves the account receive-only.
    pub smtp: Option<SmtpConfig>,
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
    pub oauth_client_id: Option<String>,
    pub oauth_tenant: Option<String>,
    /// Submission endpoint, when one has been configured. `None` means the
    /// account can receive but not send.
    pub smtp: Option<SmtpConfig>,
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

/// A stored message, with enough to find its raw blob again.
///
/// Distinct from `MessageSummary`, which is what a list view needs; this is
/// what a reply needs, and the difference is `body_path`.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: MessageId,
    pub rfc822_message_id: Option<String>,
    pub subject: Option<String>,
    pub from_addr: Option<String>,
    pub date_utc: Option<i64>,
    /// Path to the raw message on disk, relative to the blob root. `None` when
    /// the message was stored without its body.
    pub body_path: Option<String>,
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

pub type OperationId = i64;

/// What a queued mutation does.
///
/// Two kinds cover everything stage 2 promises: archive, delete and move are
/// all "move to folder X", and mark-read/unread is "set or clear one flag".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationKind {
    /// Move the message to `target_folder`.
    ///
    /// Deliberately no `Expunge` sibling. Deleting means moving to Trash, so
    /// nothing this client does destroys mail permanently — the one part of
    /// the original read-only safety story that survives stage 2 intact.
    Move { target_folder: String },
    /// Add or remove a single IMAP flag, e.g. `\Seen`.
    Flag { flag: String, set: bool },
}

impl OperationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Move { .. } => "move",
            Self::Flag { .. } => "flag",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    /// Queued. Cancellable, and eligible for the server once `execute_after`
    /// has passed.
    Pending,
    /// The server accepted it.
    Done,
    /// Abandoned. Either the user cancelled it or it was superseded by a
    /// later change — `last_error` says which.
    Cancelled,
    /// No longer applicable: the world moved on, and the intent was either
    /// already satisfied or has nothing left to act on. Not an error.
    Obsolete,
    /// Refused. The server no longer matches what was recorded, so applying
    /// the operation would act on the wrong message.
    Failed,
}

impl OperationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Obsolete => "obsolete",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "done" => Some(Self::Done),
            "cancelled" => Some(Self::Cancelled),
            "obsolete" => Some(Self::Obsolete),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A mutation to queue.
#[derive(Debug, Clone)]
pub struct NewOperation {
    pub account_id: AccountId,
    pub message_id: MessageId,
    pub kind: OperationKind,
    /// Where the message was when the user acted, and what the executor will
    /// verify before touching the server.
    pub source_folder_id: FolderId,
    pub source_uid: u32,
    pub source_uid_validity: Option<u32>,
    /// The Message-ID the caller believes lives at that UID.
    pub expect_message_id: Option<String>,
    /// Unix seconds before which this must not be sent to the server.
    pub execute_after: i64,
}

/// A queued mutation as stored.
#[derive(Debug, Clone)]
pub struct Operation {
    pub id: OperationId,
    pub account_id: AccountId,
    pub message_id: MessageId,
    pub kind: OperationKind,
    pub source_folder_id: FolderId,
    pub source_uid: u32,
    pub source_uid_validity: Option<u32>,
    pub expect_message_id: Option<String>,
    pub state: OperationState,
    pub execute_after: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}
