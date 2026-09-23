//! The triage surface.
//!
//! Every command here is a thin wrapper over [`core_rpc::Core`]. That is the
//! whole design: the shell decides what to draw and which key does what, and
//! knows nothing else. Anything that looks like a decision — which copy of a
//! message to archive, what to show when a subject is missing, whether a
//! change can still be undone — is answered in the core, where it can be
//! tested without a window.
//!
//! The list fetches windows rather than the mailbox. The spike that retired
//! this stack measured the Rust/JS bridge at roughly 78 MiB/s of JSON, which
//! is plenty for a screenful and nowhere near enough for 200k rows; see
//! docs/spike-tauri-list.md.
//!
//! Reads and queued changes go through the store and never touch a network,
//! so the window never blocks on one. Syncing does, and runs on its own
//! thread with its own connection — see [`sync`].

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod files;
mod logging;

use std::path::PathBuf;
use std::sync::Mutex;

use core_rpc::{AccountView, Core, MessageDetail, MessagePage, MessageRow, QueuedChange};
use core_store::model::ListFilter;
use tauri::State;

/// `rusqlite`'s connection is `Send` but not `Sync`, so the one store this app
/// owns is behind a lock. At personal-mailbox scale every query here is well
/// under a frame, so there is nothing to gain from a pool.
struct App {
    core: Mutex<Core>,
    /// A sync opens a connection of its own rather than borrowing the one
    /// behind the lock: it runs for as long as the network takes, and a lock
    /// held across that is a frozen window. SQLite is in WAL mode, so the list
    /// goes on being served from the other connection while this one writes.
    data_dir: PathBuf,
    logs: logging::Logs,
}

/// Tauri needs a serialisable error; `RpcError` is not one.
///
/// The string is the error's own message, which is written to be read by a
/// person — so this loses the type and keeps the part the UI shows.
fn fail(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[tauri::command]
fn accounts(app: State<'_, App>) -> Result<Vec<AccountView>, String> {
    app.core.lock().unwrap().accounts().map_err(fail)
}

/// How the list is narrowed and ordered, as the window sends it: one value,
/// because they are asked and answered together.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    category: Option<String>,
    #[serde(default)]
    unread_only: bool,
    folder: Option<i64>,
    #[serde(default)]
    oldest_first: bool,
    /// A smart mailbox, by id: its rules narrow the list like any filter.
    smart: Option<i64>,
}

#[tauri::command]
fn messages(
    app: State<'_, App>,
    account: i64,
    offset: usize,
    limit: usize,
    filter: ListQuery,
) -> Result<MessagePage, String> {
    let core = app.core.lock().unwrap();
    let smart = match filter.smart {
        Some(id) => Some(core.smart_query(id).map_err(fail)?),
        None => None,
    };
    let filter = ListFilter {
        category: filter.category,
        unread_only: filter.unread_only,
        folder: filter.folder,
        oldest_first: filter.oldest_first,
        // All mail — no folder, no smart mailbox — is mail that arrived: your
        // own replies and what you threw away are in their folders, and in
        // search, but not in the list you read top to bottom. A smart mailbox
        // says for itself what it wants, and a folder listing is of that
        // folder whatever is in it.
        incoming_only: filter.folder.is_none() && smart.is_none(),
        smart,
    };
    core.messages(account, offset, limit, &filter).map_err(fail)
}

#[tauri::command]
fn message(app: State<'_, App>, account: i64, id: i64) -> Result<MessageDetail, String> {
    app.core.lock().unwrap().message(account, id).map_err(fail)
}

/// An attachment's bytes, for the window to show: a binary response, since
/// an attachment is often megabytes.
#[tauri::command]
fn attachment(
    app: State<'_, App>,
    account: i64,
    id: i64,
    index: usize,
) -> Result<tauri::ipc::Response, String> {
    let found = app
        .core
        .lock()
        .unwrap()
        .attachment(account, id, index)
        .map_err(fail)?;
    Ok(tauri::ipc::Response::new(found.bytes))
}

/// Saves an attachment to Downloads, shows it there, and says where.
#[tauri::command]
fn save_attachment(
    app: State<'_, App>,
    account: i64,
    id: i64,
    index: usize,
) -> Result<String, String> {
    let found = app
        .core
        .lock()
        .unwrap()
        .attachment(account, id, index)
        .map_err(fail)?;
    let dir = files::downloads().ok_or("there is no Downloads folder to save to")?;
    let path = files::save_new(&dir, &found.view.name, &found.bytes)?;
    tracing::info!(id, index, "saved an attachment");
    let _ = logging::reveal(&path);
    Ok(path.display().to_string())
}

/// Opens an attachment in the program the system has for it. Not one that
/// could run something: those are saved, and opening them is the person's
/// own deliberate step.
#[tauri::command]
fn open_attachment(app: State<'_, App>, account: i64, id: i64, index: usize) -> Result<(), String> {
    let found = app
        .core
        .lock()
        .unwrap()
        .attachment(account, id, index)
        .map_err(fail)?;
    if found.view.risky {
        return Err(format!(
            "{} could run something when opened; save it instead if you trust it",
            found.view.name
        ));
    }
    let path = files::copy_to_open(&found.view.name, &found.bytes)?;
    tracing::info!(id, index, "opened an attachment");
    files::open(&path)
}

#[tauri::command]
fn search(
    app: State<'_, App>,
    account: i64,
    query: String,
    limit: usize,
) -> Result<Vec<MessageRow>, String> {
    app.core
        .lock()
        .unwrap()
        .search(account, &query, limit)
        .map_err(fail)
}

#[tauri::command]
fn folders(app: State<'_, App>, account: i64) -> Result<Vec<core_rpc::FolderView>, String> {
    app.core.lock().unwrap().folders(account).map_err(fail)
}

#[tauri::command]
fn category_counts(
    app: State<'_, App>,
    account: i64,
    folder: Option<i64>,
) -> Result<Vec<(String, usize)>, String> {
    app.core
        .lock()
        .unwrap()
        .category_counts(account, folder)
        .map_err(fail)
}

#[tauri::command]
fn move_to(app: State<'_, App>, account: i64, id: i64, target: String) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .move_to(account, id, &target, core_rpc::DEFAULT_UNDO_WINDOW_SECS)
        .map_err(fail)
}

#[tauri::command]
fn set_read(app: State<'_, App>, account: i64, id: i64, read: bool) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .set_read(account, id, read, core_rpc::DEFAULT_UNDO_WINDOW_SECS)
        .map_err(fail)
}

/// Files a message under a category. Local only — never sent to a server.
#[tauri::command]
fn set_category(
    app: State<'_, App>,
    account: i64,
    id: i64,
    category: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_category(account, id, &category)
        .map_err(fail)
}

// -- postal addresses -------------------------------------------------------
//
// The reads are async and the store is behind a `Mutex`, so each one takes the
// lock only long enough to build a session — the client, the selector and the
// classifier — and releases it before touching the network. A guard held
// across an `await` is not `Send`, and would deadlock the window besides.

#[tauri::command]
fn paper_mailboxes(app: State<'_, App>) -> Result<Vec<core_rpc::PaperMailboxView>, String> {
    app.core.lock().unwrap().paper_mailboxes().map_err(fail)
}

#[tauri::command]
fn save_paper_mailbox(
    app: State<'_, App>,
    input: core_rpc::PaperMailboxInput,
) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .save_paper_mailbox(&input)
        .map_err(fail)
}

#[tauri::command]
fn delete_paper_mailbox(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .delete_paper_mailbox(id)
        .map_err(fail)
}

#[tauri::command]
fn set_paper_token(app: State<'_, App>, id: i64, token: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_paper_token(id, &token)
        .map_err(fail)
}

fn paper_session(app: &State<'_, App>, id: i64) -> Result<core_rpc::PaperSession, String> {
    app.core.lock().unwrap().paper_session(id).map_err(fail)
}

/// A window of a postbox: newest first by the letter's date, or by when it was
/// scanned (`order: "added"`), and within one category when one is given.
#[tauri::command]
async fn paper_documents(
    app: State<'_, App>,
    id: i64,
    offset: usize,
    limit: usize,
    query: Option<String>,
    order: Option<String>,
    category: Option<String>,
) -> Result<core_rpc::PaperPage, String> {
    let session = paper_session(&app, id)?;
    session
        .documents_by(
            offset,
            limit,
            query.as_deref(),
            order.as_deref(),
            category.as_deref(),
        )
        .await
        .map_err(fail)
}

#[tauri::command]
fn set_paper_read(
    app: State<'_, App>,
    id: i64,
    document_id: i64,
    read: bool,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_paper_read(id, document_id, read)
        .map_err(fail)
}

#[tauri::command]
async fn paper_unread_count(app: State<'_, App>, id: i64) -> Result<usize, String> {
    let session = paper_session(&app, id)?;
    session.unread_count().await.map_err(fail)
}

/// Letters per category, as `[category, count]` pairs like `category_counts`.
#[tauri::command]
async fn paper_category_counts(
    app: State<'_, App>,
    id: i64,
) -> Result<Vec<(String, usize)>, String> {
    let session = paper_session(&app, id)?;
    session.category_counts().await.map_err(fail)
}

/// Has a vision model read a letter's scan, keeps what it read, and returns
/// `[model, text]`.
///
/// The model and provider are the ones chosen for reading scans in settings.
/// Reading takes seconds a page, and all of it happens with no store lock held:
/// the lock is taken only to choose the model and to keep the result.
#[tauri::command]
async fn paper_transcribe(
    app: State<'_, App>,
    id: i64,
    document_id: i64,
) -> Result<(String, String), String> {
    let session = paper_session(&app, id)?;
    let choice = ai_choice(&app, core_rpc::Task::Vision)?;
    let text = session
        .transcribe(document_id, &choice.provider, &choice.model)
        .await
        .map_err(fail)?;
    app.core
        .lock()
        .unwrap()
        .save_paper_transcript(id, document_id, &choice.model, &text)
        .map_err(fail)?;
    Ok((choice.model, text))
}

#[tauri::command]
async fn paper_document(
    app: State<'_, App>,
    id: i64,
    document_id: i64,
) -> Result<core_rpc::PaperDetail, String> {
    let session = paper_session(&app, id)?;
    session.document(document_id).await.map_err(fail)
}

/// A letter's file — the scan itself — as raw bytes.
///
/// A binary response rather than JSON: a scan is megabytes, and as a JSON
/// array of numbers it would be several times that across the bridge. Through
/// the app at all because the window's content policy keeps it from reaching
/// Paperless, and the download needs the token the window never sees.
#[tauri::command]
async fn paper_file(
    app: State<'_, App>,
    id: i64,
    document_id: i64,
) -> Result<tauri::ipc::Response, String> {
    let session = paper_session(&app, id)?;
    let download = session.download(document_id).await.map_err(fail)?;
    Ok(tauri::ipc::Response::new(download.bytes))
}

#[tauri::command]
async fn paper_check(app: State<'_, App>, id: i64) -> Result<core_rpc::PaperReport, String> {
    let session = paper_session(&app, id)?;
    session.check().await.map_err(fail)
}

/// Files a piece of post by hand.
///
/// Who sent it is Paperless's to say, so the document is fetched with no store
/// lock held, and the lock is taken again only to record — the same shape as
/// every other paper command, for the same reason.
#[tauri::command]
async fn file_post(
    app: State<'_, App>,
    id: i64,
    document_id: i64,
    category: String,
) -> Result<(), String> {
    let session = paper_session(&app, id)?;
    let detail = session.document(document_id).await.map_err(fail)?;
    app.core
        .lock()
        .unwrap()
        .record_paper_correction(
            id,
            document_id,
            detail.sender.as_deref(),
            detail.row.category.as_deref(),
            &category,
        )
        .map_err(fail)
}

// -- models -------------------------------------------------------------------
//
// Where models run and which each job uses. Listing and trying models go over
// the network, so, as with post, the lock is held only to build a client.

fn ai_choice(app: &State<'_, App>, task: core_rpc::Task) -> Result<core_rpc::AiChoice, String> {
    app.core.lock().unwrap().ai_for(task).map_err(fail)
}

fn ai_client(app: &State<'_, App>, provider_id: i64) -> Result<core_rpc::ModelProvider, String> {
    app.core
        .lock()
        .unwrap()
        .ai_provider_client(provider_id)
        .map_err(fail)
}

#[tauri::command]
fn ai_providers(app: State<'_, App>) -> Result<Vec<core_rpc::AiProviderView>, String> {
    app.core.lock().unwrap().ai_providers().map_err(fail)
}

#[tauri::command]
fn save_ai_provider(app: State<'_, App>, input: core_rpc::AiProviderInput) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .save_ai_provider(&input)
        .map_err(fail)
}

#[tauri::command]
fn delete_ai_provider(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .delete_ai_provider(id)
        .map_err(fail)
}

/// Stores a hosted service's API key in the keychain. One way only, like
/// passwords: nothing here hands a key back to the window.
#[tauri::command]
fn set_ai_key(app: State<'_, App>, id: i64, key: String) -> Result<(), String> {
    app.core.lock().unwrap().set_ai_key(id, &key).map_err(fail)
}

#[tauri::command]
fn ai_tasks(app: State<'_, App>) -> Result<Vec<core_rpc::AiTaskView>, String> {
    app.core.lock().unwrap().ai_tasks().map_err(fail)
}

#[tauri::command]
fn set_ai_task(
    app: State<'_, App>,
    task: core_rpc::Task,
    provider_id: i64,
    model: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_ai_task(task, provider_id, &model)
        .map_err(fail)
}

/// The models a provider offers, with what each can do where it says.
#[tauri::command]
async fn ai_models(
    app: State<'_, App>,
    provider_id: i64,
) -> Result<Vec<core_rpc::ModelInfo>, String> {
    let provider = ai_client(&app, provider_id)?;
    core_rpc::ai::list_models(&provider).await.map_err(fail)
}

/// Asks a model to do a job once, on a sample rather than anyone's mail.
#[tauri::command]
async fn ai_try(
    app: State<'_, App>,
    task: core_rpc::Task,
    provider_id: i64,
    model: String,
) -> Result<core_rpc::AiTrial, String> {
    let provider = ai_client(&app, provider_id)?;
    core_rpc::ai::try_model(&provider, task, &model)
        .await
        .map_err(fail)
}

#[tauri::command]
fn undo(app: State<'_, App>, account: i64) -> Result<Option<QueuedChange>, String> {
    app.core.lock().unwrap().undo(account).map_err(fail)
}

#[tauri::command]
fn queue(app: State<'_, App>, account: i64) -> Result<Vec<QueuedChange>, String> {
    app.core.lock().unwrap().queue(account).map_err(fail)
}

/// Sends queued changes and brings the mailbox up to date.
///
/// Holds no lock while it runs, so the window stays usable and rows appear as
/// the pass writes them.
///
/// It runs on a thread of its own, which is not a performance choice. A sync
/// holds its `Store` across every await, and `rusqlite`'s connection is `Send`
/// but not `Sync`, so `&Store` is not `Send` and the future is not either —
/// which is what Tauri's async commands require, because they are spawned onto
/// a work-stealing runtime that may move them mid-poll. Giving the future one
/// thread it never leaves satisfies that without making the store something it
/// is not.
#[tauri::command]
async fn sync(
    app: State<'_, App>,
    email: String,
    // Where the sync has got to, folder by folder. A first sync downloads a
    // whole mailbox, and the window has to be able to show that it is moving.
    on_progress: tauri::ipc::Channel<core_rpc::SyncProgress>,
) -> Result<core_rpc::SyncSummary, String> {
    let data_dir = app.data_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(fail)?;
        runtime.block_on(async {
            let session = core_rpc::Session::new(data_dir);
            let summary = session
                .sync_account_reporting(&email, |at| {
                    let _ = on_progress.send(at);
                })
                .await
                .map_err(fail)?;
            // Headers the store came to keep after this mail was synced, read
            // back from disk once. Nothing to do on every sync after the first.
            while let Ok(left) = session.backfill_headers(&email, 2_000) {
                if left == 0 {
                    break;
                }
            }
            // Mail the rules filed before they last changed, filed again —
            // once, after an update; nothing is behind on the syncs after.
            loop {
                match session.reclassify(&email, 2_000, true) {
                    Ok(step) => {
                        if step.changed > 0 {
                            tracing::info!(
                                changed = step.changed,
                                "filed mail again with the current rules"
                            );
                        }
                        if step.remaining == 0 {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "could not file mail again with the current rules");
                        break;
                    }
                }
            }
            Ok(summary)
        })
    })
    .await
    .map_err(|err| format!("the sync thread did not finish: {err}"))?
}

/// Sends a message and files a copy in Sent.
///
/// On its own thread for the same reason [`sync`] is: the send path holds a
/// `Store` across awaits, so its future is not `Send`.
#[tauri::command]
async fn send(
    app: State<'_, App>,
    email: String,
    draft: core_rpc::DraftInput,
) -> Result<core_rpc::SentSummary, String> {
    let data_dir = app.data_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(fail)?;
        runtime.block_on(async {
            core_rpc::Session::new(data_dir)
                .send(&email, &draft, true)
                .await
                .map_err(fail)
        })
    })
    .await
    .map_err(|err| format!("the send thread did not finish: {err}"))?
}

/// Builds a draft without sending it, so compose can show the envelope.
///
/// Synchronous and lock-free: it reads the store but talks to nothing.
#[tauri::command]
fn preview(
    app: State<'_, App>,
    email: String,
    draft: core_rpc::DraftInput,
) -> Result<core_rpc::DraftPreview, String> {
    core_rpc::Session::new(&app.data_dir)
        .preview(&email, &draft)
        .map_err(fail)
}

// -- encryption ---------------------------------------------------------------
//
// OpenPGP keys. The keyring is files in the data directory and passphrases in
// the keychain, so none of this goes near the network or waits on it; the
// store lock is held only by `pgp_import_from_message`, which reads a message.
// Passphrases go one way, like passwords: in, and never back to the window.
// Signing and encrypting outgoing mail are `sign` and `encrypt` on the draft
// that `send` and `preview` already take; decryption happens in `message`.

#[tauri::command]
fn pgp_keys(app: State<'_, App>) -> Result<Vec<core_rpc::KeyView>, String> {
    app.core.lock().unwrap().pgp_keys().map_err(fail)
}

/// Without a passphrase one is made up and kept in the keychain.
#[tauri::command]
fn pgp_generate(
    app: State<'_, App>,
    name: String,
    email: String,
    passphrase: Option<String>,
) -> Result<core_rpc::KeyView, String> {
    app.core
        .lock()
        .unwrap()
        .pgp_generate(&name, &email, passphrase.as_deref())
        .map_err(fail)
}

#[tauri::command]
fn pgp_import(app: State<'_, App>, armored: String) -> Result<core_rpc::ImportReport, String> {
    app.core.lock().unwrap().pgp_import(&armored).map_err(fail)
}

/// The public half only; secret keys are never handed to the window.
#[tauri::command]
fn pgp_export_public(app: State<'_, App>, fingerprint: String) -> Result<String, String> {
    app.core
        .lock()
        .unwrap()
        .pgp_export_public(&fingerprint)
        .map_err(fail)
}

#[tauri::command]
fn pgp_delete(app: State<'_, App>, fingerprint: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .pgp_delete(&fingerprint)
        .map_err(fail)
}

/// Checked against the secret key before it is stored.
#[tauri::command]
fn pgp_set_passphrase(
    app: State<'_, App>,
    fingerprint: String,
    passphrase: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .pgp_set_passphrase(&fingerprint, &passphrase)
        .map_err(fail)
}

/// For compose: which recipients have keys, and whether `email` can sign.
#[tauri::command]
fn pgp_recipients(
    app: State<'_, App>,
    email: String,
    recipients: Vec<String>,
) -> Result<core_rpc::RecipientKeys, String> {
    app.core
        .lock()
        .unwrap()
        .pgp_recipients(&email, &recipients)
        .map_err(fail)
}

#[tauri::command]
fn pgp_import_from_message(
    app: State<'_, App>,
    account: i64,
    id: i64,
) -> Result<core_rpc::ImportReport, String> {
    app.core
        .lock()
        .unwrap()
        .pgp_import_from_message(account, id)
        .map_err(fail)
}

// -- settings ---------------------------------------------------------------

#[tauri::command]
fn account_settings(app: State<'_, App>) -> Result<Vec<core_rpc::AccountSettings>, String> {
    app.core
        .lock()
        .unwrap()
        .all_account_settings()
        .map_err(fail)
}

#[tauri::command]
fn save_account(app: State<'_, App>, input: core_rpc::AccountInput) -> Result<i64, String> {
    app.core.lock().unwrap().save_account(&input).map_err(fail)
}

#[tauri::command]
fn delete_account(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core.lock().unwrap().delete_account(id).map_err(fail)
}

/// Stores an app password in the OS keychain.
///
/// One way only: nothing here reads a password back out. See the note on
/// `core_rpc::settings`.
#[tauri::command]
fn set_password(app: State<'_, App>, email: String, password: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_password(&email, &password)
        .map_err(fail)
}

#[tauri::command]
fn clear_password(app: State<'_, App>, email: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .clear_password(&email)
        .map_err(fail)
}

/// Connects without changing anything and reports what it found.
///
/// Own thread, for the reason [`sync`] explains.
#[tauri::command]
async fn verify_account(
    app: State<'_, App>,
    email: String,
) -> Result<core_rpc::VerifyReport, String> {
    let data_dir = app.data_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(fail)?;
        runtime.block_on(async {
            core_rpc::Session::new(data_dir)
                .verify(&email)
                .await
                .map_err(fail)
        })
    })
    .await
    .map_err(|err| format!("the verify thread did not finish: {err}"))?
}

/// Where Archive and Trash are for this account, resolved from what sync
/// recorded rather than guessed in JavaScript.
#[tauri::command]
fn special_folders(app: State<'_, App>, account: i64) -> Result<core_rpc::SpecialFolders, String> {
    app.core
        .lock()
        .unwrap()
        .special_folders(account)
        .map_err(fail)
}

// -- the setup assistant -------------------------------------------------------
//
// First run, and "Set up again" in settings: the local model server, a
// Paperless, and mail accounts other programs already know. What goes over
// the network or runs a program holds no store lock, as everywhere else.

#[tauri::command]
fn setup_status(app: State<'_, App>) -> Result<core_rpc::setup::SetupStatus, String> {
    app.core
        .lock()
        .unwrap()
        .setup_status(&app.data_dir)
        .map_err(fail)
}

#[tauri::command]
fn finish_setup(app: State<'_, App>) -> Result<(), String> {
    core_rpc::setup::finish(&app.data_dir).map_err(fail)
}

fn ollama_probe(app: &State<'_, App>) -> Result<core_rpc::setup::OllamaProbe, String> {
    app.core.lock().unwrap().ollama_setup().map_err(fail)
}

#[tauri::command]
async fn ollama_status(app: State<'_, App>) -> Result<core_rpc::setup::OllamaStatus, String> {
    let probe = ollama_probe(&app)?;
    Ok(probe.status().await)
}

#[tauri::command]
async fn start_ollama(app: State<'_, App>) -> Result<(), String> {
    let probe = ollama_probe(&app)?;
    probe.start().await.map_err(fail)
}

/// Downloads a model into the Ollama on this computer, reporting progress on
/// `on_progress` as Ollama gives it.
#[tauri::command]
async fn pull_model(
    app: State<'_, App>,
    model: String,
    on_progress: tauri::ipc::Channel<core_rpc::setup::PullProgress>,
) -> Result<(), String> {
    let probe = ollama_probe(&app)?;
    probe
        .pull(&model, |update| {
            let _ = on_progress.send(update);
        })
        .await
        .map_err(fail)
}

/// Runs `docker info`, which takes a moment and blocks, on a thread of its own.
#[tauri::command]
async fn docker_status() -> Result<core_rpc::setup::DockerStatus, String> {
    tauri::async_runtime::spawn_blocking(core_rpc::setup::docker_status)
        .await
        .map_err(fail)
}

#[tauri::command]
fn start_docker() -> Result<(), String> {
    core_rpc::setup::start_docker().map_err(fail)
}

#[tauri::command]
async fn find_paperless(
    app: State<'_, App>,
) -> Result<Vec<core_rpc::setup::PaperlessFound>, String> {
    let configured: Vec<String> = app
        .core
        .lock()
        .unwrap()
        .paper_mailboxes()
        .map_err(fail)?
        .into_iter()
        .map(|mailbox| mailbox.base_url)
        .collect();
    Ok(core_rpc::setup::find_paperless(&configured, &app.data_dir).await)
}

/// Saves a postal address once its token works, and returns its id.
fn save_postbox(
    app: &State<'_, App>,
    input: &core_rpc::PaperMailboxInput,
    token: &str,
) -> Result<i64, String> {
    let core = app.core.lock().unwrap();
    let id = core.save_paper_mailbox(input).map_err(fail)?;
    core.set_paper_token(id, token).map_err(fail)?;
    Ok(id)
}

/// Connects to a Paperless that already exists: with a user name and
/// password, which are exchanged for the user's token, or with the token.
#[tauri::command]
async fn connect_paperless(
    app: State<'_, App>,
    input: core_rpc::PaperMailboxInput,
    username: Option<String>,
    password: Option<String>,
    token: Option<String>,
) -> Result<core_rpc::PaperReport, String> {
    let token = match (username.filter(|u| !u.trim().is_empty()), password, token) {
        (Some(username), Some(password), _) => {
            core_rpc::setup::sign_in_paperless(&input.base_url, username.trim(), &password)
                .await
                .map_err(fail)?
        }
        (_, _, Some(token)) if !token.trim().is_empty() => token.trim().to_string(),
        _ => {
            return Err("sign in with a Paperless user name and password, or paste a token".into())
        }
    };
    let report = core_rpc::setup::check_paperless(
        &input.base_url,
        &token,
        &input.selector_kind,
        input.selector_value.as_deref(),
    )
    .await
    .map_err(fail)?;
    save_postbox(&app, &input, &token)?;
    Ok(report)
}

/// Installs Paperless with Docker, starts it, signs in and adds it as a
/// postal address. Docker's output arrives on `on_output` line by line.
#[tauri::command]
async fn install_paperless(
    app: State<'_, App>,
    install: core_rpc::setup::PaperlessInstall,
    label: String,
    on_output: tauri::ipc::Channel<String>,
) -> Result<i64, String> {
    // A password nobody chose is a password nobody has to think up, reuse or
    // remember: it is kept in the keychain and shown on request in settings.
    let install = core_rpc::setup::PaperlessInstall {
        password: match install.password.trim().is_empty() {
            true => core_rpc::setup::generated_password().map_err(fail)?,
            false => install.password,
        },
        ..install
    };
    let (base_url, dir) =
        core_rpc::setup::write_paperless(&app.data_dir, &install).map_err(fail)?;
    let _ = on_output.send(format!("Paperless goes in {}", dir.display()));

    let output = on_output.clone();
    tauri::async_runtime::spawn_blocking(move || {
        core_rpc::setup::compose_up(&dir, |line| {
            let _ = output.send(line);
        })
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    let token = core_rpc::setup::wait_and_sign_in(
        &base_url,
        install.username.trim(),
        &install.password,
        |line| {
            let _ = on_output.send(line);
        },
    )
    .await
    .map_err(fail)?;
    let input = core_rpc::PaperMailboxInput {
        id: None,
        label: if label.trim().is_empty() {
            "Post".to_string()
        } else {
            label.trim().to_string()
        },
        base_url,
        selector_kind: "everything".to_string(),
        selector_value: None,
    };
    let id = save_postbox(&app, &input, &token)?;
    app.core
        .lock()
        .unwrap()
        .set_paper_sign_in(id, install.username.trim(), &install.password)
        .map_err(fail)?;
    let _ = on_output.send("Paperless is running and connected.".to_string());
    Ok(id)
}

/// The sign-in for a Paperless kuverta installed: what to type on its own
/// pages, which is not the token kuverta reads with. `None` for an address
/// whose Paperless belongs to somebody else.
///
/// An install made before kuverta kept this in the keychain has it only in
/// the settings file it wrote, so the first ask moves it across.
#[tauri::command]
fn paper_sign_in(app: State<'_, App>, id: i64) -> Result<Option<core_rpc::PaperSignIn>, String> {
    let core = app.core.lock().unwrap();
    if let Some(sign_in) = core.paper_sign_in(id).map_err(fail)? {
        return Ok(Some(sign_in));
    }
    // Only for the address that is this computer's install: another
    // Paperless's sign-in is not in a file here.
    let ours = core_rpc::setup::installed_paperless(&app.data_dir).map(|(url, _)| url);
    let mine = core
        .paper_mailboxes()
        .map_err(fail)?
        .into_iter()
        .find(|mailbox| mailbox.id == id)
        .ok_or_else(|| format!("no postal address {id}"))?;
    let same = |a: &str, b: &str| {
        a.trim()
            .trim_end_matches('/')
            .eq_ignore_ascii_case(b.trim().trim_end_matches('/'))
    };
    if !ours.is_some_and(|ours| same(&ours, &mine.base_url)) {
        return Ok(None);
    }
    let Some((username, password)) = core_rpc::setup::installed_sign_in(&app.data_dir) else {
        return Ok(None);
    };
    core.set_paper_sign_in(id, &username, &password)
        .map_err(fail)?;
    Ok(Some(core_rpc::PaperSignIn { username, password }))
}

/// The folders on the shelf an address's post is sorted into, as Paperless
/// keeps them.
#[tauri::command]
async fn paper_folders(app: State<'_, App>, id: i64) -> Result<Vec<core_rpc::ShelfFolder>, String> {
    let session = paper_session(&app, id)?;
    session.folders().await.map_err(fail)
}

/// Sets them up there, which is also where the scanner reads them from.
#[tauri::command]
async fn set_paper_folders(
    app: State<'_, App>,
    id: i64,
    folders: Vec<core_rpc::ShelfFolder>,
) -> Result<(), String> {
    let session = paper_session(&app, id)?;
    session.set_folders(&folders).await.map_err(fail)
}

/// Whether each postal address's Paperless answers, and which kuverta can
/// start.
#[tauri::command]
async fn paperless_health(
    app: State<'_, App>,
) -> Result<Vec<core_rpc::setup::PaperlessHealth>, String> {
    let mailboxes: Vec<(i64, String)> = app
        .core
        .lock()
        .unwrap()
        .paper_mailboxes()
        .map_err(fail)?
        .into_iter()
        .map(|mailbox| (mailbox.id, mailbox.base_url))
        .collect();
    Ok(core_rpc::setup::paperless_health(&mailboxes, &app.data_dir).await)
}

/// Starts the Paperless kuverta installed, Docker first if need be. What is
/// happening arrives on `on_output` line by line.
#[tauri::command]
async fn start_paperless(
    app: State<'_, App>,
    on_output: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    core_rpc::setup::start_paperless(&app.data_dir, move |line| {
        let _ = on_output.send(line);
    })
    .await
    .map_err(fail)
}

fn known_addresses(app: &State<'_, App>) -> Result<Vec<String>, String> {
    Ok(app
        .core
        .lock()
        .unwrap()
        .accounts()
        .map_err(fail)?
        .into_iter()
        .map(|account| account.email)
        .collect())
}

/// What other mail programs on this computer know. `primary_password` is
/// Thunderbird's, when it has one; empty otherwise.
#[tauri::command]
async fn import_accounts(
    app: State<'_, App>,
    primary_password: Option<String>,
) -> Result<core_rpc::setup::ImportScan, String> {
    let existing = known_addresses(&app)?;
    Ok(
        core_rpc::setup::scan_imports(&existing, primary_password.as_deref().unwrap_or_default())
            .await,
    )
}

/// Saves an imported account with the password the program it came from has,
/// so nothing has to be typed again. The password goes from that program to
/// the keychain without passing through the window.
#[tauri::command]
fn save_account_with_found_password(
    app: State<'_, App>,
    input: core_rpc::AccountInput,
    primary_password: Option<String>,
) -> Result<i64, String> {
    let password =
        core_rpc::setup::stored_password(&input, primary_password.as_deref().unwrap_or_default())
            .ok_or_else(|| {
            format!(
                "no saved password was found for {} — enter it here",
                input.email
            )
        })?;
    let core = app.core.lock().unwrap();
    let id = core.save_account(&input).map_err(fail)?;
    core.set_password(&input.email, &password).map_err(fail)?;
    Ok(id)
}

#[tauri::command]
async fn lookup_account(
    app: State<'_, App>,
    email: String,
) -> Result<core_rpc::setup::FoundAccount, String> {
    let existing = known_addresses(&app)?;
    core_rpc::setup::lookup(&email, &existing)
        .await
        .map_err(fail)
}

/// Opens a web page, or the System Settings pane the assistant points at, in
/// the program that shows it. Nothing else: the window has no business
/// starting arbitrary programs.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let allowed = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("x-apple.systempreferences:");
    if !allowed || url.chars().any(char::is_whitespace) {
        return Err(format!("{url} is not something to open"));
    }
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .status()
    } else {
        std::process::Command::new("xdg-open").arg(&url).status()
    };
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("could not open {url} ({status})")),
        Err(err) => Err(format!("could not open {url}: {err}")),
    }
}

/// Holds `kuverta.lock` in the data directory for as long as the window is
/// open. The operating system releases it when the process ends, however it
/// ends, so a crash never leaves the directory locked.
fn lock_data_dir(data_dir: &std::path::Path) -> Result<std::fs::File, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|err| format!("cannot create {}: {err}", data_dir.display()))?;
    let path = data_dir.join("kuverta.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "kuverta is already open on {} — switch to that window, or set KUVERTA_DATA_DIR",
            data_dir.display()
        )),
        Err(std::fs::TryLockError::Error(err)) => {
            Err(format!("cannot lock {}: {err}", path.display()))
        }
    }
}

// -- updates ------------------------------------------------------------------

/// The running version and whether a newer one has been released. `None` when
/// checking is switched off: for a dev instance, or with
/// `KUVERTA_NO_UPDATE_CHECK` set.
#[tauri::command]
async fn check_for_update(manual: bool) -> Result<Option<core_rpc::update::UpdateInfo>, String> {
    let off = std::env::var_os("KUVERTA_NO_UPDATE_CHECK").is_some()
        || core_accounts::instance().is_some();
    if off && !manual {
        return Ok(None);
    }
    let current = env!("CARGO_PKG_VERSION");
    let found = core_rpc::update::check(&core_rpc::update::repo(), current)
        .await
        .map_err(fail)?;
    // No release yet reads, to a person who asked, as being up to date.
    Ok(Some(found.unwrap_or_else(|| {
        core_rpc::update::UpdateInfo {
            current: current.to_string(),
            latest: current.to_string(),
            newer: false,
            url: format!("https://github.com/{}/releases", core_rpc::update::repo()),
            download_url: None,
            notes: String::new(),
        }
    })))
}

// -- folders ------------------------------------------------------------------

/// Runs a future that holds a `Store` across awaits on a thread of its own —
/// see [`sync`] for why that is needed at all.
async fn on_own_thread<T, F, Fut>(what: &'static str, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(fail)?;
        runtime.block_on(work())
    })
    .await
    .map_err(|err| format!("the {what} thread did not finish: {err}"))?
}

#[tauri::command]
async fn create_folder(
    app: State<'_, App>,
    email: String,
    name: String,
    parent: Option<String>,
) -> Result<core_rpc::CreatedFolder, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("folder", move || async move {
        core_rpc::Session::new(data_dir)
            .create_folder(&email, &name, parent.as_deref())
            .await
            .map_err(fail)
    })
    .await
}

#[tauri::command]
async fn delete_folder(app: State<'_, App>, email: String, folder: i64) -> Result<String, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("folder", move || async move {
        core_rpc::Session::new(data_dir)
            .delete_folder(&email, folder)
            .await
            .map_err(fail)
    })
    .await
}

// -- sending later ------------------------------------------------------------

/// Puts a message in the outbox. Checked the way sending would check it, and
/// sent by the scheduler thread started in `main` when its time comes.
#[tauri::command]
fn schedule_send(
    app: State<'_, App>,
    email: String,
    draft: core_rpc::DraftInput,
    send_at: i64,
) -> Result<i64, String> {
    core_rpc::Session::new(&app.data_dir)
        .schedule(&email, &draft, send_at)
        .map_err(fail)
}

#[tauri::command]
fn outbox(app: State<'_, App>) -> Result<Vec<core_rpc::OutboxView>, String> {
    app.core.lock().unwrap().outbox().map_err(fail)
}

#[tauri::command]
fn cancel_scheduled(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core.lock().unwrap().cancel_scheduled(id).map_err(fail)
}

#[tauri::command]
fn reschedule(app: State<'_, App>, id: i64, send_at: i64) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .reschedule(id, send_at)
        .map_err(fail)
}

/// A waiting message's account and draft, for opening it in compose again.
#[tauri::command]
fn scheduled_draft(app: State<'_, App>, id: i64) -> Result<(String, core_rpc::DraftInput), String> {
    app.core.lock().unwrap().scheduled_draft(id).map_err(fail)
}

/// Sends whatever is due now, rather than at the scheduler's next look.
#[tauri::command]
async fn send_due(app: State<'_, App>) -> Result<Vec<core_rpc::OutboxSent>, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("outbox", move || async move {
        core_rpc::Session::new(data_dir)
            .send_due()
            .await
            .map_err(fail)
    })
    .await
}

// -- smart mailboxes ----------------------------------------------------------

#[tauri::command]
fn smart_mailboxes(
    app: State<'_, App>,
    account: i64,
) -> Result<Vec<core_rpc::SmartMailboxView>, String> {
    app.core
        .lock()
        .unwrap()
        .smart_mailboxes(account)
        .map_err(fail)
}

#[tauri::command]
fn save_smart_mailbox(
    app: State<'_, App>,
    input: core_rpc::SmartMailboxInput,
) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .save_smart_mailbox(&input)
        .map_err(fail)
}

#[tauri::command]
fn delete_smart_mailbox(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .delete_smart_mailbox(id)
        .map_err(fail)
}

#[tauri::command]
fn preview_smart(
    app: State<'_, App>,
    account: i64,
    query: core_rpc::SmartQuery,
) -> Result<core_rpc::SmartPreview, String> {
    app.core
        .lock()
        .unwrap()
        .preview_smart(account, &query, 8)
        .map_err(fail)
}

#[tauri::command]
fn scan_smart_imports(app: State<'_, App>) -> Result<core_rpc::SmartImportScan, String> {
    app.core.lock().unwrap().scan_smart_imports().map_err(fail)
}

#[tauri::command]
fn import_smart_mailboxes(
    app: State<'_, App>,
    chosen: Vec<core_rpc::FoundSmartMailbox>,
    fallback: i64,
) -> Result<usize, String> {
    app.core
        .lock()
        .unwrap()
        .import_smart_mailboxes(&chosen, fallback)
        .map_err(fail)
}

// -- cleanup ------------------------------------------------------------------

#[tauri::command]
fn cleanup_counts(app: State<'_, App>, account: i64) -> Result<core_rpc::CleanupCounts, String> {
    app.core
        .lock()
        .unwrap()
        .cleanup_counts(account)
        .map_err(fail)
}

#[tauri::command]
fn unsubscribe_senders(
    app: State<'_, App>,
    account: i64,
) -> Result<Vec<core_rpc::UnsubscribeSenderView>, String> {
    app.core
        .lock()
        .unwrap()
        .unsubscribe_senders(account)
        .map_err(fail)
}

/// Reads headers back out of mail synced before they were kept; returns how
/// many messages are left. Its own connection, so the list stays usable.
#[tauri::command]
async fn backfill_headers(app: State<'_, App>, email: String) -> Result<usize, String> {
    let data_dir = app.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        core_rpc::Session::new(data_dir)
            .backfill_headers(&email, 2_000)
            .map_err(fail)
    })
    .await
    .map_err(fail)?
}

#[tauri::command]
async fn unsubscribe(
    app: State<'_, App>,
    email: String,
    keys: Vec<String>,
) -> Result<Vec<core_rpc::UnsubscribeResult>, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("unsubscribe", move || async move {
        core_rpc::Session::new(data_dir)
            .unsubscribe(&email, &keys)
            .await
            .map_err(fail)
    })
    .await
}

#[tauri::command]
fn unsubscribe_opened(
    app: State<'_, App>,
    account: i64,
    key: String,
    url: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .record_unsubscribe_opened(account, &key, &url)
        .map_err(fail)
}

/// Every message from a sender, to the Trash — each an ordinary queued move
/// with its own undo.
#[tauri::command]
fn trash_from_sender(app: State<'_, App>, account: i64, key: String) -> Result<usize, String> {
    let core = app.core.lock().unwrap();
    let trash = core
        .special_folders(account)
        .map_err(fail)?
        .trash
        .ok_or("this account has no Trash folder")?;
    core.trash_from_sender(account, &key, &trash).map_err(fail)
}

/// Messages like `id`: for asking, after it is deleted, whether they should go too.
#[tauri::command]
fn similar(app: State<'_, App>, account: i64, id: i64) -> Result<core_rpc::SimilarReport, String> {
    let core = app.core.lock().unwrap();
    let trash = core.special_folders(account).map_err(fail)?.trash;
    core.similar(account, id, trash.as_deref()).map_err(fail)
}

// -- what needs attention -----------------------------------------------------

/// How far the urgency agent has got, for the window to show.
#[derive(Clone, serde::Serialize)]
struct UrgencyProgress {
    done: usize,
    total: usize,
    subject: String,
}

/// What an urgency pass did.
#[derive(serde::Serialize)]
struct UrgencyPass {
    /// Messages the rules judged for the first time.
    by_rules: usize,
    /// Messages the model judged.
    by_model: usize,
    model: Option<String>,
    /// Whether the model runs on this computer; a hosted one was sent mail.
    local: bool,
    /// Why the model stopped, when it did.
    stopped: Option<String>,
}

/// Runs the urgency agent over recent Inbox mail: the rules first, for every
/// message at once, then — with `use_model` — the model chosen for sorting
/// mail, one message at a time and with no store lock held while it thinks.
/// Stops after three failures in a row rather than grinding on against a
/// model that is not there.
#[tauri::command]
async fn find_urgent(
    app: State<'_, App>,
    account: i64,
    use_model: bool,
    on_progress: tauri::ipc::Channel<UrgencyProgress>,
) -> Result<UrgencyPass, String> {
    let by_rules = app
        .core
        .lock()
        .unwrap()
        .judge_by_rules(account)
        .map_err(fail)?;
    let mut pass = UrgencyPass {
        by_rules,
        by_model: 0,
        model: None,
        local: true,
        stopped: None,
    };
    if !use_model {
        return Ok(pass);
    }
    let choice = ai_choice(&app, core_rpc::Task::Chat)?;
    pass.model = Some(choice.model.clone());
    pass.local = choice.local;
    let jobs = app
        .core
        .lock()
        .unwrap()
        .urgency_jobs(account, true, 150)
        .map_err(fail)?;
    let total = jobs.len();
    let mut failures = 0;
    for (done, job) in jobs.into_iter().enumerate() {
        let _ = on_progress.send(UrgencyProgress {
            done,
            total,
            subject: job.subject.clone(),
        });
        match core_rpc::urgency::ask_model(&choice.provider, &choice.model, &job).await {
            Ok(urgency) => {
                failures = 0;
                app.core
                    .lock()
                    .unwrap()
                    .record_urgency(job.id, &urgency)
                    .map_err(fail)?;
                pass.by_model += 1;
            }
            Err(err) => {
                tracing::warn!(message = job.id, %err, "the model could not judge urgency");
                failures += 1;
                if failures >= 3 {
                    pass.stopped = Some(err);
                    break;
                }
            }
        }
    }
    let _ = on_progress.send(UrgencyProgress {
        done: total,
        total,
        subject: String::new(),
    });
    Ok(pass)
}

#[tauri::command]
fn urgent_messages(app: State<'_, App>, account: i64) -> Result<Vec<core_rpc::UrgentRow>, String> {
    app.core
        .lock()
        .unwrap()
        .urgent(account, 2, 300)
        .map_err(fail)
}

#[tauri::command]
fn urgency_of(app: State<'_, App>, id: i64) -> Result<Option<core_rpc::UrgencyView>, String> {
    app.core.lock().unwrap().urgency_of(id).map_err(fail)
}

// -- conversations ---------------------------------------------------------------

#[tauri::command]
fn conversations(
    app: State<'_, App>,
    account: i64,
    offset: usize,
    limit: usize,
    include_bulk: bool,
    query: Option<String>,
) -> Result<core_rpc::ConversationPage, String> {
    app.core
        .lock()
        .unwrap()
        .conversations(account, include_bulk, query.as_deref(), offset, limit)
        .map_err(fail)
}

#[tauri::command]
fn conversation(
    app: State<'_, App>,
    account: i64,
    key: String,
) -> Result<core_rpc::Thread, String> {
    app.core
        .lock()
        .unwrap()
        .conversation(account, &key)
        .map_err(fail)
}

// -- profiles -------------------------------------------------------------------

#[tauri::command]
fn profiles(app: State<'_, App>) -> Result<Vec<core_rpc::ProfileView>, String> {
    app.core.lock().unwrap().profiles().map_err(fail)
}

#[tauri::command]
fn save_profile(app: State<'_, App>, id: Option<i64>, name: String) -> Result<i64, String> {
    app.core
        .lock()
        .unwrap()
        .save_profile(id, &name)
        .map_err(fail)
}

#[tauri::command]
fn delete_profile(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core.lock().unwrap().delete_profile(id).map_err(fail)
}

#[tauri::command]
fn reorder_profiles(app: State<'_, App>, ids: Vec<i64>) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .reorder_profiles(&ids)
        .map_err(fail)
}

#[tauri::command]
fn set_account_profile(
    app: State<'_, App>,
    account: i64,
    profile: Option<i64>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_account_profile(account, profile)
        .map_err(fail)
}

#[tauri::command]
fn set_postbox_profile(
    app: State<'_, App>,
    postbox: i64,
    profile: Option<i64>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_postbox_profile(postbox, profile)
        .map_err(fail)
}

// -- the assistant and tasks -------------------------------------------------------

/// One question to the assistant. The window keeps the conversation and sends
/// it each time; what the assistant does arrives on `on_event` as it happens.
/// On a thread of its own with a store connection of its own, like a sync, so
/// the window stays usable however long the model thinks.
#[tauri::command]
async fn assistant_ask(
    app: State<'_, App>,
    account: i64,
    turns: Vec<core_ai::Turn>,
    message: String,
    // The messages the person has open, which the question is about.
    about: Vec<i64>,
    on_event: tauri::ipc::Channel<core_rpc::AssistantEvent>,
) -> Result<core_rpc::AssistantTurn, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("assistant", move || async move {
        core_rpc::Session::new(data_dir)
            .assistant_turn(account, turns, &message, &about, |event| {
                let _ = on_event.send(event.clone());
            })
            .await
            .map_err(fail)
    })
    .await
}

#[tauri::command]
fn tasks(app: State<'_, App>, account: i64) -> Result<Vec<core_rpc::TaskView>, String> {
    app.core.lock().unwrap().tasks(account).map_err(fail)
}

#[tauri::command]
fn save_task(app: State<'_, App>, input: core_rpc::TaskInput) -> Result<i64, String> {
    app.core.lock().unwrap().save_task(&input).map_err(fail)
}

#[tauri::command]
fn delete_task(app: State<'_, App>, id: i64) -> Result<(), String> {
    app.core.lock().unwrap().delete_task(id).map_err(fail)
}

#[tauri::command]
fn set_task_enabled(app: State<'_, App>, id: i64, enabled: bool) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_task_enabled(id, enabled)
        .map_err(fail)
}

/// Runs the account's tasks: the simple ones at once, and with `use_model`
/// those that ask the model, reporting `[done, total]` for those.
#[tauri::command]
async fn run_tasks(
    app: State<'_, App>,
    account: i64,
    use_model: bool,
    on_progress: tauri::ipc::Channel<(usize, usize)>,
) -> Result<Vec<core_rpc::TaskRun>, String> {
    let data_dir = app.data_dir.clone();
    on_own_thread("tasks", move || async move {
        core_rpc::Session::new(data_dir)
            .run_tasks(account, use_model, |done, total| {
                let _ = on_progress.send((done, total));
            })
            .await
            .map_err(fail)
    })
    .await
}

#[tauri::command]
fn proposals(app: State<'_, App>, account: i64) -> Result<Vec<core_rpc::ProposalView>, String> {
    app.core.lock().unwrap().proposals(account).map_err(fail)
}

#[tauri::command]
fn approve_proposal(app: State<'_, App>, account: i64, id: i64) -> Result<Option<i64>, String> {
    app.core
        .lock()
        .unwrap()
        .approve_proposal(account, id)
        .map_err(fail)
}

#[tauri::command]
fn settle_proposal(
    app: State<'_, App>,
    id: i64,
    state: String,
    detail: Option<String>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .settle_proposal(id, &state, detail.as_deref())
        .map_err(fail)
}

/// Cancels queued changes — the assistant's, by the ids it reported — while
/// they are still waiting.
#[tauri::command]
fn cancel_changes(app: State<'_, App>, ids: Vec<i64>) -> Result<usize, String> {
    app.core.lock().unwrap().cancel_changes(&ids).map_err(fail)
}

// -- the log ------------------------------------------------------------------

#[tauri::command]
fn log_status(app: State<'_, App>) -> logging::LogStatus {
    app.logs.status()
}

#[tauri::command]
fn set_detailed_logging(app: State<'_, App>, on: bool) -> Result<(), String> {
    app.logs.set_detailed(on)
}

#[tauri::command]
fn log_tail(app: State<'_, App>, lines: usize) -> String {
    app.logs.tail(lines.min(2_000))
}

/// Writes the log to Downloads and shows it there.
#[tauri::command]
fn export_log(app: State<'_, App>) -> Result<String, String> {
    let path = app.logs.export(&app.data_dir)?;
    let _ = logging::reveal(&path);
    Ok(path.display().to_string())
}

/// Something the window wants on the record — an error it showed, mostly —
/// so a log sent with a report has both halves of what happened.
#[tauri::command]
fn log_ui(level: String, message: String) {
    let message: String = message.chars().take(2_000).collect();
    match level.as_str() {
        "error" => tracing::error!(target: "window", "{message}"),
        "warn" => tracing::warn!(target: "window", "{message}"),
        _ => tracing::info!(target: "window", "{message}"),
    }
}

/// Sends scheduled mail when it falls due, for as long as the window is open.
///
/// A thread of its own with its own store connection, like a sync. Every
/// half minute is often enough that "8:00" means 8:00 to anyone reading it,
/// and a look at an empty outbox is one indexed query.
fn start_scheduler(handle: tauri::AppHandle, data_dir: PathBuf) {
    use tauri::Emitter;
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::error!(%err, "could not start the outbox scheduler");
                return;
            }
        };
        let session = core_rpc::Session::new(&data_dir);
        loop {
            match runtime.block_on(session.send_due()) {
                Ok(sent) if !sent.is_empty() => {
                    let _ = handle.emit("outbox-sent", &sent);
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(%err, "the outbox could not be checked"),
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
    });
}

fn default_data_dir() -> PathBuf {
    core_accounts::default_data_dir()
}

fn main() {
    let data_dir = default_data_dir();
    // One window per data directory. Two — the installed app and one built
    // from source, say — would each sync the same store and each believe its
    // own list; the second is told where the first is instead.
    let _lock = match lock_data_dir(&data_dir) {
        Ok(lock) => lock,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let logs = logging::Logs::init(&data_dir);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "kuverta starting");
    let core = match Core::open(&data_dir) {
        Ok(core) => core,
        Err(err) => {
            // A window showing an empty list would be a worse way to say this.
            eprintln!("cannot open the store in {}: {err}", data_dir.display());
            eprintln!("set KUVERTA_DATA_DIR, or run `kuverta sync` first");
            std::process::exit(1);
        }
    };
    tracing::info!(dir = %data_dir.display(), "opened store");
    // A send the last window was in the middle of when it closed. Whether it
    // went cannot be known from here, so it waits for a person rather than
    // being sent again on a guess.
    match core.store().recover_interrupted_sends() {
        Ok(0) => {}
        Ok(n) => tracing::warn!(n, "scheduled messages were interrupted mid-send"),
        Err(err) => tracing::warn!(%err, "could not check the outbox"),
    }

    let instance = core_accounts::instance();
    let scheduler_dir = data_dir.clone();
    tauri::Builder::default()
        .setup(move |app| {
            start_scheduler(app.handle().clone(), scheduler_dir.clone());
            // A dev window says so, so it is never mistaken for the real one.
            if let Some(instance) = &instance {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title(&format!("kuverta — {instance}"));
                }
            }
            Ok(())
        })
        .manage(App {
            core: Mutex::new(core),
            data_dir,
            logs,
        })
        .invoke_handler(tauri::generate_handler![
            accounts,
            messages,
            message,
            attachment,
            save_attachment,
            open_attachment,
            search,
            folders,
            category_counts,
            move_to,
            set_read,
            set_category,
            paper_mailboxes,
            paper_folders,
            set_paper_folders,
            save_paper_mailbox,
            delete_paper_mailbox,
            set_paper_token,
            paper_documents,
            paper_document,
            paper_file,
            set_paper_read,
            paper_unread_count,
            paper_category_counts,
            paper_transcribe,
            paper_check,
            file_post,
            undo,
            queue,
            sync,
            send,
            preview,
            pgp_keys,
            pgp_generate,
            pgp_import,
            pgp_export_public,
            pgp_delete,
            pgp_set_passphrase,
            pgp_recipients,
            pgp_import_from_message,
            special_folders,
            account_settings,
            save_account,
            delete_account,
            set_password,
            clear_password,
            verify_account,
            ai_providers,
            save_ai_provider,
            delete_ai_provider,
            set_ai_key,
            ai_tasks,
            set_ai_task,
            ai_models,
            ai_try,
            setup_status,
            finish_setup,
            ollama_status,
            start_ollama,
            pull_model,
            docker_status,
            start_docker,
            find_paperless,
            connect_paperless,
            install_paperless,
            paperless_health,
            paper_sign_in,
            start_paperless,
            import_accounts,
            save_account_with_found_password,
            lookup_account,
            open_external,
            check_for_update,
            create_folder,
            delete_folder,
            schedule_send,
            outbox,
            cancel_scheduled,
            reschedule,
            scheduled_draft,
            send_due,
            smart_mailboxes,
            save_smart_mailbox,
            delete_smart_mailbox,
            preview_smart,
            scan_smart_imports,
            import_smart_mailboxes,
            cleanup_counts,
            unsubscribe_senders,
            backfill_headers,
            unsubscribe,
            unsubscribe_opened,
            trash_from_sender,
            similar,
            log_status,
            set_detailed_logging,
            log_tail,
            export_log,
            log_ui,
            find_urgent,
            urgent_messages,
            urgency_of,
            conversations,
            conversation,
            profiles,
            save_profile,
            delete_profile,
            reorder_profiles,
            set_account_profile,
            set_postbox_profile,
            assistant_ask,
            tasks,
            save_task,
            delete_task,
            set_task_enabled,
            run_tasks,
            proposals,
            approve_proposal,
            settle_proposal,
            cancel_changes,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the window");
}
