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

pub mod ai;
pub mod paper;
pub mod session;
pub mod settings;
pub mod setup;

pub use ai::{AiChoice, AiProviderInput, AiProviderView, AiTaskView, AiTrial, Task};
pub use core_ai::{ModelInfo, Provider as ModelProvider};
pub use core_paper::PaperReport;
pub use paper::{
    PaperDetail, PaperMailboxInput, PaperMailboxView, PaperPage, PaperRow, PaperSession,
};
pub use session::{
    DraftInput, DraftPreview, SavedDraft, SentSummary, Session, SyncSummary, VerifiedFolder,
    VerifyReport,
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
    /// What the rules classifier reads, so the shell can say why the message
    /// was filed where it was. `None` when the message is not on disk: the
    /// stored row keeps too few of the headers to explain from, and an
    /// explanation built from half the evidence would be confidently wrong.
    pub facts: Option<FactsView>,
    /// The plain-text body.
    ///
    /// Text rather than the raw message: parsing MIME is not the shell's job,
    /// and sending bytes it would only have to decode wastes the bridge the
    /// whole list design is built around conserving. An HTML-only message
    /// yields `None` here — rendering that is a UI question, and a later one.
    pub body_text: Option<String>,
}

/// What a model pass did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelPass {
    pub model: String,
    /// Messages that had no verdict from this model when the pass started.
    pub waiting: usize,
    pub classified: usize,
    /// Replies that named no single category, recorded as Unknown.
    pub unparseable: usize,
    pub failed: usize,
    pub mean_latency_ms: Option<i64>,
    /// Set when the pass gave up — three failures in a row — with the last reason.
    pub stopped: Option<String>,
}

/// The facts the rules classifier reads, as the shell's classifier takes them.
///
/// camelCase because the other end is `kuverta-bird/core/facts.js`, which runs the
/// same rules — ported line for line and verified against `core-rules` — to
/// explain a verdict in the reading pane. The explanation is the half of the
/// rules layer that makes a wrong answer arguable rather than annoying, and it
/// needs exactly these fields and no others.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactsView {
    pub from_addr: Option<String>,
    pub from_name: Option<String>,
    pub subject: Option<String>,
    pub list_id: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub precedence: Option<String>,
    pub auto_submitted: Option<String>,
    pub in_reply_to: Option<String>,
    pub has_attachments: bool,
    pub recipient_count: usize,
    pub snippet: Option<String>,
}

impl From<&core_rules::MessageFacts<'_>> for FactsView {
    fn from(facts: &core_rules::MessageFacts<'_>) -> Self {
        let owned = |value: Option<&str>| value.map(str::to_string);
        Self {
            from_addr: owned(facts.from_addr),
            from_name: owned(facts.from_name),
            subject: owned(facts.subject),
            list_id: owned(facts.list_id),
            list_unsubscribe: owned(facts.list_unsubscribe),
            precedence: owned(facts.precedence),
            auto_submitted: owned(facts.auto_submitted),
            in_reply_to: owned(facts.in_reply_to),
            has_attachments: facts.has_attachments,
            recipient_count: facts.recipient_count,
            snippet: owned(facts.snippet),
        }
    }
}

/// Where archiving and trashing move mail to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialFolders {
    /// `None` when the account has no Archive folder — which a shell has to
    /// treat as "cannot archive" rather than failing per keystroke.
    pub archive: Option<String>,
    pub trash: Option<String>,
}

/// Where the store lives unless told otherwise.
///
/// `KUVERTA_DATA_DIR`, else `~/.local/share/kuverta`. In one place because
/// the window and the agent surface must open the same store — an assistant
/// reading a different mailbox from the one on screen would be worse than no
/// assistant.
pub fn default_data_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("KUVERTA_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".local/share/kuverta"))
        .unwrap_or_else(|| std::path::PathBuf::from(".kuverta"))
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
            store: Store::open(dir.join("kuverta.db"))?,
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

        // Re-parsed for the classifier's facts. A second parse of one message,
        // on open, is cheap; storing every header the rules read for every
        // message in case one is opened is not.
        let facts = raw
            .as_deref()
            .and_then(|bytes| core_proto::parse::parse_message(bytes, None))
            .map(|parsed| FactsView::from(&parsed.facts.as_message_facts()));

        Ok(MessageDetail {
            facts,
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
        if self.store.user_category(id)?.as_deref() == Some(category.as_str()) {
            // Already filed there by the user. Recording it would put a correction in the
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

    /// Files a message under a category on an assistant's say-so.
    ///
    /// Shown in the list exactly as a user's filing is, and recorded as
    /// [`ClassifierSource::Agent`] rather than as a correction. The difference
    /// matters more than it looks: corrections are what the classifier learns
    /// from, and they are trusted because a person made them. An assistant's
    /// filing is a suggestion that happens to be visible — if the user agrees
    /// and files it the same way, *that* is recorded as a correction.
    ///
    /// [`ClassifierSource::Agent`]: core_store::model::ClassifierSource::Agent
    pub fn suggest_category(
        &self,
        account: AccountId,
        id: MessageId,
        category: &str,
    ) -> Result<()> {
        let category = core_rules::Category::parse(category)
            .ok_or_else(|| RpcError::Rejected(format!("not a category: {category}")))?;
        self.store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;

        if self.store.current_category(id)?.as_deref() == Some(category.as_str()) {
            return Ok(());
        }

        self.store.record_verdict(
            id,
            &core_store::model::Verdict {
                category: category.as_str().to_string(),
                // An assistant's filing is not a probability either, but it is
                // not certain the way a person's is. No number is more honest
                // than a made-up one.
                confidence: None,
                source: core_store::model::ClassifierSource::Agent,
                model: None,
                latency_ms: None,
            },
        )?;
        Ok(())
    }

    /// Where archiving and trashing move mail to, for one account.
    ///
    /// Moved here from the desktop app, which was reaching into `core-proto`
    /// for it — the agent surface needs the same answer, and two shells
    /// deciding separately where "Archive" is would be two places to disagree.
    pub fn special_folders(&self, account: AccountId) -> Result<SpecialFolders> {
        let folders = self.store.folders(account)?;
        let pairs: Vec<(&str, Option<&str>)> = folders
            .iter()
            .map(|f| (f.name.as_str(), f.special_use.as_deref()))
            .collect();
        Ok(SpecialFolders {
            archive: core_proto::client::find_archive(pairs.iter().copied()).map(str::to_string),
            trash: core_proto::client::find_trash(pairs.iter().copied()).map(str::to_string),
        })
    }

    /// Runs the local model over mail it has not classified yet.
    ///
    /// Brief §3.3, all three rules at once. **After sync, not during**: this is
    /// its own call, so a slow model can never make a sync slow. **Log both**:
    /// the verdict is recorded as `model` beside the rules' verdict, keyed by
    /// the model's name, and `disagreements` compares them. **Never shown**:
    /// the list does not read model verdicts, so a model that is wrong for a
    /// month moves nobody's mail.
    ///
    /// Stops after three failures in a row rather than grinding through a
    /// mailbox against a server that is down — each failure can be a long
    /// timeout, and a hundred of them is an afternoon.
    pub async fn model_pass(
        &self,
        account: AccountId,
        server: &impl core_ai::Chat,
        classifier: &core_ai::PromptClassifier,
        limit: usize,
    ) -> Result<ModelPass> {
        let waiting = self
            .store
            .awaiting_model_verdict(account, &classifier.model, limit)?;
        let mut pass = ModelPass {
            model: classifier.model.clone(),
            waiting: waiting.len(),
            ..ModelPass::default()
        };

        let mut latency_total: i64 = 0;
        let mut failures_in_a_row = 0;

        for summary in &waiting {
            let facts = self.facts_for_model(account, summary);
            match classifier.classify(server, &facts.as_message_facts()).await {
                Ok(verdict) => {
                    failures_in_a_row = 0;
                    // An answer that names no single category is the model not
                    // knowing, which is exactly what Unknown means — recorded as
                    // such, and counted, so a model that cannot follow the
                    // instruction shows up as one.
                    if verdict.category.is_none() {
                        pass.unparseable += 1;
                        tracing::debug!(message = summary.id, reply = %verdict.raw, "unparseable model reply");
                    }
                    let category = verdict.category.unwrap_or(core_rules::Category::Unknown);
                    self.store.record_verdict(
                        summary.id,
                        &core_store::model::Verdict {
                            category: category.as_str().to_string(),
                            confidence: None,
                            source: core_store::model::ClassifierSource::Model,
                            model: Some(classifier.model.clone()),
                            latency_ms: Some(verdict.latency_ms),
                        },
                    )?;
                    pass.classified += 1;
                    latency_total += verdict.latency_ms;
                }
                Err(err) => {
                    pass.failed += 1;
                    failures_in_a_row += 1;
                    tracing::warn!(message = summary.id, %err, "the model did not answer");
                    if failures_in_a_row >= 3 {
                        pass.stopped = Some(err.to_string());
                        break;
                    }
                }
            }
        }

        pass.mean_latency_ms =
            (pass.classified > 0).then(|| latency_total / pass.classified as i64);
        Ok(pass)
    }

    /// The facts a model is shown for a stored message.
    ///
    /// Re-parsed from the message on disk where there is one, because the store
    /// keeps only some of what the rules read at sync time — `List-Unsubscribe`,
    /// `Precedence` and `Auto-Submitted` are read then and not kept. Without the
    /// body on disk the summary stands in, and the model sees less; that is
    /// worth knowing when reading its disagreements.
    fn facts_for_model(
        &self,
        account: AccountId,
        summary: &core_store::MessageSummary,
    ) -> core_proto::parse::ClassifyFacts {
        self.store
            .message_by_id(account, summary.id)
            .ok()
            .flatten()
            .and_then(|stored| stored.body_path)
            .and_then(|path| self.blobs.get(path.as_str()).ok())
            .and_then(|raw| core_proto::parse::parse_message(&raw, None))
            .map(|parsed| parsed.facts)
            .unwrap_or_else(|| core_proto::parse::ClassifyFacts {
                from_addr: summary.from_addr.clone(),
                from_name: summary.from_name.clone(),
                subject: summary.subject.clone(),
                list_id: summary.list_id.clone(),
                has_attachments: summary.has_attachments,
                snippet: summary.snippet.clone(),
                ..Default::default()
            })
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
