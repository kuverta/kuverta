//! Draining the mutation queue onto the server.
//!
//! Stage 2 of the write capability (docs/implementation-plan.md section 1a).
//! The queue in `core-store` records what the user meant; this decides whether
//! that intent still makes sense against the server as it is now, and applies
//! it only if it does.
//!
//! The rule the whole module exists to enforce: **never act on a UID without
//! first proving it still refers to the message the user acted on.** A UID is
//! stable within a UIDVALIDITY only in the sense that the server will not
//! reissue it — the message can be moved or deleted by another client at any
//! time, and archiving whatever now happens to sit at UID 412 is exactly the
//! failure the read-only design used to make impossible. Three things are
//! checked before anything is written: the folder still has the UIDVALIDITY the
//! operation was queued under, there is still a message at the UID, and its
//! Message-ID is the one expected.
//!
//! Failure is never silent and never destructive. An operation that cannot be
//! verified is failed with a reason and left for the user to look at; one that
//! turns out to be moot is settled as obsolete; a network error keeps it
//! pending for the next attempt.

use std::collections::HashMap;

use core_store::model::{AccountId, Operation, OperationKind, OperationState};
use core_store::Store;

use crate::client::{ImapClient, MoveOutcome, UidProbe};
use crate::ProtoError;

#[derive(Debug, Default, Clone)]
pub struct FlushReport {
    /// Operations the server accepted.
    pub applied: usize,
    /// Operations that no longer apply: the message had already left, or was
    /// already in the state asked for. Not errors.
    pub obsolete: usize,
    /// Operations refused because the server no longer matches what was
    /// recorded. These are left settled as failed, with the reason.
    pub conflicted: usize,
    /// Operations left pending after a retryable failure.
    pub retryable: usize,
    /// Moves the server could not perform atomically, leaving a flagged copy
    /// behind. See [`ImapClient::uid_move`].
    pub non_atomic_moves: usize,
}

impl FlushReport {
    pub fn is_empty(&self) -> bool {
        self.applied == 0 && self.obsolete == 0 && self.conflicted == 0 && self.retryable == 0
    }
}

/// Why an operation was not applied.
enum Outcome {
    Applied(Option<MoveOutcome>),
    /// No longer applicable. Settled, not retried.
    Obsolete(String),
    /// The server does not match what was recorded. Settled as failed.
    Conflict(String),
}

/// Applies every operation whose undo window has elapsed.
///
/// Operations run oldest first and are settled one at a time, so an
/// interruption leaves the queue accurate rather than ambiguous: everything
/// before the break is recorded as done, everything after is still pending.
///
/// This does not update the local store's idea of where messages are. The sync
/// pass that follows does that, by observing the server — which is the same
/// path that reconciles changes made from any other client, and therefore the
/// one already trusted to get it right.
pub async fn flush_operations(
    client: &mut ImapClient,
    store: &Store,
    account_id: AccountId,
    now_utc: i64,
) -> Result<FlushReport, ProtoError> {
    let mut report = FlushReport::default();
    let due = store.due_operations(account_id, now_utc)?;
    if due.is_empty() {
        return Ok(report);
    }

    // Folder names are resolved once: the queue holds ids, IMAP takes names,
    // and a folder that has since been deleted locally is a conflict rather
    // than a lookup failure.
    let mut folder_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut selected: Option<String> = None;

    for op in due {
        let name = match folder_names.entry(op.source_folder_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
            std::collections::hash_map::Entry::Vacant(entry) => entry
                .insert(store.folder(op.source_folder_id)?.map(|f| f.name))
                .clone(),
        };

        let Some(name) = name else {
            settle(
                store,
                &op,
                Outcome::Conflict("its source folder is no longer in the store".into()),
                &mut report,
            )?;
            continue;
        };

        // Re-SELECT only when the folder changes. Operations arrive grouped by
        // nothing in particular, but consecutive ones on the same folder are
        // the common case.
        if selected.as_deref() != Some(name.as_str()) {
            let state = client.select(&name).await?;
            selected = Some(name.clone());

            if let (Some(expected), Some(actual)) = (op.source_uid_validity, state.uid_validity) {
                if expected != actual {
                    // Every UID queued against this folder is meaningless now.
                    // Failing one at a time is deliberate: the next operation
                    // re-checks and fails for itself, which keeps the reason
                    // attached to each rather than inferred.
                    settle(
                        store,
                        &op,
                        Outcome::Conflict(format!(
                            "{name} was renumbered by the server (UIDVALIDITY {expected} -> \
                             {actual}), so the recorded UID no longer identifies anything"
                        )),
                        &mut report,
                    )?;
                    continue;
                }
            }
        }

        let outcome = apply(client, &op, &name).await;
        match outcome {
            Ok(outcome) => settle(store, &op, outcome, &mut report)?,
            // A transport error says nothing about whether the operation is
            // sound, only that it did not get through. It stays pending.
            Err(err) => {
                tracing::warn!(op = op.id, error = %err, "operation could not be sent");
                store.settle_operation(op.id, OperationState::Pending, Some(&err.to_string()))?;
                report.retryable += 1;
                return Ok(report);
            }
        }
    }

    Ok(report)
}

/// Verifies the operation still makes sense, then performs it.
async fn apply(
    client: &mut ImapClient,
    op: &Operation,
    folder: &str,
) -> Result<Outcome, ProtoError> {
    let found = match client.uid_probe(op.source_uid).await? {
        // Somebody else moved or deleted it. For a move that is the intent
        // already satisfied by other means; for a flag change there is nothing
        // left to flag. Either way there is nothing to do and nothing wrong.
        UidProbe::Vacant => {
            return Ok(Outcome::Obsolete(format!(
                "no message at UID {} in {folder} any more",
                op.source_uid
            )));
        }
        UidProbe::Present { message_id } => message_id,
    };

    // The dangerous case, and the reason the expected id is recorded at all.
    //
    // Both sides are compared including their absence: `expect_message_id` is
    // `None` precisely when the queued message had no `Message-ID` header, so
    // finding one there now is as much a mismatch as finding the wrong one.
    if found.as_deref() != op.expect_message_id.as_deref() {
        let describe = |id: Option<&str>| match id {
            Some(id) => format!("<{id}>"),
            None => "a message with no Message-ID".to_string(),
        };
        return Ok(Outcome::Conflict(format!(
            "UID {} in {folder} now holds {}, not {}",
            op.source_uid,
            describe(found.as_deref()),
            describe(op.expect_message_id.as_deref()),
        )));
    }

    match &op.kind {
        OperationKind::Move { target_folder } => {
            if target_folder == folder {
                return Ok(Outcome::Obsolete(format!("already in {folder}")));
            }
            let outcome = client.uid_move(op.source_uid, target_folder).await?;
            Ok(Outcome::Applied(Some(outcome)))
        }
        OperationKind::Flag { flag, set } => {
            client.uid_store_flag(op.source_uid, flag, *set).await?;
            Ok(Outcome::Applied(None))
        }
    }
}

fn settle(
    store: &Store,
    op: &Operation,
    outcome: Outcome,
    report: &mut FlushReport,
) -> Result<(), ProtoError> {
    match outcome {
        Outcome::Applied(how) => {
            if how == Some(MoveOutcome::CopiedAndFlagged) {
                report.non_atomic_moves += 1;
            }
            store.settle_operation(op.id, OperationState::Done, None)?;
            report.applied += 1;
        }
        Outcome::Obsolete(reason) => {
            tracing::info!(op = op.id, %reason, "operation no longer applies");
            store.settle_operation(op.id, OperationState::Obsolete, Some(&reason))?;
            report.obsolete += 1;
        }
        Outcome::Conflict(reason) => {
            tracing::warn!(op = op.id, %reason, "refusing to apply operation");
            store.settle_operation(op.id, OperationState::Failed, Some(&reason))?;
            report.conflicted += 1;
        }
    }
    Ok(())
}
