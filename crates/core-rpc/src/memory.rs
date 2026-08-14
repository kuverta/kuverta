//! What the assistant remembers, and how it travels between devices.
//!
//! A note is one short fact about an account the assistant would otherwise
//! look up again with each question: who the tax adviser is, which folder
//! invoices go to, that the person signs letters with their first name. The
//! notes go into the system prompt, so a question they answer needs no search
//! at all — and they cost their length on every question, which is why they
//! are kept few and short.
//!
//! ## Who may write one
//!
//! The model, when the person told it something or agreed to it, and the
//! person, in the settings. Never a message: mail is written by strangers,
//! and a note is read on every later question, so an instruction planted in
//! one would outlive the mail it came in. The model is told so; and every
//! note it keeps is shown in the chat with a way to forget it, and listed in
//! the settings.
//!
//! ## Syncing
//!
//! Through the account's own mail server, in a folder of its own that the
//! mail sync leaves alone ([`core_proto::MEMORY_FOLDER`]). Each upload is one
//! message holding every note, signed with the account's own OpenPGP key and
//! encrypted to it alone: the server sees a message to and from the account,
//! and nothing of what it says. A snapshot is only believed if it decrypts
//! *and* carries a valid signature by a key whose secret half is here —
//! anyone can encrypt to a public key, and a note is read by the model.
//!
//! Notes merge one by one, the later change winning, so two devices that
//! both changed something both keep their changes. Snapshots that have been
//! merged and replaced go to the Trash: this client never expunges.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use core_store::model::AccountId;

use crate::assistant::{AssistantEvent, ToolOutcome};
use crate::session::Session;
use crate::{Core, Result, RpcError};

/// How long one note may be, in characters.
pub const MAX_NOTE_CHARS: usize = 300;
/// How many notes an account may have: they are sent with every question.
pub const MAX_NOTES: usize = 60;

/// A note as the settings and the model see it.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryView {
    pub id: i64,
    pub text: String,
    pub updated_at: i64,
}

/// Whether the notes sync, and whether they can.
#[derive(Debug, Clone, Serialize)]
pub struct MemorySyncView {
    pub enabled: bool,
    /// Why they cannot, when they cannot: no key, or no passphrase for it.
    pub unavailable: Option<String>,
    /// The key they are encrypted to, when there is one.
    pub key_id: Option<String>,
    pub pending: bool,
    pub synced_at: Option<i64>,
    pub last_error: Option<String>,
}

/// What a snapshot on the server holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    notes: Vec<SnapshotNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SnapshotNote {
    key: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    forgotten: bool,
    updated_at: i64,
}

const BEGIN: &str = "-----BEGIN KUVERTA MEMORY-----";
const END: &str = "-----END KUVERTA MEMORY-----";
const SUBJECT: &str = "kuverta assistant memory";

/// The text of a snapshot message: a line for whoever opens it in another
/// program, and the notes as base64 between markers — no line of JSON for a
/// mail program to wrap or re-encode.
fn snapshot_body(snapshot: &Snapshot) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(snapshot).unwrap_or_default());
    let mut body = String::from(
        "What kuverta's assistant remembers about this account, for kuverta on your \
         other devices. Leave it here; kuverta replaces it when the notes change.\n\n",
    );
    body.push_str(BEGIN);
    body.push('\n');
    for chunk in encoded.as_bytes().chunks(76) {
        body.push_str(&String::from_utf8_lossy(chunk));
        body.push('\n');
    }
    body.push_str(END);
    body.push('\n');
    body
}

fn parse_snapshot(text: &str) -> Option<Snapshot> {
    use base64::Engine as _;
    let start = text.find(BEGIN)? + BEGIN.len();
    let end = start + text[start..].find(END)?;
    let encoded: String = text[start..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A key the same on every device, made here.
fn new_key() -> String {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("the system has no randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A note as given, tidied, or why it cannot be one.
fn note_text(text: &str) -> Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(RpcError::Rejected("a note needs some text".into()));
    }
    if text.chars().count() > MAX_NOTE_CHARS {
        return Err(RpcError::Rejected(format!(
            "a note is at most {MAX_NOTE_CHARS} characters; keep one fact per note"
        )));
    }
    Ok(text)
}

impl Core {
    pub fn memories(&self, account: AccountId) -> Result<Vec<MemoryView>> {
        Ok(self
            .store
            .memories(account, false)?
            .into_iter()
            .map(|memory| MemoryView {
                id: memory.id,
                text: memory.text,
                updated_at: memory.updated_at,
            })
            .collect())
    }

    pub fn add_memory(&self, account: AccountId, text: &str) -> Result<MemoryView> {
        let text = note_text(text)?;
        let notes = self.store.memories(account, false)?;
        if let Some(same) = notes
            .iter()
            .find(|note| note.text.eq_ignore_ascii_case(&text))
        {
            return Ok(MemoryView {
                id: same.id,
                text: same.text.clone(),
                updated_at: same.updated_at,
            });
        }
        if notes.len() >= MAX_NOTES {
            return Err(RpcError::Rejected(format!(
                "the memory is full ({MAX_NOTES} notes); forget one first"
            )));
        }
        let id = self.store.add_memory(account, &new_key(), &text)?;
        self.memory_view(account, id)
    }

    pub fn update_memory(&self, account: AccountId, id: i64, text: &str) -> Result<MemoryView> {
        let text = note_text(text)?;
        self.own_memory(account, id)?;
        self.store.update_memory(id, &text)?;
        self.memory_view(account, id)
    }

    pub fn forget_memory(&self, account: AccountId, id: i64) -> Result<String> {
        let memory = self.own_memory(account, id)?;
        self.store.forget_memory(id)?;
        Ok(memory.text)
    }

    pub fn memory_sync_status(&self, account: AccountId) -> Result<MemorySyncView> {
        let sync = self.store.memory_sync(account)?;
        let email = self.account_email(account)?;
        let (key_id, unavailable) = match self.keyring.as_ref() {
            Some(keyring) => match own_key(keyring, &email) {
                Ok(key) => (Some(key.key_id), None),
                Err(why) => (None, Some(why)),
            },
            None => (None, Some("there is no keyring".to_string())),
        };
        Ok(MemorySyncView {
            enabled: sync.enabled,
            unavailable,
            key_id,
            pending: sync.pending,
            synced_at: sync.synced_at,
            last_error: sync.last_error,
        })
    }

    /// Turns syncing on or off. On needs a key to encrypt to.
    pub fn set_memory_sync(&self, account: AccountId, enabled: bool) -> Result<()> {
        if enabled {
            if let Some(why) = self.memory_sync_status(account)?.unavailable {
                return Err(RpcError::Rejected(why));
            }
        }
        Ok(self.store.set_memory_sync(account, enabled)?)
    }

    fn own_memory(&self, account: AccountId, id: i64) -> Result<core_store::StoredMemory> {
        self.store
            .memory(id)?
            .filter(|memory| memory.account_id == account && !memory.forgotten)
            .ok_or_else(|| RpcError::Rejected(format!("there is no note {id} on this account")))
    }

    fn memory_view(&self, account: AccountId, id: i64) -> Result<MemoryView> {
        let memory = self.own_memory(account, id)?;
        Ok(MemoryView {
            id: memory.id,
            text: memory.text,
            updated_at: memory.updated_at,
        })
    }

    /// The notes, as the system prompt carries them. Empty without any.
    pub(crate) fn memory_prompt(&self, account: AccountId) -> String {
        let notes = match self.memories(account) {
            Ok(notes) => notes,
            Err(err) => {
                tracing::warn!(%err, "the assistant's notes could not be read");
                return String::new();
            }
        };
        if notes.is_empty() {
            return "You have no notes on this account yet.".into();
        }
        let mut prompt = String::from(
            "Your notes on this account, kept from earlier conversations (id: note). Use them \
             instead of searching again when they answer the question:\n",
        );
        for note in notes {
            prompt.push_str(&format!("{}: {}\n", note.id, note.text));
        }
        prompt
    }

    /// The `remember` and `forget` tools.
    pub(crate) fn memory_tool(
        &self,
        account: AccountId,
        name: &str,
        args: &Value,
    ) -> std::result::Result<ToolOutcome, String> {
        let e = |err: RpcError| err.to_string();
        let id = || {
            args.get("id")
                .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
        };
        match name {
            "remember" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("text is missing")?;
                let note = match args
                    .get("replaces")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
                {
                    Some(old) => self.update_memory(account, old, text).map_err(e)?,
                    None => self.add_memory(account, text).map_err(e)?,
                };
                Ok(ToolOutcome::ok(
                    json!({ "remembered": note.id }),
                    Some(AssistantEvent::Remembered {
                        id: note.id,
                        text: note.text,
                    }),
                ))
            }
            "forget" => {
                let id = id().ok_or("id is missing")?;
                let text = self.forget_memory(account, id).map_err(e)?;
                Ok(ToolOutcome::ok(
                    json!({ "forgotten": id }),
                    Some(AssistantEvent::Looked {
                        what: format!("forgot “{text}”"),
                    }),
                ))
            }
            other => Err(format!("there is no tool {other}")),
        }
    }
}

/// The account's own key, ready to sign and to decrypt with, or why not.
fn own_key(
    keyring: &core_pgp::Keyring,
    email: &str,
) -> std::result::Result<core_pgp::KeyView, String> {
    let key = keyring
        .key_for_email(email)
        .map_err(|err| err.to_string())?
        .filter(|key| key.has_secret && !key.revoked && !key.expired)
        .ok_or_else(|| {
            format!(
                "{email} has no OpenPGP key of its own: make or import one under Settings → \
                 Encryption, and use the same key on every device"
            )
        })?;
    if !key.has_passphrase {
        return Err(format!(
            "the key for {email} has no stored passphrase, so it can neither sign nor decrypt \
             without asking: store it under Settings → Encryption"
        ));
    }
    if !key.can_encrypt || !key.can_sign {
        return Err(format!("the key for {email} cannot both sign and encrypt"));
    }
    Ok(key)
}

/// How a sync of the notes went.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemorySyncReport {
    /// Notes changed here by what came from the server.
    pub received: usize,
    /// Whether a new snapshot was uploaded.
    pub uploaded: bool,
    /// Messages in the folder that were not believed and left alone.
    pub ignored: usize,
}

impl Session {
    /// Brings the notes of one account and those on its server together:
    /// downloads and merges every snapshot, uploads one with everything if
    /// that differs from what is there, and moves the snapshots it replaced
    /// to the Trash. Does nothing when syncing is off.
    pub async fn sync_memory(&self, email: &str) -> Result<MemorySyncReport> {
        let (store, _) = self.open_store()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;
        if !store.memory_sync(account.id)?.enabled {
            return Ok(MemorySyncReport::default());
        }
        let outcome = self.sync_memory_inner(&store, &account).await;
        store.memory_synced(
            account.id,
            outcome.as_ref().err().map(|err| err.to_string()).as_deref(),
        )?;
        outcome
    }

    async fn sync_memory_inner(
        &self,
        store: &core_store::Store,
        account: &core_store::model::Account,
    ) -> Result<MemorySyncReport> {
        let keyring = self.keyring();
        let key = own_key(&keyring, &account.email).map_err(RpcError::Rejected)?;
        // Whose signature a snapshot may carry: a key whose secret half is
        // here, which is to say the person's own.
        let own: Vec<String> = keyring
            .keys()?
            .into_iter()
            .filter(|key| key.has_secret)
            .map(|key| key.fingerprint.to_ascii_uppercase())
            .collect();

        let mut client = self.connect(account).await?;
        let folders = client.folders().await?;
        let folder = match folders
            .iter()
            .find(|folder| core_proto::is_memory_folder(&folder.name))
        {
            Some(folder) => folder.name.clone(),
            None => {
                client
                    .create_folder(core_proto::MEMORY_FOLDER, None)
                    .await?;
                core_proto::MEMORY_FOLDER.to_string()
            }
        };

        client.select(&folder).await?;
        let mut uids: Vec<u32> = client.all_uids().await?.into_iter().collect();
        uids.sort_unstable();
        let mut report = MemorySyncReport::default();
        let mut believed: Vec<(u32, Snapshot)> = Vec::new();
        for message in client.fetch_uids(&uids).await? {
            let opened = core_pgp::open_message(&message.raw, Some(&keyring));
            let signed_by_us = opened.security.as_ref().is_some_and(|security| {
                security.decrypted
                    && security.signature.as_ref().is_some_and(|signature| {
                        signature.state == core_pgp::SignatureState::Valid
                            && signature
                                .fingerprint
                                .as_ref()
                                .is_some_and(|f| own.contains(&f.to_ascii_uppercase()))
                    })
            });
            match opened
                .body_text
                .as_deref()
                .filter(|_| signed_by_us)
                .and_then(parse_snapshot)
            {
                Some(snapshot) => believed.push((message.uid, snapshot)),
                None => {
                    tracing::warn!(uid = message.uid, "a message in the memory folder is not a snapshot signed by this account's key; left alone");
                    report.ignored += 1;
                }
            }
        }

        for (_, snapshot) in &believed {
            for note in &snapshot.notes {
                if store.merge_memory(
                    account.id,
                    &note.key,
                    &note.text,
                    note.forgotten,
                    note.updated_at,
                )? {
                    report.received += 1;
                }
            }
        }

        let ours = Snapshot {
            version: 1,
            notes: store
                .memories(account.id, true)?
                .into_iter()
                .map(|memory| SnapshotNote {
                    key: memory.key,
                    text: memory.text,
                    forgotten: memory.forgotten,
                    updated_at: memory.updated_at,
                })
                .collect(),
        };
        let current = believed.len() == 1 && same_notes(&believed[0].1, &ours);
        if current || (believed.is_empty() && ours.notes.is_empty()) {
            client.logout().await.ok();
            return Ok(report);
        }

        let draft = core_smtp::Draft {
            from: core_smtp::Mailbox::new(account.email.clone()),
            to: vec![core_smtp::Mailbox::new(account.email.clone())],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: SUBJECT.into(),
            body: snapshot_body(&ours),
            in_reply_to: None,
            references: Vec::new(),
            attachments: Vec::new(),
        };
        let built = draft
            .build()
            .map_err(|err| RpcError::Rejected(err.to_string()))?;
        let sealed = core_pgp::protect(
            &keyring,
            &built.rfc822,
            &built.sender,
            &[],
            &[],
            core_pgp::Protection {
                sign: true,
                encrypt: true,
            },
        )?;
        client.append(&folder, &["\\Seen"], &sealed).await?;
        report.uploaded = true;
        tracing::info!(account = %account.email, key = %key.key_id, notes = ours.notes.len(), "uploaded the assistant's notes");

        // What the new snapshot replaces goes to the Trash, where the server
        // empties it in its own time. Without a Trash it stays: harmless, as
        // every snapshot merges into the same notes.
        if let Some(trash) = core_proto::client::find_trash(
            folders
                .iter()
                .map(|folder| (folder.name.as_str(), folder.special_use.as_deref())),
        ) {
            for (uid, _) in &believed {
                if let Err(err) = client.uid_move(*uid, trash).await {
                    tracing::warn!(uid, %err, "a replaced memory snapshot could not be moved to the Trash");
                }
            }
        }
        client.logout().await.ok();
        Ok(report)
    }
}

/// Whether two snapshots hold the same notes, in whatever order.
fn same_notes(a: &Snapshot, b: &Snapshot) -> bool {
    let sorted = |snapshot: &Snapshot| {
        let mut notes = snapshot.notes.clone();
        notes.sort_by(|x, y| x.key.cmp(&y.key));
        notes
    };
    sorted(a) == sorted(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_survives_its_trip_through_a_message_body() {
        let snapshot = Snapshot {
            version: 1,
            notes: vec![
                SnapshotNote {
                    key: "a1".into(),
                    text: "Die Steuerberaterin ist Erika Mustermann, erika@example.com — \"quoted\" and long ".repeat(3),
                    forgotten: false,
                    updated_at: 1_790_000_000,
                },
                SnapshotNote {
                    key: "b2".into(),
                    text: String::new(),
                    forgotten: true,
                    updated_at: 1_790_000_100,
                },
            ],
        };
        let body = snapshot_body(&snapshot);
        assert!(body.lines().all(|line| line.len() <= 160));
        // As a mail program might hand it back: CRLF, a trailing line added.
        let mangled = body.replace('\n', "\r\n") + "\r\n-- \r\nsent from somewhere\r\n";
        assert_eq!(parse_snapshot(&mangled), Some(snapshot));
        assert_eq!(parse_snapshot("no markers here"), None);
    }

    #[test]
    fn notes_are_tidied_and_kept_short() {
        assert_eq!(
            note_text("  invoices go\n to  Rechnungen ").unwrap(),
            "invoices go to Rechnungen"
        );
        assert!(note_text("   ").is_err());
        assert!(note_text(&"x".repeat(MAX_NOTE_CHARS + 1)).is_err());
    }

    #[test]
    fn the_order_of_notes_does_not_make_a_snapshot_different() {
        let note = |key: &str| SnapshotNote {
            key: key.into(),
            text: key.into(),
            forgotten: false,
            updated_at: 1,
        };
        let a = Snapshot {
            version: 1,
            notes: vec![note("a"), note("b")],
        };
        let b = Snapshot {
            version: 1,
            notes: vec![note("b"), note("a")],
        };
        assert!(same_notes(&a, &b));
    }
}
