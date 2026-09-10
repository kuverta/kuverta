//! Folder synchronisation into the local store.
//!
//! Three things happen per folder, in order: new messages are fetched, flag
//! changes are applied, and messages the server no longer has are removed.
//! A folder whose `HIGHESTMODSEQ` is unchanged since the last pass is skipped
//! entirely — on a quiet mailbox that is every folder, and a sync costs one
//! `EXAMINE` each.

use core_store::model::{AccountId, FolderId, Location};
use core_store::{dedup_key, Blobs, Store, Upsert};

use crate::client::{ImapClient, RemoteFolder};
use crate::parse::parse_message;
use crate::ProtoError;

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub folders_synced: usize,
    /// Folders whose `HIGHESTMODSEQ` was unchanged, so nothing was fetched.
    pub folders_skipped: usize,
    /// Messages new to the store.
    pub inserted: usize,
    /// Messages already known — the same mail under another folder or a
    /// re-sync. On Gmail this is routinely large and is not a problem.
    pub deduplicated: usize,
    /// Locations whose flags actually changed.
    pub flag_updates: usize,
    /// Locations removed because the server no longer has them.
    pub expunged: usize,
    /// Bodies the parser could not make sense of at all.
    pub unparseable: usize,
    /// Folders whose UIDVALIDITY changed, forcing a rebuild.
    pub invalidated: usize,
}

/// The parts of a sync that do not change between folders.
struct Context<'a> {
    store: &'a Store,
    blobs: &'a Blobs,
    account_id: AccountId,
}

/// Syncs every selectable folder on the account.
pub async fn sync_account(
    client: &mut ImapClient,
    store: &Store,
    blobs: &Blobs,
    account_id: AccountId,
) -> Result<SyncReport, ProtoError> {
    let mut report = SyncReport::default();
    let ctx = Context {
        store,
        blobs,
        account_id,
    };

    for remote in client.folders().await? {
        if !remote.selectable {
            tracing::debug!(folder = %remote.name, "skipping \\Noselect folder");
            continue;
        }
        sync_folder(client, &ctx, &remote, &mut report).await?;
    }

    Ok(report)
}

async fn sync_folder(
    client: &mut ImapClient,
    ctx: &Context<'_>,
    remote: &RemoteFolder,
    report: &mut SyncReport,
) -> Result<(), ProtoError> {
    let store = ctx.store;
    let folder_id =
        store.upsert_folder(ctx.account_id, &remote.name, remote.special_use.as_deref())?;
    let cached = store.folder(folder_id)?;
    let state = client.examine(&remote.name).await?;

    // A folder is "new to us" until a sync has recorded its UIDVALIDITY. Until
    // then there is nothing cached to reconcile against.
    let mut first_pass = cached.as_ref().is_none_or(|f| f.uid_validity.is_none());
    let mut known_modseq = cached.as_ref().and_then(|f| f.highest_modseq);

    // UIDVALIDITY changing means the server renumbered the folder and every UID
    // cached for it is meaningless. Anything less than a rebuild risks
    // attaching stored messages to the wrong mail.
    if let (Some(known), Some(current)) = (
        cached.as_ref().and_then(|f| f.uid_validity),
        state.uid_validity,
    ) {
        if known != current {
            tracing::warn!(
                folder = %remote.name, known, current,
                "UIDVALIDITY changed, discarding cached UIDs for this folder"
            );
            store.invalidate_folder(folder_id)?;
            report.invalidated += 1;
            first_pass = true;
            known_modseq = None;
        }
    }

    // Nothing in this folder has changed since the last pass. This is the
    // common case and the whole point of asking for CONDSTORE.
    if !first_pass {
        if let (Some(known), Some(current)) = (known_modseq, state.highest_modseq) {
            if known == current {
                tracing::debug!(folder = %remote.name, modseq = known, "unchanged, skipping");
                report.folders_skipped += 1;
                return Ok(());
            }
        }
    }

    fetch_new(client, ctx, folder_id, &state, report).await?;

    // Flag and expunge reconciliation only make sense against something already
    // cached; on a first pass the fetch above has just recorded current state.
    if !first_pass {
        apply_flag_changes(client, store, folder_id, known_modseq, report).await?;
        apply_expunges(client, store, folder_id, report).await?;
    }

    store.set_folder_sync_state(
        folder_id,
        state.uid_validity,
        state.uid_next,
        state.highest_modseq,
    )?;
    report.folders_synced += 1;
    Ok(())
}

async fn fetch_new(
    client: &mut ImapClient,
    ctx: &Context<'_>,
    folder_id: FolderId,
    // Passed in rather than re-examined: the caller has already opened the
    // folder, and a second EXAMINE is a needless round trip.
    state: &crate::client::FolderState,
    report: &mut SyncReport,
) -> Result<(), ProtoError> {
    let store = ctx.store;
    let after = store.max_uid(folder_id)?;

    for message in client.fetch_since(state, after).await? {
        let Some(mut parsed) = parse_message(&message.raw, message.size) else {
            tracing::warn!(folder_id, uid = message.uid, "could not parse message");
            report.unparseable += 1;
            continue;
        };

        // Blob path is keyed by the same identity the store dedups on, so one
        // message shared across folders keeps exactly one body on disk.
        let key = dedup_key(&parsed);
        parsed.body_path = Some(ctx.blobs.put(ctx.account_id, &key, &message.raw)?);

        let location = Location {
            folder_id,
            uid: message.uid,
            flags: message.flags,
        };

        match store.upsert_message(ctx.account_id, &parsed, Some(&location))? {
            (_, Upsert::Inserted) => report.inserted += 1,
            (_, Upsert::Deduplicated) => report.deduplicated += 1,
        }
    }

    Ok(())
}

async fn apply_flag_changes(
    client: &mut ImapClient,
    store: &Store,
    folder_id: FolderId,
    known_modseq: Option<u64>,
    report: &mut SyncReport,
) -> Result<(), ProtoError> {
    for (uid, flags) in client.fetch_flag_changes(known_modseq).await? {
        if store.set_location_flags(folder_id, uid, &flags)? {
            report.flag_updates += 1;
        }
    }
    Ok(())
}

async fn apply_expunges(
    client: &mut ImapClient,
    store: &Store,
    folder_id: FolderId,
    report: &mut SyncReport,
) -> Result<(), ProtoError> {
    let on_server = client.all_uids().await?;
    let gone: Vec<u32> = store
        .folder_uids(folder_id)?
        .into_iter()
        .filter(|uid| !on_server.contains(uid))
        .collect();

    if !gone.is_empty() {
        tracing::debug!(folder_id, count = gone.len(), "removing expunged messages");
        report.expunged += store.remove_locations(folder_id, &gone)?;
    }

    Ok(())
}
