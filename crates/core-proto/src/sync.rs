//! Folder synchronisation into the local store.
//!
//! Three things happen per folder, in order: new messages are fetched, flag
//! changes are applied, and messages the server no longer has are removed.
//! A folder whose `HIGHESTMODSEQ` is unchanged since the last pass is skipped
//! entirely — on a quiet mailbox that is every folder, and a sync costs one
//! `EXAMINE` each.

use core_store::model::{AccountId, FolderId, Location, MessageId};
use core_store::{dedup_key, Blobs, Store, Upsert};

use crate::client::{ImapClient, RemoteFolder};
use crate::parse::{parse_message, Parsed};
use crate::ProtoError;
use core_rules::Classifier;

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
    /// Messages that ended the sync in no folder at all, and were dropped.
    /// A message that merely moved is not one of these.
    pub deleted: usize,
    /// Folders the account is configured not to sync.
    pub folders_excluded: usize,
}

/// Where a sync has got to, for something that wants to show it.
///
/// The counts are per folder: how many messages this folder is fetching and
/// how many of them are in. A caller wanting one number for the account gets
/// it from `folders_done` plus the fraction of the folder in hand.
#[derive(Debug, Clone, Default)]
pub struct SyncProgress {
    /// The folder being fetched.
    pub folder: String,
    /// Folders finished before this one.
    pub folders_done: usize,
    /// Folders this sync will visit at all.
    pub folders_total: usize,
    /// Messages fetched from this folder so far.
    pub messages_done: usize,
    /// Messages this folder has to fetch. Zero until the sizes are in, and
    /// zero for a folder with nothing new.
    pub messages_total: usize,
}

/// The parts of a sync that do not change between folders.
struct Context<'a> {
    store: &'a Store,
    blobs: &'a Blobs,
    account_id: AccountId,
    classifier: Classifier,
}

/// Builds the learned overrides from the corrections the user has made.
pub fn load_history(
    store: &Store,
    account_id: AccountId,
) -> Result<core_rules::Learned, ProtoError> {
    let mut learned = core_rules::Learned::new();
    for row in store.learned_categories(account_id)? {
        let Some(category) = core_rules::Category::parse(&row.category) else {
            continue;
        };
        if let Some(sender) = row.sender {
            learned.insert_sender(&sender, category);
        }
        if let Some(list_id) = row.list_id {
            learned.insert_list(&list_id, category);
        }
    }
    Ok(learned)
}

/// Syncs every selectable folder on the account.
pub async fn sync_account(
    client: &mut ImapClient,
    store: &Store,
    blobs: &Blobs,
    account_id: AccountId,
) -> Result<SyncReport, ProtoError> {
    sync_account_reporting(client, store, blobs, account_id, &mut |_| {}).await
}

/// Syncs every selectable folder, calling `progress` as it goes.
///
/// A first sync of a real mailbox takes long enough that something has to be
/// able to show how far along it is. `progress` is called once per folder
/// before anything is fetched, and again after each batch of messages.
pub async fn sync_account_reporting(
    client: &mut ImapClient,
    store: &Store,
    blobs: &Blobs,
    account_id: AccountId,
    progress: &mut dyn FnMut(SyncProgress),
) -> Result<SyncReport, ProtoError> {
    let mut report = SyncReport::default();
    // The learned history is loaded once per sync. It changes only when the
    // user files something, which cannot happen mid-sync.
    let ctx = Context {
        store,
        blobs,
        account_id,
        classifier: Classifier::new(load_history(store, account_id)?),
    };

    let exclusions = store.folder_exclusions(account_id)?;

    // Which folders this sync will visit is settled before the first one is
    // fetched: a progress bar needs a denominator, and the skipped ones are
    // cheap to recognise here.
    let mut wanted = Vec::new();
    for remote in client.folders().await? {
        if !remote.selectable {
            tracing::debug!(folder = %remote.name, "skipping \\Noselect folder");
            continue;
        }
        if core_store::folder_is_excluded(&exclusions, &remote.name, remote.special_use.as_deref())
        {
            tracing::debug!(folder = %remote.name, "excluded, not syncing");
            report.folders_excluded += 1;
            continue;
        }
        wanted.push(remote);
    }

    let folders_total = wanted.len();
    for (folders_done, remote) in wanted.iter().enumerate() {
        let mut at = SyncProgress {
            folder: remote.name.clone(),
            folders_done,
            folders_total,
            ..SyncProgress::default()
        };
        progress(at.clone());
        sync_folder(client, &ctx, remote, &mut report, &mut at, progress).await?;
    }

    // Only now, with every folder seen. A message that moved was out of its
    // old folder before it turned up in the new one, and collecting orphans
    // per folder would delete it in between — taking its classifier verdict,
    // the user's corrections and any queued operation with it. See
    // `Store::delete_orphaned_messages`.
    report.deleted = store.delete_orphaned_messages()?;

    Ok(report)
}

async fn sync_folder(
    client: &mut ImapClient,
    ctx: &Context<'_>,
    remote: &RemoteFolder,
    report: &mut SyncReport,
    at: &mut SyncProgress,
    progress: &mut dyn FnMut(SyncProgress),
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

    fetch_new(client, ctx, folder_id, &state, report, at, progress).await?;

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
    at: &mut SyncProgress,
    progress: &mut dyn FnMut(SyncProgress),
) -> Result<(), ProtoError> {
    let store = ctx.store;
    let after = store.max_uid(folder_id)?;
    let start = after.map_or(1, |uid| uid.saturating_add(1));

    // Nothing new. Checked against UIDNEXT rather than by asking, so an
    // unchanged folder costs no FETCH at all.
    if let Some(uid_next) = state.uid_next {
        if start >= uid_next {
            return Ok(());
        }
    } else if state.exists == 0 {
        return Ok(());
    }

    // Sizes first, then bodies in batches. The extra round trip buys a bounded
    // memory profile: without it the first sync of a real mailbox holds every
    // message at once. See `plan_batches`.
    let sizes = client.uid_sizes(start).await?;
    at.messages_total = sizes.len();
    progress(at.clone());
    for batch in crate::client::plan_batches(&sizes) {
        for message in client.fetch_uids(&batch).await? {
            store_message(ctx, folder_id, message, report)?;
            at.messages_done += 1;
        }
        // Once per batch, not per message: a batch is up to 200 messages or
        // 16 MB, which is often enough to watch and rare enough to be free.
        progress(at.clone());
    }

    Ok(())
}

/// Parses one fetched message, files its body, and records it.
fn store_message(
    ctx: &Context<'_>,
    folder_id: FolderId,
    message: crate::client::RawMessage,
    report: &mut SyncReport,
) -> Result<(), ProtoError> {
    let Some(mut parsed) = parse_message(&message.raw, message.size) else {
        tracing::warn!(folder_id, uid = message.uid, "could not parse message");
        report.unparseable += 1;
        return Ok(());
    };

    // Blob path is keyed by the same identity the store dedups on, so one
    // message shared across folders keeps exactly one body on disk.
    let key = dedup_key(&parsed.message);
    parsed.message.body_path = Some(ctx.blobs.put(ctx.account_id, &key, &message.raw)?);

    let location = Location {
        folder_id,
        uid: message.uid,
        flags: message.flags,
    };

    let (message_id, outcome) =
        ctx.store
            .upsert_message(ctx.account_id, &parsed.message, Some(&location))?;
    match outcome {
        Upsert::Inserted => {
            report.inserted += 1;
            // Classify only new messages: a deduplicated one already has a
            // verdict, and the same mail seen in a second folder has not
            // changed. The rules run on every message so that when the
            // model lands its verdicts can be compared against a baseline
            // that already exists for the whole mailbox — plan section 4.
            record_rules_verdict(ctx, message_id, &parsed);
        }
        Upsert::Deduplicated => report.deduplicated += 1,
    }
    Ok(())
}

/// Records the deterministic verdict for a freshly stored message.
///
/// A classification failure must never fail a sync — the mail is already
/// safely stored, and an unclassified message simply shows as Unknown. So this
/// logs and moves on rather than propagating.
fn record_rules_verdict(ctx: &Context<'_>, message_id: MessageId, parsed: &Parsed) {
    let classification = ctx.classifier.classify(&parsed.facts.as_message_facts());

    if let Err(err) = ctx.store.record_rules_verdict(
        message_id,
        classification.category.as_str(),
        classification.confidence,
        core_rules::RULES_VERSION,
    ) {
        tracing::warn!(message_id, %err, "failed to record rules verdict");
    }
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
