//! Folder synchronisation into the local store.

use core_store::model::{AccountId, Location};
use core_store::{dedup_key, Blobs, Store, Upsert};

use crate::client::ImapClient;
use crate::parse::parse_message;
use crate::ProtoError;

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub folders_synced: usize,
    /// Messages new to the store.
    pub inserted: usize,
    /// Messages already known — the same mail under another folder or a
    /// re-sync. On Gmail this is routinely large and is not a problem.
    pub deduplicated: usize,
    /// Bodies the parser could not make sense of at all.
    pub unparseable: usize,
    /// Folders whose UIDVALIDITY changed, forcing a rebuild.
    pub invalidated: usize,
}

/// Syncs every selectable folder on the account.
pub async fn sync_account(
    client: &mut ImapClient,
    store: &Store,
    blobs: &Blobs,
    account_id: AccountId,
) -> Result<SyncReport, ProtoError> {
    let mut report = SyncReport::default();

    for remote in client.folders().await? {
        if !remote.selectable {
            tracing::debug!(folder = %remote.name, "skipping \\Noselect folder");
            continue;
        }

        let folder_id =
            store.upsert_folder(account_id, &remote.name, remote.special_use.as_deref())?;
        let state = client.examine(&remote.name).await?;

        // UIDVALIDITY changing means the server has renumbered the folder and
        // every UID cached for it is meaningless. Anything else risks attaching
        // stored messages to the wrong mail.
        let known = store
            .folders(account_id)?
            .into_iter()
            .find(|f| f.id == folder_id)
            .and_then(|f| f.uid_validity);

        if let (Some(known), Some(current)) = (known, state.uid_validity) {
            if known != current {
                tracing::warn!(
                    folder = %remote.name, known, current,
                    "UIDVALIDITY changed, discarding cached UIDs for this folder"
                );
                store.invalidate_folder(folder_id)?;
                report.invalidated += 1;
            }
        }

        let after = store.max_uid(folder_id)?;
        let fetched = client.fetch_since(&state, after).await?;

        for message in fetched {
            let Some(mut parsed) = parse_message(&message.raw, message.size) else {
                tracing::warn!(folder = %remote.name, uid = message.uid, "could not parse message");
                report.unparseable += 1;
                continue;
            };

            // Blob path is keyed by the same identity the store dedups on, so
            // one message shared across folders keeps exactly one body on disk.
            let key = dedup_key(&parsed);
            parsed.body_path = Some(blobs.put(account_id, &key, &message.raw)?);

            let location = Location {
                folder_id,
                uid: message.uid,
                flags: message.flags,
            };

            match store.upsert_message(account_id, &parsed, Some(&location))? {
                (_, Upsert::Inserted) => report.inserted += 1,
                (_, Upsert::Deduplicated) => report.deduplicated += 1,
            }
        }

        store.set_folder_sync_state(
            folder_id,
            state.uid_validity,
            state.uid_next,
            state.highest_modseq,
        )?;
        report.folders_synced += 1;
    }

    Ok(report)
}
