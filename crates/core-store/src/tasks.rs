//! Tasks: standing jobs the assistant does to mail as it arrives.
//!
//! A task is a question and an answer. The question is a smart mailbox's rules
//! — which mail it is about. The answer is what to do with each message: move
//! it, archive it, file it, or have a model decide or draft a reply. The store
//! keeps the task (its action as JSON, which core-rpc owns), which messages it
//! has already dealt with, so each is dealt with once, and **proposals**: what
//! a task — or the assistant in the chat — would do, waiting for a person to
//! say yes, for tasks that ask first and for anything that would send mail.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};

use crate::model::{AccountId, MessageId};
use crate::smart::{SmartQuery, SmartRule};
use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTask {
    pub id: i64,
    pub account_id: AccountId,
    pub name: String,
    pub query: SmartQuery,
    /// JSON, as core-rpc wrote it.
    pub action: String,
    /// Propose rather than do: each action waits for a person to approve it.
    pub review: bool,
    pub enabled: bool,
    pub created_at: i64,
    pub last_run_at: Option<i64>,
    pub last_summary: Option<String>,
}

/// Something a task or the assistant would do, waiting for a yes or a no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: i64,
    pub task_id: Option<i64>,
    pub account_id: AccountId,
    pub message_id: Option<MessageId>,
    /// `move`, `archive`, `trash`, `mark_read`, `file` or `reply`.
    pub kind: String,
    /// The folder or category, for the kinds that take one.
    pub target: Option<String>,
    /// The reply's text, for `reply`.
    pub body: Option<String>,
    pub reason: Option<String>,
    /// `pending`, `done`, `rejected` or `failed`.
    pub state: String,
    pub detail: Option<String>,
    pub created_at: i64,
}

/// A proposal to record.
#[derive(Debug, Clone, Default)]
pub struct NewProposal {
    pub task_id: Option<i64>,
    pub account_id: AccountId,
    pub message_id: Option<MessageId>,
    pub kind: String,
    pub target: Option<String>,
    pub body: Option<String>,
    pub reason: Option<String>,
}

const TASK_COLUMNS: &str = "id, account_id, name, match_all, rules, action, review, enabled, \
                            created_at, last_run_at, last_summary";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTask> {
    let rules: String = row.get(4)?;
    Ok(StoredTask {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        query: SmartQuery {
            match_all: row.get(3)?,
            rules: serde_json::from_str::<Vec<SmartRule>>(&rules).unwrap_or_default(),
        },
        action: row.get(5)?,
        review: row.get(6)?,
        enabled: row.get(7)?,
        created_at: row.get(8)?,
        last_run_at: row.get(9)?,
        last_summary: row.get(10)?,
    })
}

const PROPOSAL_COLUMNS: &str = "id, task_id, account_id, message_id, kind, target, body, reason, \
                                state, detail, created_at";

fn row_to_proposal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Proposal> {
    Ok(Proposal {
        id: row.get(0)?,
        task_id: row.get(1)?,
        account_id: row.get(2)?,
        message_id: row.get(3)?,
        kind: row.get(4)?,
        target: row.get(5)?,
        body: row.get(6)?,
        reason: row.get(7)?,
        state: row.get(8)?,
        detail: row.get(9)?,
        created_at: row.get(10)?,
    })
}

impl Store {
    pub fn tasks(&self, account_id: AccountId) -> Result<Vec<StoredTask>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM task WHERE account_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![account_id], row_to_task)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn task(&self, id: i64) -> Result<Option<StoredTask>> {
        self.conn
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM task WHERE id = ?1"),
                params![id],
                row_to_task,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Adds a task, or replaces the one with `id`.
    #[allow(clippy::too_many_arguments)]
    pub fn save_task(
        &self,
        id: Option<i64>,
        account_id: AccountId,
        name: &str,
        query: &SmartQuery,
        action: &str,
        review: bool,
        enabled: bool,
    ) -> Result<i64> {
        let rules = serde_json::to_string(&query.rules).unwrap_or_else(|_| "[]".into());
        match id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE task SET name = ?2, match_all = ?3, rules = ?4, action = ?5,
                            review = ?6, enabled = ?7
                     WHERE id = ?1",
                    params![id, name, query.match_all, rules, action, review, enabled],
                )?;
                Ok(id)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO task (account_id, name, match_all, rules, action, review, enabled, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![account_id, name, query.match_all, rules, action, review, enabled, crate::now()],
                )?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_task(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM task WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_task_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE task SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(())
    }

    pub fn task_ran(&self, id: i64, summary: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE task SET last_run_at = ?2, last_summary = ?3 WHERE id = ?1",
            params![id, crate::now(), summary],
        )?;
        Ok(())
    }

    /// The messages a task has already dealt with.
    pub fn task_seen(&self, task_id: i64) -> Result<HashSet<MessageId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT message_id FROM task_seen WHERE task_id = ?1")?;
        let rows = stmt.query_map(params![task_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(Into::into)
    }

    /// Records that a task has dealt with these messages, so it never deals
    /// with them again — not even after a person undoes what it did.
    pub fn mark_task_seen(&self, task_id: i64, ids: &[MessageId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx
                .prepare("INSERT OR IGNORE INTO task_seen (task_id, message_id) VALUES (?1, ?2)")?;
            for id in ids {
                stmt.execute(params![task_id, id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn add_proposal(&self, proposal: &NewProposal) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO task_proposal
                 (task_id, account_id, message_id, kind, target, body, reason, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
            params![
                proposal.task_id,
                proposal.account_id,
                proposal.message_id,
                proposal.kind,
                proposal.target,
                proposal.body,
                proposal.reason,
                crate::now()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn proposal(&self, id: i64) -> Result<Option<Proposal>> {
        self.conn
            .query_row(
                &format!("SELECT {PROPOSAL_COLUMNS} FROM task_proposal WHERE id = ?1"),
                params![id],
                row_to_proposal,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Proposals still waiting on an account, oldest first.
    pub fn pending_proposals(&self, account_id: AccountId) -> Result<Vec<Proposal>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM task_proposal
             WHERE account_id = ?1 AND state = 'pending'
             ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![account_id], row_to_proposal)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Settles a proposal. Only a pending one: approving twice must not act twice.
    pub fn settle_proposal(&self, id: i64, state: &str, detail: Option<&str>) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE task_proposal SET state = ?2, detail = ?3 WHERE id = ?1 AND state = 'pending'",
            params![id, state, detail],
        )?;
        Ok(changed == 1)
    }

    /// How many proposals each task has waiting.
    pub fn pending_by_task(&self, account_id: AccountId) -> Result<Vec<(Option<i64>, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, COUNT(*) FROM task_proposal
             WHERE account_id = ?1 AND state = 'pending' GROUP BY task_id",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
