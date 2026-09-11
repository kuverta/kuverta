//! The typed surface the shell talks to.
//!
//! Deliberately knows nothing about Tauri. The plan (§2) has this doubling as
//! the MCP/agent surface later, and a layer that has taken a dependency on one
//! shell is no longer a surface — so the Tauri commands in `apps/desktop` are
//! thin wrappers over these calls and nothing here mentions a window, an
//! `invoke`, or a webview.
//!
//! Everything crossing this boundary is plain, serialisable data, for the same
//! reason `core-store::model` is: the caller should not need `rusqlite` types
//! to ask a question, and a test should be able to assert on an answer without
//! rendering it.
//!
//! ## Windows, not mailboxes
//!
//! [`Core::messages`] takes an offset and a limit and returns a total. That is
//! the one shape the list may use: the Tauri spike measured the Rust/JS bridge
//! at roughly 78 MiB/s of JSON, so passing a whole mailbox across it is the
//! single thing that would undo the 59.8 fps it otherwise reaches. See
//! docs/spike-tauri-list.md.

pub mod paper;
pub mod session;
pub mod settings;

pub use core_paper::PaperReport;
pub use paper::{
    PaperDetail, PaperMailboxInput, PaperMailboxView, PaperPage, PaperRow, PaperSession,
};
pub use session::{
    DraftInput, DraftPreview, SentSummary, Session, SyncSummary, VerifiedFolder, VerifyReport,
};
pub use settings::{AccountInput, AccountSettings};

use std::path::Path;

use core_store::model::{
    AccountId, ListFilter, ListedMessage, MessageId, NewOperation, OperationKind, OperationState,
};
use core_store::{Blobs, Store};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error(transparent)]
    Store(#[from] core_store::StoreError),

    #[error("no account {0}")]
    UnknownAccount(String),

    #[error("no message {0}")]
    UnknownMessage(MessageId),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Rejected(String),

    #[error("network: {0}")]
    Network(String),

    #[error("credentials: {0}")]
    Auth(String),
}

pub type Result<T> = std::result::Result<T, RpcError>;

/// How long a change waits before a sync may send it, so it can still be undone.
pub const DEFAULT_UNDO_WINDOW_SECS: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountView {
    pub id: AccountId,
    pub email: String,
    pub label: String,
    /// False when no submission endpoint is configured, which the UI needs in
    /// order to grey out compose rather than fail at send time.
    pub can_send: bool,
}

/// A row as the list renders it.
///
/// `date_utc` rather than a formatted string: the window is a screenful, so
/// formatting it in the shell costs nothing and keeps locale out of the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRow {
    pub id: MessageId,
    pub date_utc: Option<i64>,
    pub from: String,
    pub subject: String,
    pub unread: bool,
    pub has_attachments: bool,
    pub list_id: Option<String>,
    pub category: Option<String>,
    /// First line or so of the body. A list showing only sender and subject
    /// makes you open mail to find out what it is.
    pub snippet: Option<String>,
}

impl From<ListedMessage> for MessageRow {
    fn from(listed: ListedMessage) -> Self {
        let summary = listed.summary;
        Self {
            id: summary.id,
            date_utc: summary.date_utc,
            from: summary
                .from_name
                .or(summary.from_addr)
                .unwrap_or_else(|| "(unknown)".into()),
            subject: summary.subject.unwrap_or_else(|| "(no subject)".into()),
            unread: listed.unread,
            has_attachments: summary.has_attachments,
            list_id: summary.list_id,
            category: listed.category,
            snippet: summary.snippet,
        }
    }
}

/// A folder as the sidebar draws it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderView {
    pub id: i64,
    pub name: String,
    /// The last path segment, for a nested name like `[Gmail]/All Mail`. What
    /// a sidebar shows; `name` is what IMAP commands take.
    pub label: String,
    pub special_use: Option<String>,
    pub total: usize,
    pub unread: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePage {
    /// Rows matching the filter, not just the ones in this window. The list
    /// sizes its scrollbar against it.
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<MessageRow>,
}

/// One message with enough to read it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDetail {
    pub id: MessageId,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from: Option<String>,
    pub date_utc: Option<i64>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Folders this message is in. On Gmail this is its labels.
    pub folders: Vec<String>,
    /// The plain-text body.
    ///
    /// Text rather than the raw message: parsing MIME is not the shell's job,
    /// and sending bytes it would only have to decode wastes the bridge the
    /// whole list design is built around conserving. An HTML-only message
    /// yields `None` here — rendering that is a UI question, and a later one.
    pub body_text: Option<String>,
}

/// A change that has been asked for but not yet sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedChange {
    pub id: i64,
    pub message_id: MessageId,
    /// Human-readable, e.g. `"move to Archive"`.
    pub what: String,
    /// Seconds until it may be sent. Zero once the undo window has passed.
    pub holds_for: i64,
    pub last_error: Option<String>,
}

/// What the shell may ask for.
///
/// Holds the store, which `rusqlite` makes `Send` but not `Sync`, so a caller
/// sharing one across threads wraps it — `Mutex<Core>` in the Tauri app. That
/// is not a limitation worth engineering around: the store is synchronous
/// because at personal-mailbox scale it is never the bottleneck.
pub struct Core {
    store: Store,
    blobs: Blobs,
}

impl Core {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = data_dir.as_ref();
        // Created here rather than expected to exist. A first run has no data
        // directory by definition, and failing before the window opens is the
        // worst possible moment to say so — there is nowhere to say it.
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            store: Store::open(dir.join("fuckmail.db"))?,
            blobs: Blobs::new(dir.join("blobs")),
        })
    }

    /// For callers that already have a store open, and for tests.
    pub fn new(store: Store, blobs: Blobs) -> Self {
        Self { store, blobs }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn blobs(&self) -> &Blobs {
        &self.blobs
    }

    pub fn accounts(&self) -> Result<Vec<AccountView>> {
        Ok(self
            .store
            .accounts()?
            .into_iter()
            .map(|account| AccountView {
                id: account.id,
                email: account.email,
                label: account.label,
                can_send: account.smtp.is_some(),
            })
            .collect())
    }

    /// Every folder on the account, with what is in it, in the order a
    /// sidebar should list them.
    pub fn folders(&self, account: AccountId) -> Result<Vec<FolderView>> {
        Ok(self
            .store
            .folder_summaries(account)?
            .into_iter()
            .map(|folder| FolderView {
                // Gmail nests everything under `[Gmail]/`; showing that prefix
                // on every row would be five wasted characters and no
                // information. The full name is kept because it is what the
                // server answers to.
                label: folder
                    .name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&folder.name)
                    .to_string(),
                id: folder.id,
                name: folder.name,
                special_use: folder.special_use,
                total: folder.total,
                unread: folder.unread,
            })
            .collect())
    }

    /// One window of the message list.
    pub fn messages(
        &self,
        account: AccountId,
        offset: usize,
        limit: usize,
        filter: &ListFilter,
    ) -> Result<MessagePage> {
        let window = self.store.message_window(account, offset, limit, filter)?;
        Ok(MessagePage {
            total: window.total,
            offset: window.offset,
            rows: window.messages.into_iter().map(MessageRow::from).collect(),
        })
    }

    /// Full-text search, as its own call rather than a filter.
    ///
    /// FTS5 ranks by relevance, so a search result is a different ordering of
    /// a different set — folding it into the list window would mean pretending
    /// two unrelated orderings were one.
    pub fn search(&self, account: AccountId, query: &str, limit: usize) -> Result<Vec<MessageRow>> {
        Ok(self
            .store
            .search(account, query, limit)?
            .into_iter()
            .map(|summary| MessageRow {
                id: summary.id,
                date_utc: summary.date_utc,
                from: summary
                    .from_name
                    .or(summary.from_addr)
                    .unwrap_or_else(|| "(unknown)".into()),
                subject: summary.subject.unwrap_or_else(|| "(no subject)".into()),
                // Not carried by the search index; the list window is where
                // these are answered.
                unread: false,
                has_attachments: summary.has_attachments,
                list_id: summary.list_id,
                category: None,
                snippet: summary.snippet,
            })
            .collect())
    }

    pub fn message(&self, account: AccountId, id: MessageId) -> Result<MessageDetail> {
        let stored = self
            .store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;

        let folders = self
            .store
            .locations_of(id)?
            .into_iter()
            .filter_map(|location| {
                self.store
                    .folder(location.folder_id)
                    .ok()
                    .flatten()
                    .map(|f| f.name)
            })
            .collect();

        // A body that will not load is not an error: the message row is still
        // worth showing, and the alternative is a list that cannot open a row
        // because of a file the user cannot do anything about.
        let raw = stored
            .body_path
            .as_deref()
            .and_then(|path| match self.blobs.get(path) {
                Ok(bytes) => Some(bytes),
                Err(err) => {
                    tracing::warn!(id, %err, "stored body could not be read");
                    None
                }
            });

        let parsed = raw
            .as_deref()
            .and_then(|bytes| mail_parser::MessageParser::default().parse(bytes));

        let addresses = |field: Option<&mail_parser::Address<'_>>| -> Vec<String> {
            match field {
                Some(mail_parser::Address::List(addrs)) => addrs
                    .iter()
                    .filter_map(|a| a.address().map(str::to_string))
                    .collect(),
                Some(mail_parser::Address::Group(groups)) => groups
                    .iter()
                    .flat_map(|g| g.addresses.iter())
                    .filter_map(|a| a.address().map(str::to_string))
                    .collect(),
                None => Vec::new(),
            }
        };

        Ok(MessageDetail {
            id: stored.id,
            message_id: stored.rfc822_message_id,
            subject: stored.subject,
            from: stored.from_addr,
            date_utc: stored.date_utc,
            to: parsed
                .as_ref()
                .map(|p| addresses(p.to()))
                .unwrap_or_default(),
            cc: parsed
                .as_ref()
                .map(|p| addresses(p.cc()))
                .unwrap_or_default(),
            folders,
            body_text: parsed
                .as_ref()
                .and_then(|p| p.body_text(0))
                .map(|text| text.into_owned()),
        })
    }

    /// How many messages fall in each rules category, for the filter bar.
    pub fn category_counts(&self, account: AccountId) -> Result<Vec<(String, usize)>> {
        Ok(self.store.category_counts(account)?)
    }

    // -- changes -----------------------------------------------------------

    /// Queues a move. `target` is a folder name as the server has it.
    pub fn move_to(
        &self,
        account: AccountId,
        id: MessageId,
        target: &str,
        undo_window_secs: i64,
    ) -> Result<i64> {
        self.enqueue(
            account,
            id,
            OperationKind::Move {
                target_folder: target.to_string(),
            },
            Some(target),
            undo_window_secs,
        )
    }

    /// Queues a read/unread change.
    pub fn set_read(
        &self,
        account: AccountId,
        id: MessageId,
        read: bool,
        undo_window_secs: i64,
    ) -> Result<i64> {
        self.enqueue(
            account,
            id,
            OperationKind::Flag {
                flag: "\\Seen".into(),
                set: read,
            },
            None,
            undo_window_secs,
        )
    }

    /// Files a message under a category, and records the correction.
    ///
    /// Not queued, and that is the point rather than an omission: a category
    /// lives in the store and is never sent to a server, so there is no round
    /// trip to hold back and nothing for the undo window to protect. Changing
    /// your mind is another correction, which is what the log wants anyway —
    /// an event, not an edit.
    ///
    /// Two rows are written. The correction is the training data of §3.4, and
    /// the verdict is what the list reads, recorded as `User` so that the
    /// rules-versus-model comparison never sees the answers.
    pub fn set_category(&self, account: AccountId, id: MessageId, category: &str) -> Result<()> {
        // Validated here rather than trusted: the column is free text, and a
        // typo would create a bucket the sidebar never offers and the
        // classifier can never return, holding a message nobody can find.
        let category = core_rules::Category::parse(category)
            .ok_or_else(|| RpcError::Rejected(format!("not a category: {category}")))?;

        // Confirms the message is this account's before writing anything about
        // it — ids are global to the store.
        self.store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;

        let was = self.store.current_category(id)?;
        if was.as_deref() == Some(category.as_str()) {
            // Already filed there. Recording it would put a correction in the
            // log that corrects nothing, which is noise in the one dataset
            // being kept deliberately clean.
            return Ok(());
        }

        self.store
            .record_correction(id, was.as_deref(), category.as_str())?;
        self.store.record_verdict(
            id,
            &core_store::model::Verdict {
                category: category.as_str().to_string(),
                // Not a probability. The user is not guessing.
                confidence: Some(1.0),
                source: core_store::model::ClassifierSource::User,
                model: None,
                latency_ms: None,
            },
        )?;
        Ok(())
    }

    /// Cancels the most recent change that has not been sent.
    ///
    /// `Ok(None)` means there was nothing to undo, which is an answer rather
    /// than a failure.
    pub fn undo(&self, account: AccountId) -> Result<Option<QueuedChange>> {
        Ok(self
            .store
            .cancel_latest_operation(account)?
            .map(|op| self.describe(&op, now_utc())))
    }

    pub fn queue(&self, account: AccountId) -> Result<Vec<QueuedChange>> {
        let now = now_utc();
        Ok(self
            .store
            .pending_operations(account)?
            .iter()
            .map(|op| self.describe(op, now))
            .collect())
    }

    /// Picks the copy to act on and records the intent.
    ///
    /// One message can be in several folders — on Gmail routinely — so which
    /// copy is meant has to be decided rather than assumed. INBOX wins when
    /// there is a choice, because getting it out of the inbox is what filing
    /// means; a copy already in the destination is never the one to move.
    fn enqueue(
        &self,
        account: AccountId,
        id: MessageId,
        kind: OperationKind,
        target: Option<&str>,
        undo_window_secs: i64,
    ) -> Result<i64> {
        let message = self
            .store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;

        let mut candidates: Vec<_> = self
            .store
            .locations_of(id)?
            .into_iter()
            .filter_map(|location| {
                self.store
                    .folder(location.folder_id)
                    .ok()
                    .flatten()
                    .map(|folder| (location, folder))
            })
            .filter(|(_, folder)| target != Some(folder.name.as_str()))
            .collect();

        if candidates.is_empty() {
            return Err(RpcError::Rejected(match target {
                Some(folder) => format!("that message is already in {folder}"),
                None => "that message is not in any synced folder".into(),
            }));
        }
        candidates.sort_by_key(|(_, folder)| folder.name != "INBOX");
        let (location, folder) = candidates.remove(0);

        Ok(self.store.enqueue_operation(&NewOperation {
            account_id: account,
            message_id: id,
            kind,
            source_folder_id: folder.id,
            source_uid: location.uid,
            source_uid_validity: folder.uid_validity,
            expect_message_id: message.rfc822_message_id,
            execute_after: now_utc() + undo_window_secs,
        })?)
    }

    fn describe(&self, op: &core_store::model::Operation, now: i64) -> QueuedChange {
        QueuedChange {
            id: op.id,
            message_id: op.message_id,
            what: match &op.kind {
                OperationKind::Move { target_folder } => format!("move to {target_folder}"),
                OperationKind::Flag { flag, set: true } => format!("set {flag}"),
                OperationKind::Flag { flag, .. } => format!("clear {flag}"),
            },
            holds_for: (op.execute_after - now).max(0),
            last_error: op
                .last_error
                .clone()
                .filter(|_| op.state == OperationState::Pending),
        }
    }
}

fn now_utc() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
