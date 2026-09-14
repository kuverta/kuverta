//! The half of the core that touches the network.
//!
//! [`crate::Core`] answers from the local store and never blocks on anything;
//! this connects, authenticates, and moves mail. They are separate because a
//! window has to keep drawing while a sync runs, and because everything worth
//! testing about the store side is testable without a server.
//!
//! ## A sync owns its own connection
//!
//! The shell holds one [`crate::Core`] behind a lock and serves the list from
//! it. A sync cannot borrow that — it runs for as long as the network takes,
//! and a lock held across that is a frozen window. So a sync opens its own
//! connection to the same database instead. SQLite in WAL mode, which
//! `core-store` sets, is built for exactly this: one writer and any number of
//! readers, concurrently.
//!
//! ## Which grant, and where the secret is
//!
//! Assembling an [`AuthProvider`] from a stored account is the one piece of
//! knowledge both the CLI and the window need and neither should own. The
//! account row says which provider and which grant; the credential itself is
//! always in the keychain and never in the database.

use std::path::{Path, PathBuf};

use core_accounts::loopback::{LoopbackConfig, OAuth2Loopback};
use core_accounts::oauth::{OAuth2Config, OAuth2Device};
use core_accounts::{AuthProvider, EnvPassword, KeychainPassword};
use core_smtp::{Draft, Mailbox, ReplyMode, ReplySource};
use core_store::model::{Account, AccountId, MessageId};
use core_store::{Blobs, Store};
use serde::{Deserialize, Serialize};

use crate::{Result, RpcError};

impl From<core_proto::ProtoError> for RpcError {
    fn from(err: core_proto::ProtoError) -> Self {
        RpcError::Network(err.to_string())
    }
}

impl From<core_accounts::AuthError> for RpcError {
    fn from(err: core_accounts::AuthError) -> Self {
        RpcError::Auth(err.to_string())
    }
}

/// What one account's sync did.
///
/// Flattened from the two reports underneath — the changes that went out and
/// the mail that came back — because to the person watching it is one action.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSummary {
    pub email: String,

    /// Queued changes the server accepted.
    pub changes_sent: usize,
    /// Changes that no longer applied, because the world had moved on.
    pub changes_obsolete: usize,
    /// Changes refused because the server no longer matched what was recorded.
    /// These need looking at.
    pub changes_refused: usize,
    /// Changes left pending after a failure worth retrying.
    pub changes_retryable: usize,
    /// Moves the server could not do atomically, leaving a flagged copy in the
    /// source folder. Rare, and worth saying out loud when it happens.
    pub non_atomic_moves: usize,

    pub folders_synced: usize,
    pub folders_skipped: usize,
    pub folders_excluded: usize,
    pub inserted: usize,
    pub deduplicated: usize,
    pub flag_updates: usize,
    pub expunged: usize,
    pub deleted: usize,
    pub unparseable: usize,
    pub invalidated: usize,
}

/// A message to send, as the caller describes it.
///
/// Addresses are strings because that is what a person types and what a UI
/// holds; parsing and validating them is `core-smtp`'s job, and doing it there
/// means the window and the CLI reject the same things for the same reasons.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DraftInput {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    /// What the sender typed. For a reply this goes above the quoted original.
    pub body: String,
    /// Reply to this stored message, by row id.
    pub reply_to: Option<MessageId>,
    pub reply_all: bool,
    /// Forward this stored message, by row id.
    pub forward: Option<MessageId>,
}

/// What a draft will look like, without sending it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPreview {
    pub from: String,
    /// To, Cc and Bcc together, deduplicated — the SMTP envelope. Worth
    /// showing: it is the only place a blind recipient appears, and the one
    /// number a sender should check before committing.
    pub recipients: Vec<String>,
    pub subject: String,
    /// The body as it will be sent, quoted original included.
    pub body: String,
    /// The whole message, exactly as it would go out.
    ///
    /// Not what a compose window draws, but what a dry run is for: the
    /// threading headers, the encoding, and the absence of a Bcc line are all
    /// only visible here.
    pub rfc822: String,
}

/// A draft that was saved rather than sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedDraft {
    /// The folder it was appended to.
    pub folder: String,
    /// What was saved, exactly as a preview would show it.
    pub preview: DraftPreview,
}

/// What happened when a message went out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentSummary {
    pub message_id: String,
    pub recipients: Vec<String>,
    /// The folder the copy was filed in, when it was.
    pub filed_in: Option<String>,
    /// Set when the message was sent but the copy could not be filed.
    ///
    /// Separate from an error on purpose: by this point the server has
    /// accepted the message, and reporting a failure would invite the sender
    /// to send it twice.
    pub filing_error: Option<String>,
}

/// What a connection test found.
///
/// A structure rather than printed lines, so a settings dialog can show the
/// same answer the CLI does without parsing it back out of text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub email: String,
    /// `host:port (security)`, so the report says what it actually tried.
    pub imap_target: String,
    pub imap_ok: bool,
    pub imap_error: Option<String>,

    pub smtp_target: Option<String>,
    pub smtp_ok: bool,
    pub smtp_error: Option<String>,

    /// Extensions the client can exploit, and whether this server has them.
    pub extensions: Vec<(String, bool)>,
    pub folders: Vec<VerifiedFolder>,
    /// Where mail would go, resolved the way the app resolves it.
    pub sent_folder: Option<String>,
    pub archive_folder: Option<String>,
    pub trash_folder: Option<String>,
    pub total_messages: u64,
    /// Things that will work but not the way the user probably expects.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedFolder {
    pub name: String,
    pub special_use: Option<String>,
    pub messages: u32,
    pub excluded: bool,
}

/// Runs the operations that need a server.
pub struct Session {
    data_dir: PathBuf,
    /// Read a password from this environment variable instead of the keychain.
    ///
    /// For the dev server and CI, where a keychain read would raise a GUI
    /// prompt and block a test run on a dialog nobody is watching.
    password_env: Option<String>,
}

impl Session {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            password_env: None,
        }
    }

    pub fn with_password_env(mut self, var: Option<String>) -> Self {
        self.password_env = var;
        self
    }

    /// Opens a connection of this session's own. See the module note.
    fn open(&self) -> Result<(Store, Blobs)> {
        std::fs::create_dir_all(&self.data_dir)?;
        Ok((
            Store::open(self.data_dir.join("fuckmail.db"))?,
            Blobs::new(self.data_dir.join("blobs")),
        ))
    }

    /// Sends queued changes, then brings the mailbox up to date.
    ///
    /// In that order, and it matters: the sync pass that follows is what makes
    /// the changes visible, by observing the server rather than by guessing
    /// what the server did. One command, one coherent result.
    pub async fn sync_account(&self, email: &str) -> Result<SyncSummary> {
        let (store, blobs) = self.open()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let auth = provider_for(&account, self.password_env.as_deref())?;
        let config = core_proto::ImapConfig {
            host: account.imap_host.clone(),
            port: account.imap_port,
            security: account.imap_security.clone(),
            username: account.username.clone(),
        };

        let mut client = core_proto::ImapClient::connect(&config, auth.as_ref()).await?;

        let flushed =
            core_proto::flush_operations(&mut client, &store, account.id, now_utc()).await?;
        let report = core_proto::sync_account(&mut client, &store, &blobs, account.id).await?;
        client.logout().await.ok();

        Ok(SyncSummary {
            email: account.email,
            changes_sent: flushed.applied,
            changes_obsolete: flushed.obsolete,
            changes_refused: flushed.conflicted,
            changes_retryable: flushed.retryable,
            non_atomic_moves: flushed.non_atomic_moves,
            folders_synced: report.folders_synced,
            folders_skipped: report.folders_skipped,
            folders_excluded: report.folders_excluded,
            inserted: report.inserted,
            deduplicated: report.deduplicated,
            flag_updates: report.flag_updates,
            expunged: report.expunged,
            deleted: report.deleted,
            unparseable: report.unparseable,
            invalidated: report.invalidated,
        })
    }

    /// Connects to an account without changing anything, and reports what it
    /// found.
    ///
    /// The one call worth making before a first sync: it answers whether the
    /// credentials work, which extensions the server has, where mail will be
    /// filed, and how much a sync would fetch — all of which are cheaper to
    /// learn now than after downloading a mailbox.
    ///
    /// A failure to connect is part of the report rather than an error. "Your
    /// password is wrong" is exactly the answer the caller asked for, and a
    /// dialog that shows it next to the field is more use than one that throws.
    pub async fn verify(&self, email: &str) -> Result<VerifyReport> {
        let (store, _) = self.open()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let mut report = VerifyReport {
            email: account.email.clone(),
            imap_target: format!(
                "{}:{} ({})",
                account.imap_host,
                account.imap_port,
                account.imap_security.as_str()
            ),
            imap_ok: false,
            imap_error: None,
            smtp_target: account
                .smtp
                .as_ref()
                .map(|s| format!("{}:{} ({})", s.host, s.port, s.security.as_str())),
            smtp_ok: false,
            smtp_error: None,
            extensions: Vec::new(),
            folders: Vec::new(),
            sent_folder: None,
            archive_folder: None,
            trash_folder: None,
            total_messages: 0,
            warnings: Vec::new(),
        };

        let auth = match provider_for(&account, self.password_env.as_deref()) {
            Ok(auth) => auth,
            Err(err) => {
                report.imap_error = Some(err.to_string());
                return Ok(report);
            }
        };

        match self
            .probe_imap(&store, &account, auth.as_ref(), &mut report)
            .await
        {
            Ok(()) => report.imap_ok = true,
            Err(err) => report.imap_error = Some(err.to_string()),
        }

        if let Some(smtp) = &account.smtp {
            match core_smtp::verify(smtp, &account.username, auth.as_ref()).await {
                Ok(()) => report.smtp_ok = true,
                Err(err) => report.smtp_error = Some(err.to_string()),
            }
        }

        Ok(report)
    }

    async fn probe_imap(
        &self,
        store: &Store,
        account: &Account,
        auth: &dyn AuthProvider,
        report: &mut VerifyReport,
    ) -> Result<()> {
        let config = core_proto::ImapConfig {
            host: account.imap_host.clone(),
            port: account.imap_port,
            security: account.imap_security.clone(),
            username: account.username.clone(),
        };
        let mut client = core_proto::ImapClient::connect(&config, auth).await?;

        let ext = client.extensions().await?;
        report.extensions = vec![
            ("CONDSTORE".into(), ext.condstore),
            ("MOVE".into(), ext.r#move),
            ("UIDPLUS".into(), ext.uidplus),
            ("QRESYNC".into(), ext.qresync),
            ("IDLE".into(), ext.idle),
        ];
        if !ext.condstore {
            report
                .warnings
                .push("without CONDSTORE every sync rescans every folder".into());
        }
        if !ext.r#move && !ext.uidplus {
            report
                .warnings
                .push("without MOVE or UIDPLUS, archiving leaves a flagged copy behind".into());
        }

        let folders = client.folders().await?;
        let exclusions = store.folder_exclusions(account.id)?;
        let pairs: Vec<(&str, Option<&str>)> = folders
            .iter()
            .filter(|f| f.selectable)
            .map(|f| (f.name.as_str(), f.special_use.as_deref()))
            .collect();

        report.sent_folder = core_proto::client::find_sent(&folders).map(|f| f.name.clone());
        report.archive_folder =
            core_proto::client::find_archive(pairs.iter().copied()).map(str::to_string);
        report.trash_folder =
            core_proto::client::find_trash(pairs.iter().copied()).map(str::to_string);

        if report.archive_folder.is_none() {
            report
                .warnings
                .push("no Archive folder — archiving will fail until one is created".into());
        }

        for folder in folders.iter().filter(|f| f.selectable) {
            let excluded = core_store::folder_is_excluded(
                &exclusions,
                &folder.name,
                folder.special_use.as_deref(),
            );
            let state = client.examine(&folder.name).await?;
            if !excluded {
                report.total_messages += state.exists as u64;
            }
            if folder.special_use.as_deref() == Some("\\All") && !excluded && state.exists > 0 {
                report.warnings.push(format!(
                    "{} holds a copy of every message, so a sync fetches most mail twice; \
                     excluding \\All syncs it once",
                    folder.name
                ));
            }
            report.folders.push(VerifiedFolder {
                name: folder.name.clone(),
                special_use: folder.special_use.clone(),
                messages: state.exists,
                excluded,
            });
        }

        client.logout().await.ok();
        Ok(())
    }

    /// Builds a draft without sending it.
    ///
    /// The same construction the send path uses, so a preview cannot disagree
    /// with what would actually go out.
    pub fn preview(&self, email: &str, input: &DraftInput) -> Result<DraftPreview> {
        let (store, blobs) = self.open()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let draft = build_draft(&store, &blobs, &account, input)?;
        let built = draft
            .build()
            .map_err(|e| RpcError::Rejected(e.to_string()))?;

        Ok(DraftPreview {
            from: built.sender,
            recipients: built.recipients,
            subject: draft.subject.clone(),
            body: draft.body.clone(),
            rfc822: String::from_utf8_lossy(&built.rfc822).into_owned(),
        })
    }

    /// Saves a draft in the account's Drafts folder, for a person to read and send.
    ///
    /// Never sends. Built by the same construction `send` uses, so the draft
    /// cannot differ from what sending it would put on the wire, and appended
    /// rather than submitted — `APPEND` can only create a message, so there is
    /// nothing to hold in an undo window and nothing another client could lose.
    ///
    /// Refuses Bcc before anything is built. Blind recipients reach the
    /// envelope and never the message (brief §3.5), so the built message has no
    /// Bcc line — which means a draft saved to the server would silently lose
    /// them, and whoever sent it later, from any client, would send it without.
    pub async fn save_draft(&self, email: &str, input: &DraftInput) -> Result<SavedDraft> {
        if input.bcc.iter().any(|address| !address.trim().is_empty()) {
            return Err(RpcError::Rejected(
                "a draft cannot keep Bcc recipients: Bcc is never written into the message, \
                 so a draft saved to the server would lose them. Add them when sending."
                    .into(),
            ));
        }

        let (store, blobs) = self.open()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let draft = build_draft(&store, &blobs, &account, input)?;
        let built = draft
            .build()
            .map_err(|e| RpcError::Rejected(e.to_string()))?;

        let auth = provider_for(&account, self.password_env.as_deref())?;
        let folder = file_in_drafts(&account, auth.as_ref(), &built.rfc822).await?;

        Ok(SavedDraft {
            folder,
            preview: DraftPreview {
                from: built.sender.clone(),
                recipients: built.recipients.clone(),
                subject: draft.subject.clone(),
                body: draft.body.clone(),
                rfc822: String::from_utf8_lossy(&built.rfc822).into_owned(),
            },
        })
    }

    /// Sends a message, and files a copy in Sent unless told not to.
    ///
    /// `file_in_sent` is a parameter rather than a field on the draft because
    /// it is not part of the message — it is what to do afterwards, and the
    /// only sane default is "yes". A `bool` field that deserialised to `false`
    /// when omitted would silently stop filing anything.
    pub async fn send(
        &self,
        email: &str,
        input: &DraftInput,
        file_in_sent: bool,
    ) -> Result<SentSummary> {
        let (store, blobs) = self.open()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let smtp = account.smtp.clone().ok_or_else(|| {
            RpcError::Rejected(format!(
                "account {} has no SMTP endpoint; configure one with \
                 `fuckmail set-smtp --email {} --smtp-host <host> --smtp-port 587 \
                 --smtp-security starttls`",
                account.email, account.email
            ))
        })?;

        let draft = build_draft(&store, &blobs, &account, input)?;
        let built = draft
            .build()
            .map_err(|e| RpcError::Rejected(e.to_string()))?;

        let auth = provider_for(&account, self.password_env.as_deref())?;
        core_smtp::submit(&smtp, &account.username, auth.as_ref(), &built)
            .await
            .map_err(|e| RpcError::Network(e.to_string()))?;

        // Everything past this point is best-effort. The message has gone.
        let mut summary = SentSummary {
            message_id: built.message_id.clone(),
            recipients: built.recipients.clone(),
            filed_in: None,
            filing_error: None,
        };

        if file_in_sent {
            match file_copy_in_sent(&account, auth.as_ref(), &built.rfc822).await {
                Ok(folder) => summary.filed_in = Some(folder),
                Err(err) => {
                    tracing::warn!(%err, "sent, but could not file a copy in Sent");
                    summary.filing_error = Some(err.to_string());
                }
            }
        }
        Ok(summary)
    }

    /// Syncs every registered account, in order.
    ///
    /// One account failing does not stop the rest: a provider being down is
    /// not a reason to leave the others stale. The error is returned in place
    /// of that account's summary.
    pub async fn sync_all(&self) -> Vec<std::result::Result<SyncSummary, String>> {
        let emails: Vec<String> = match self.open() {
            Ok((store, _)) => match store.accounts() {
                Ok(accounts) => accounts.into_iter().map(|a| a.email).collect(),
                Err(err) => return vec![Err(err.to_string())],
            },
            Err(err) => return vec![Err(err.to_string())],
        };

        let mut results = Vec::new();
        for email in emails {
            results.push(
                self.sync_account(&email)
                    .await
                    .map_err(|err| format!("{email}: {err}")),
            );
        }
        results
    }
}

/// Assembles the draft, resolving a reply or forward from the store.
fn build_draft(
    store: &Store,
    blobs: &Blobs,
    account: &Account,
    input: &DraftInput,
) -> Result<Draft> {
    // The label doubles as the display name, unless it is just the address
    // again — in which case a `Name <addr>` header would only repeat itself.
    let from = if account.label == account.email {
        Mailbox::new(account.email.clone())
    } else {
        Mailbox::named(account.label.clone(), account.email.clone())
    };

    let mut draft = match (input.reply_to, input.forward) {
        (Some(id), _) => {
            let source = reply_source(store, blobs, account.id, id)?;
            let mode = if input.reply_all {
                ReplyMode::All
            } else {
                ReplyMode::Sender
            };
            Draft::reply(from, &source, mode)
        }
        (_, Some(id)) => Draft::forward(from, &reply_source(store, blobs, account.id, id)?),
        _ => Draft::new(from),
    };

    let parse =
        |address: &String| Mailbox::parse(address).map_err(|e| RpcError::Rejected(e.to_string()));
    // A compose form sends an empty string for an empty field. That is the
    // absence of a recipient, not a malformed one — the Bcc refusal in
    // `save_draft` already reads it that way, and so must every field here.
    let present = |address: &&String| !address.trim().is_empty();
    for address in input.to.iter().filter(present) {
        draft = draft.to(parse(address)?);
    }
    for address in input.cc.iter().filter(present) {
        draft = draft.cc(parse(address)?);
    }
    for address in input.bcc.iter().filter(present) {
        draft = draft.bcc(parse(address)?);
    }
    if !input.subject.is_empty() {
        draft = draft.subject(input.subject.clone());
    }

    // What was typed goes above whatever the reply or forward put there, which
    // is where a reply is read from.
    draft.body = format!("{}{}", input.body, draft.body);
    Ok(draft)
}

/// Reads a stored message back into the parts a reply needs.
fn reply_source(
    store: &Store,
    blobs: &Blobs,
    account: AccountId,
    id: MessageId,
) -> Result<ReplySource> {
    let stored = store
        .message_by_id(account, id)?
        .ok_or(RpcError::UnknownMessage(id))?;
    let path = stored.body_path.as_deref().ok_or_else(|| {
        RpcError::Rejected(format!(
            "message {id} was stored without its body, so it cannot be quoted"
        ))
    })?;
    let raw = blobs
        .get(path)
        .map_err(|e| RpcError::Rejected(format!("reading the stored body of {id}: {e}")))?;

    ReplySource::from_rfc822(&raw)
        .ok_or_else(|| RpcError::Rejected(format!("the stored body of {id} is not a message")))
}

/// Appends a draft, returning the folder it went to.
async fn file_in_drafts(account: &Account, auth: &dyn AuthProvider, raw: &[u8]) -> Result<String> {
    let config = core_proto::ImapConfig {
        host: account.imap_host.clone(),
        port: account.imap_port,
        security: account.imap_security.clone(),
        username: account.username.clone(),
    };

    let mut client = core_proto::ImapClient::connect(&config, auth).await?;
    let folders = client.folders().await?;
    let drafts = core_proto::client::find_drafts(&folders)
        .map(|folder| folder.name.clone())
        .ok_or_else(|| {
            RpcError::Rejected(
                "the server declares no Drafts folder and none of the usual names exist".into(),
            )
        })?;

    // \Draft so every client shows it as one; \Seen because nobody needs
    // telling about a draft that was just written for them.
    client.append(&drafts, &["\\Draft", "\\Seen"], raw).await?;
    client.logout().await.ok();
    Ok(drafts)
}

/// Files the sent copy, returning the folder it went to.
async fn file_copy_in_sent(
    account: &Account,
    auth: &dyn AuthProvider,
    raw: &[u8],
) -> Result<String> {
    let config = core_proto::ImapConfig {
        host: account.imap_host.clone(),
        port: account.imap_port,
        security: account.imap_security.clone(),
        username: account.username.clone(),
    };

    let mut client = core_proto::ImapClient::connect(&config, auth).await?;
    let folders = client.folders().await?;
    let sent = core_proto::client::find_sent(&folders)
        .map(|folder| folder.name.clone())
        .ok_or_else(|| {
            RpcError::Rejected(
                "the server declares no Sent folder and none of the usual names exist".into(),
            )
        })?;

    // \Seen because the sender has, by definition, read it; without it every
    // client shows Sent as full of unread mail.
    client.append(&sent, &["\\Seen"], raw).await?;
    client.logout().await.ok();
    Ok(sent)
}

/// Builds the auth provider an account is configured for.
///
/// The account row records which provider and which grant, because they are
/// not interchangeable — Microsoft takes the device flow and Gmail cannot have
/// it. The credential is always in the keychain.
pub fn provider_for(
    account: &Account,
    password_env: Option<&str>,
) -> Result<Box<dyn AuthProvider>> {
    if let Some(var) = password_env {
        return Ok(Box::new(EnvPassword::new(var)));
    }

    match account.auth_method.as_str() {
        "oauth2" if account.oauth_provider.as_deref() == Some("google") => {
            Ok(Box::new(OAuth2Loopback::new(google_config(account)?)))
        }
        "oauth2" => Ok(Box::new(OAuth2Device::new(microsoft_config(account)?))),
        _ => Ok(Box::new(KeychainPassword::new(&account.email))),
    }
}

/// Where Google's client secret lives.
///
/// The keychain, like every other credential. RFC 8252 is clear it is not
/// really secret for an installed app, but "not secret" is not a reason to put
/// it in a config file, and it has to persist because Google wants it on the
/// refresh grant as well as at login.
pub fn google_client_secret(email: &str) -> KeychainPassword {
    KeychainPassword::new(format!("{email} (google oauth client secret)"))
}

pub fn google_config(account: &Account) -> Result<LoopbackConfig> {
    let client_id = account.oauth_client_id.as_deref().ok_or_else(|| {
        RpcError::Rejected(format!("account {} has no OAuth client id", account.email))
    })?;

    // Absent is not an error here. A client configured without one still works
    // if Google accepts the exchange on PKCE alone, and failing at the point
    // it is actually needed says more than refusing up front.
    let secret = google_client_secret(&account.email).peek()?;

    Ok(LoopbackConfig::google(&account.username, client_id, secret))
}

pub fn microsoft_config(account: &Account) -> Result<OAuth2Config> {
    let client_id = account.oauth_client_id.as_deref().ok_or_else(|| {
        RpcError::Rejected(format!("account {} has no OAuth client id", account.email))
    })?;
    let tenant = account.oauth_tenant.as_deref().unwrap_or("common");
    Ok(OAuth2Config::microsoft(
        &account.username,
        client_id,
        tenant,
    ))
}

fn now_utc() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
