//! Plain data types crossing the store boundary.
//!
//! Deliberately free of `mail-parser` types: the store should not care which
//! parser produced a message, and keeping the boundary plain makes it possible
//! to construct fixtures in tests without building a MIME document.

use crate::MessageSummary;

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
    /// `"microsoft"` or `"google"`, for `oauth2` accounts. Decides which grant
    /// the login uses — they are not interchangeable.
    pub oauth_provider: Option<String>,
}

impl From<&Account> for NewAccount {
    /// So a settings form can be loaded, edited and written back without
    /// restating every field it did not touch.
    fn from(account: &Account) -> Self {
        Self {
            label: account.label.clone(),
            email: account.email.clone(),
            imap_host: account.imap_host.clone(),
            imap_port: account.imap_port,
            imap_security: account.imap_security.clone(),
            username: account.username.clone(),
            auth_method: account.auth_method.clone(),
            oauth_client_id: account.oauth_client_id.clone(),
            oauth_tenant: account.oauth_tenant.clone(),
            smtp: account.smtp.clone(),
            oauth_provider: account.oauth_provider.clone(),
        }
    }
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
    /// `"microsoft"` or `"google"`, for `oauth2` accounts.
    pub oauth_provider: Option<String>,
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
    /// To and Cc addresses, lowercased and comma-separated, for smart
    /// mailboxes that ask who a message was sent to.
    pub recipients: Option<String>,
    /// `List-Unsubscribe`, as sent: one or more `<…>` URIs.
    pub list_unsubscribe: Option<String>,
    /// `List-Unsubscribe-Post` (RFC 8058). Present when the sender accepts a
    /// one-click unsubscribe, which is the only kind that can be automated
    /// safely.
    pub list_unsubscribe_post: Option<String>,
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
    /// The user said so. Outranks both of the above, and is kept apart from
    /// them on purpose: a correction is not something the classifier worked
    /// out, and filing it as a rules verdict would put the answers into the
    /// baseline the model is supposed to be measured against.
    User,
    /// An assistant filed it, through the agent surface.
    ///
    /// Shown in the list like a user's filing, because an assistant that files
    /// mail and nothing visibly changes is useless. Never learned from, because
    /// §3.4 treats corrections as the one signal that is definitely right and
    /// an assistant's judgement is not that. And never part of `disagreements`,
    /// which compares rules against the model and nothing else.
    Agent,
}

impl ClassifierSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Model => "model",
            Self::User => "user",
            Self::Agent => "agent",
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
    ///
    /// `None` means "a message with no `Message-ID` header", which is legal
    /// and does happen — not "do not check". The executor compares presence
    /// and absence alike, so finding an id where none was expected is as much
    /// a conflict as finding the wrong one.
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

/// Whether a sync should leave this folder alone.
///
/// A pattern beginning with `\` is matched against the folder's RFC 6154
/// special-use attribute, anything else against its name. The attribute form
/// is the more robust of the two — `\All` means Gmail's All Mail whatever
/// Google decides to call it — but the name form is what someone reading
/// `kuverta check` can copy.
///
/// Matching is case-insensitive, because IMAP folder names are compared that
/// way in every other part of this codebase and a skip that silently did not
/// apply would be worse than one that over-applied.
pub fn folder_is_excluded(patterns: &[String], name: &str, special_use: Option<&str>) -> bool {
    patterns.iter().any(|pattern| {
        if let Some(attribute) = pattern.strip_prefix('\\') {
            special_use
                .and_then(|s| s.strip_prefix('\\'))
                .is_some_and(|s| s.eq_ignore_ascii_case(attribute))
        } else {
            name.eq_ignore_ascii_case(pattern)
        }
    })
}

/// What to narrow the message list to. All-`None` means everything.
///
/// The fields combine rather than override: folder, category and unread are
/// three independent questions and a sidebar asks several at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    /// A rules category, e.g. `"transactional"`.
    pub category: Option<String>,
    pub unread_only: bool,
    /// Restrict to messages with a copy in this folder.
    pub folder: Option<FolderId>,
    /// Oldest first instead of newest first — for reading a folder forwards,
    /// or reaching the start of a long one without scrolling to the end.
    pub oldest_first: bool,
    /// A smart mailbox's rules, combined with the rest like any filter.
    pub smart: Option<crate::smart::SmartQuery>,
    /// Leave out mail you wrote or threw away — see [`holds_incoming`]. What
    /// "All mail" means; a folder listing is of that folder either way.
    pub incoming_only: bool,
}

/// Names a client might have given the folder behind each RFC 6154 attribute.
///
/// The fallbacks exist because plenty of servers — Dovecot in its default
/// configuration among them — advertise no special-use attributes at all, and
/// guessing from a known list beats creating a second Sent folder beside the
/// one the user's other clients already use. German names are here for the same
/// reason the classifier is bilingual.
///
/// They live in the store rather than in the protocol layer because they are
/// not about IMAP: they are what people call these folders, and both folder
/// resolution and the order a sidebar lists them in need the same answer.
pub const SENT_NAMES: &[&str] = &[
    "Sent",
    "Sent Items",
    "Sent Messages",
    // Gmail's own.
    "Sent Mail",
    "INBOX.Sent",
    "Gesendet",
    "Gesendete Objekte",
    "Gesendete Elemente",
];

pub const ARCHIVE_NAMES: &[&str] = &["Archive", "Archiv", "Archived"];

/// Gmail has no Archive folder; archiving there means All Mail.
///
/// Kept apart from [`ARCHIVE_NAMES`] because for *resolution* it must be tried
/// last — a server with both a real Archive and an All Mail should file into
/// the Archive — while for ordering a sidebar the two belong in the same place.
pub const ALL_MAIL_NAMES: &[&str] = &["All Mail"];

pub const DRAFTS_NAMES: &[&str] = &["Drafts", "Entwürfe", "Entwuerfe", "INBOX.Drafts"];

pub const JUNK_NAMES: &[&str] = &["Junk", "Spam", "Junk E-mail", "Bulk Mail"];

pub const TRASH_NAMES: &[&str] = &[
    "Trash",
    "Deleted Items",
    "Deleted Messages",
    "INBOX.Trash",
    "Papierkorb",
    "Gelöschte Objekte",
    "Gelöschte Elemente",
];

fn named_like(name: &str, known: &[&str]) -> bool {
    // The leaf, so Gmail's `[Gmail]/Sent Mail` matches on `Sent Mail`.
    let leaf = name.rsplit('/').next().unwrap_or(name);
    known.iter().any(|k| leaf.eq_ignore_ascii_case(k))
}

/// Whether a folder holds mail that arrived, as against mail you wrote
/// (Sent, Drafts) or mail that has been thrown away (Trash, Junk).
///
/// "All mail" is the list of what has been sent *to* you: a reply of your own
/// sitting between two letters is the thread showing up twice, and nothing in
/// Trash is waiting for anybody. Each folder is still there in the sidebar,
/// and search looks everywhere, so nothing is hidden — only kept out of the
/// one list that is read top to bottom.
pub fn holds_incoming(name: &str, special_use: Option<&str>) -> bool {
    !matches!(
        special_use,
        Some("\\Sent" | "\\Drafts" | "\\Trash" | "\\Junk")
    ) && ![SENT_NAMES, DRAFTS_NAMES, TRASH_NAMES, JUNK_NAMES]
        .iter()
        .any(|known| named_like(name, known))
}

/// A folder and what is in it, for the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSummary {
    pub id: FolderId,
    pub name: String,
    /// RFC 6154 attribute, when the server declares one.
    pub special_use: Option<String>,
    pub total: usize,
    pub unread: usize,
}

impl FolderSummary {
    /// Sort key, so a sidebar reads the way every other mail client's does.
    ///
    /// Alphabetical would put Archive above INBOX and bury Trash in the
    /// middle. The conventional order is worth more than the simple rule:
    /// people look for these by position, not by name.
    pub fn rank(&self) -> u8 {
        if self.name.eq_ignore_ascii_case("INBOX") {
            return 0;
        }

        // The attribute first, because it is what the server asserts.
        match self.special_use.as_deref() {
            Some("\\Drafts") => return 1,
            Some("\\Sent") => return 2,
            Some("\\Archive") | Some("\\All") => return 3,
            Some("\\Junk") => return 5,
            Some("\\Trash") => return 6,
            _ => {}
        }

        // Then the name, so a folder the app already treats as the archive
        // does not sit among the user's own folders in the list. Dovecot will
        // not let a client set a special-use attribute at all, so an Archive
        // created from inside this app has none — and would otherwise sort
        // wherever the alphabet put it.
        if named_like(&self.name, DRAFTS_NAMES) {
            1
        } else if named_like(&self.name, SENT_NAMES) {
            2
        } else if named_like(&self.name, ARCHIVE_NAMES) || named_like(&self.name, ALL_MAIL_NAMES) {
            3
        } else if named_like(&self.name, JUNK_NAMES) {
            5
        } else if named_like(&self.name, TRASH_NAMES) {
            6
        } else {
            // Everything the user made themselves sits between the archive and
            // the bins, which is where it belongs: it is theirs, not plumbing.
            4
        }
    }
}

/// A message as the list shows it.
#[derive(Debug, Clone)]
pub struct ListedMessage {
    pub summary: MessageSummary,
    pub category: Option<String>,
    pub confidence: Option<f64>,
    /// True when no copy of this message anywhere carries `\Seen`.
    pub unread: bool,
}

/// One window of the list, plus how many rows there are in total.
///
/// The total is what a virtualized list sizes its scrollbar against, so it has
/// to come back with the window rather than from a second round trip that
/// could disagree with it.
#[derive(Debug, Clone)]
pub struct MessageWindow {
    pub total: usize,
    pub offset: usize,
    pub messages: Vec<ListedMessage>,
}
