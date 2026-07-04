//! Sending later.
//!
//! A scheduled message is checked when it is scheduled — built exactly as
//! sending it would build it — so a typo in an address is refused now, while
//! the person is still looking, rather than failing at eight in the morning
//! with nobody watching. Then the draft is kept as it was described, and sent
//! through the same path as any other message when its time comes.
//!
//! Only while kuverta is running: nothing here asks a server to hold the mail,
//! because no IMAP or SMTP server offers that portably. A message whose time
//! passed while the app was closed goes out the next time it opens, and the
//! outbox says when it was due.

use serde::{Deserialize, Serialize};

use crate::session::{DraftInput, SentSummary, Session};
use crate::{Core, Result, RpcError};

/// A message in the outbox, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxView {
    pub id: i64,
    pub account_email: String,
    pub subject: String,
    pub recipients: String,
    pub send_at: i64,
    /// `scheduled`, `sending` or `failed`.
    pub state: String,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// What became of one due message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxSent {
    pub id: i64,
    pub subject: String,
    pub sent: Option<SentSummary>,
    pub error: Option<String>,
}

impl Core {
    pub fn outbox(&self) -> Result<Vec<OutboxView>> {
        let emails: std::collections::HashMap<i64, String> = self
            .store()
            .accounts()?
            .into_iter()
            .map(|account| (account.id, account.email))
            .collect();
        Ok(self
            .store()
            .outbox()?
            .into_iter()
            .map(|entry| OutboxView {
                account_email: emails.get(&entry.account_id).cloned().unwrap_or_default(),
                id: entry.id,
                subject: entry.subject,
                recipients: entry.recipients,
                send_at: entry.send_at,
                state: entry.state,
                last_error: entry.last_error,
                created_at: entry.created_at,
            })
            .collect())
    }

    pub fn cancel_scheduled(&self, id: i64) -> Result<()> {
        if self.store().cancel_outbox(id)? {
            Ok(())
        } else {
            Err(RpcError::Rejected(
                "that message is being sent or has gone, so it cannot be cancelled".into(),
            ))
        }
    }

    pub fn reschedule(&self, id: i64, send_at: i64) -> Result<()> {
        if self.store().reschedule_outbox(id, send_at)? {
            Ok(())
        } else {
            Err(RpcError::Rejected(
                "that message is being sent or has gone, so it cannot be moved".into(),
            ))
        }
    }

    /// The draft of a waiting message, to open it in compose again. Taking it
    /// out of the outbox is the caller's next step, once compose has it.
    pub fn scheduled_draft(&self, id: i64) -> Result<(String, DraftInput)> {
        let entry = self
            .store()
            .outbox_entry(id)?
            .ok_or_else(|| RpcError::Rejected(format!("no scheduled message {id}")))?;
        let account = self
            .store()
            .accounts()?
            .into_iter()
            .find(|account| account.id == entry.account_id)
            .ok_or_else(|| RpcError::UnknownAccount(entry.account_id.to_string()))?;
        let draft = serde_json::from_str(&entry.draft)
            .map_err(|err| RpcError::Rejected(format!("the stored draft is unreadable: {err}")))?;
        Ok((account.email, draft))
    }
}

impl Session {
    /// Puts a message in the outbox, due at `send_at` (Unix seconds).
    ///
    /// Built first, the way sending would build it, so anything that would
    /// stop it going is said now.
    pub fn schedule(&self, email: &str, draft: &DraftInput, send_at: i64) -> Result<i64> {
        let preview = self.preview(email, draft)?;
        let (store, _) = self.open_store()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;
        if account.smtp.is_none() {
            return Err(RpcError::Rejected(format!(
                "{email} has no outgoing server, so it cannot send — add one in settings"
            )));
        }
        let json = serde_json::to_string(draft)
            .map_err(|err| RpcError::Rejected(format!("could not keep the draft: {err}")))?;
        let id = store.schedule_send(
            account.id,
            &json,
            &preview.subject,
            &preview.recipients.join(", "),
            send_at,
        )?;
        tracing::info!(id, send_at, "scheduled a message");
        Ok(id)
    }

    /// Sends everything that is due, one at a time.
    ///
    /// Each message is claimed before it is sent, and a claim is never
    /// retried on its own: a message that fails is left `failed` with the
    /// reason, for a person to reschedule or cancel. Sending twice is the one
    /// mistake here that cannot be taken back.
    pub async fn send_due(&self) -> Result<Vec<OutboxSent>> {
        let due = {
            let (store, _) = self.open_store()?;
            store.due_outbox(crate::now_utc())?
        };
        let mut results = Vec::new();
        for entry in due {
            let (email, draft) = {
                let (store, _) = self.open_store()?;
                if !store.claim_outbox(entry.id, crate::now_utc())? {
                    continue;
                }
                let email = store
                    .accounts()?
                    .into_iter()
                    .find(|account| account.id == entry.account_id)
                    .map(|account| account.email);
                let draft = serde_json::from_str::<DraftInput>(&entry.draft);
                match (email, draft) {
                    (Some(email), Ok(draft)) => (email, draft),
                    (None, _) => {
                        store.outbox_failed(entry.id, "its account has been removed")?;
                        continue;
                    }
                    (_, Err(err)) => {
                        store.outbox_failed(
                            entry.id,
                            &format!("the stored draft is unreadable: {err}"),
                        )?;
                        continue;
                    }
                }
            };

            tracing::info!(id = entry.id, "sending a scheduled message");
            let outcome = self.send(&email, &draft, true).await;
            let (store, _) = self.open_store()?;
            match outcome {
                Ok(sent) => {
                    // It has gone, so this must be written down: a row left
                    // `sending` would read, after a restart, as "may not have
                    // gone". A busy store gets a few more tries.
                    let mut recorded = store.outbox_sent(entry.id);
                    for _ in 0..5 {
                        if recorded.is_ok() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        recorded = store.outbox_sent(entry.id);
                    }
                    if let Err(err) = recorded {
                        tracing::error!(id = entry.id, %err, "sent, but could not record it");
                    }
                    results.push(OutboxSent {
                        id: entry.id,
                        subject: entry.subject,
                        sent: Some(sent),
                        error: None,
                    });
                }
                Err(err) => {
                    tracing::warn!(id = entry.id, %err, "a scheduled message did not go");
                    // A network failure can come after the server took the
                    // message, so a failure is never retried on its own.
                    store.outbox_failed(entry.id, &err.to_string())?;
                    results.push(OutboxSent {
                        id: entry.id,
                        subject: entry.subject,
                        sent: None,
                        error: Some(err.to_string()),
                    });
                }
            }
        }
        Ok(results)
    }
}
