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
) -> Result<MessagePage, String> {
    let filter = ListFilter {
        category,
        unread_only,
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

/// Where Archive and Trash are for this account, resolved from what sync
/// recorded rather than guessed in JavaScript.
#[tauri::command]
fn special_folders(app: State<'_, App>, account: i64) -> Result<SpecialFolders, String> {
    let core = app.core.lock().unwrap();
    let folders = core.store().folders(account).map_err(fail)?;
    let pairs: Vec<(&str, Option<&str>)> = folders
        .iter()
        .map(|f| (f.name.as_str(), f.special_use.as_deref()))
        .collect();

    Ok(SpecialFolders {
        archive: core_proto::client::find_archive(pairs.iter().copied()).map(str::to_string),
        trash: core_proto::client::find_trash(pairs.iter().copied()).map(str::to_string),
    })
}

#[derive(serde::Serialize)]
struct SpecialFolders {
    /// `None` when the account has no Archive folder — on which the UI has to
    /// disable archiving rather than fail per keystroke.
    archive: Option<String>,
    trash: Option<String>,
}

fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FUCKMAIL_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs_home()
        .map(|home| home.join(".local/share/fuckmail"))
        .unwrap_or_else(|| PathBuf::from(".fuckmail"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let data_dir = default_data_dir();
    let core = match Core::open(&data_dir) {
        Ok(core) => core,
        Err(err) => {
            // A window showing an empty list would be a worse way to say this.
            eprintln!("cannot open the store in {}: {err}", data_dir.display());
            eprintln!("set FUCKMAIL_DATA_DIR, or run `fuckmail sync` first");
            std::process::exit(1);
        }
    };
    tracing::info!(dir = %data_dir.display(), "opened store");

    tauri::Builder::default()
        .manage(App {
            core: Mutex::new(core),
            data_dir,
        })
        .invoke_handler(tauri::generate_handler![
            accounts,
            messages,
            message,
            search,
            category_counts,
            move_to,
            set_read,
            undo,
            queue,
            sync,
            special_folders,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the window");
}
