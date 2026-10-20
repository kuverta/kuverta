//! Optional presentation groups. Never merge stored messages or IMAP state.
use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::functions::FunctionFlags;

use crate::{normalize_message_id, AccountId, Result, Store};

pub(crate) struct ThreadCache {
    accounts: Vec<AccountId>,
    changes: u64,
    version: i64,
}

fn subject_key(subject: &str) -> String {
    let mut subject = subject.trim();
    while let Some((prefix, rest)) = subject.split_once(':') {
        if !["re", "aw", "fw", "fwd", "wg"].contains(&prefix.trim().to_ascii_lowercase().as_str()) {
            break;
        }
        subject = rest.trim();
    }
    subject
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

fn join(parents: &mut [usize], a: usize, b: usize) {
    let a = root(parents, a);
    let b = root(parents, b);
    parents[a.max(b)] = a.min(b);
}

impl Store {
    /// Cache only header identities. Changes in this connection or another sync
    /// connection invalidate it. Bodies never cross the list bridge to group mail.
    pub(crate) fn prepare_threads(&self, accounts: &[AccountId]) -> Result<()> {
        let mut accounts = accounts.to_vec();
        accounts.sort_unstable();
        accounts.dedup();
        let changes = self.conn.total_changes();
        let version: i64 = self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        if self.thread_cache.borrow().as_ref().is_some_and(|cache| {
            cache.accounts == accounts && cache.changes == changes && cache.version == version
        }) {
            return Ok(());
        }
        let marks = vec!["?"; accounts.len()].join(",");
        let mut statement = self.conn.prepare(&format!(
            "SELECT id, account_id, subject, rfc822_message_id, in_reply_to
             FROM message WHERE account_id IN ({marks}) ORDER BY id"
        ))?;
        let headers = statement
            .query_map(rusqlite::params_from_iter(&accounts), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut parents: Vec<usize> = (0..headers.len()).collect();
        let mut subjects = HashMap::new();
        let mut references = HashMap::new();
        for (index, (_, account, subject, message_id, reply)) in headers.iter().enumerate() {
            let subject = subject_key(subject.as_deref().unwrap_or_default());
            // Empty subjects alone are not a conversation.
            if !subject.is_empty() {
                if let Some(previous) = subjects.insert((*account, subject), index) {
                    join(&mut parents, index, previous);
                }
            }
            // An explicit reply stays together even when its subject changed.
            // Sharing a missing parent also groups replies; cycles are harmless.
            for id in [message_id, reply].into_iter().flatten() {
                let id = normalize_message_id(id);
                if !id.is_empty() {
                    if let Some(previous) = references.insert((*account, id.to_string()), index) {
                        join(&mut parents, index, previous);
                    }
                }
            }
        }
        let keys: HashMap<i64, i64> = headers
            .iter()
            .enumerate()
            .map(|(index, header)| (header.0, headers[root(&mut parents, index)].0))
            .collect();
        let keys = Arc::new(keys);
        self.conn.create_scalar_function(
            "kuverta_thread",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            move |ctx| {
                let id: i64 = ctx.get(0)?;
                Ok(*keys.get(&id).unwrap_or(&id))
            },
        )?;
        *self.thread_cache.borrow_mut() = Some(ThreadCache {
            accounts,
            changes,
            version,
        });
        Ok(())
    }
}
