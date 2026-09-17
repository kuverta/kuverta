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

#[tauri::command]
fn messages(
    app: State<'_, App>,
    account: i64,
    offset: usize,
    limit: usize,
    category: Option<String>,
    unread_only: bool,
    folder: Option<i64>,
) -> Result<MessagePage, String> {
    let filter = ListFilter {
        category,
        unread_only,
        folder,
    };
    app.core
        .lock()
        .unwrap()
        .messages(account, offset, limit, &filter)
        .map_err(fail)
}

#[tauri::command]
fn message(app: State<'_, App>, account: i64, id: i64) -> Result<MessageDetail, String> {
    app.core.lock().unwrap().message(account, id).map_err(fail)
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
fn category_counts(app: State<'_, App>, account: i64) -> Result<Vec<(String, usize)>, String> {
    app.core
        .lock()
        .unwrap()
        .category_counts(account)
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
async fn sync(app: State<'_, App>, email: String) -> Result<core_rpc::SyncSummary, String> {
    let data_dir = app.data_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(fail)?;
        runtime.block_on(async {
            core_rpc::Session::new(data_dir)
                .sync_account(&email)
                .await
                .map_err(fail)
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
    let _ = on_output.send("Paperless is running and connected.".to_string());
    Ok(id)
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

fn default_data_dir() -> PathBuf {
    core_accounts::default_data_dir()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

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

    let instance = core_accounts::instance();
    tauri::Builder::default()
        .setup(move |app| {
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
        })
        .invoke_handler(tauri::generate_handler![
            accounts,
            messages,
            message,
            search,
            folders,
            category_counts,
            move_to,
            set_read,
            set_category,
            paper_mailboxes,
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
            import_accounts,
            save_account_with_found_password,
            lookup_account,
            open_external,
            check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the window");
}
