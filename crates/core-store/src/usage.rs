//! What the models cost, in tokens, as a tally per day, job and model.

use rusqlite::params;

use crate::{Result, Store};

/// One line of the tally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    /// The local date, `YYYY-MM-DD`.
    pub day: String,
    /// The job that asked: `assistant`, `chat`, `vision`, …
    pub task: String,
    /// The provider's name as the person gave it.
    pub provider: String,
    pub model: String,
    /// Whether it ran on this computer.
    pub local: bool,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Store {
    /// Adds one request to the day's tally.
    #[allow(clippy::too_many_arguments)]
    pub fn record_ai_usage(
        &self,
        day: &str,
        task: &str,
        provider: &str,
        model: &str,
        local: bool,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_usage (day, task, provider, model, local, calls, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)
             ON CONFLICT (day, task, provider, model) DO UPDATE SET
                 calls = calls + 1,
                 input_tokens = input_tokens + excluded.input_tokens,
                 output_tokens = output_tokens + excluded.output_tokens,
                 local = excluded.local",
            params![
                day,
                task,
                provider,
                model,
                local,
                input_tokens as i64,
                output_tokens as i64
            ],
        )?;
        Ok(())
    }

    /// The tally from `since` (a `YYYY-MM-DD`, inclusive) on, or all of it.
    pub fn ai_usage(&self, since: Option<&str>) -> Result<Vec<UsageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT day, task, provider, model, local, calls, input_tokens, output_tokens
               FROM ai_usage WHERE ?1 IS NULL OR day >= ?1
              ORDER BY day, task, provider, model",
        )?;
        let rows = stmt.query_map(params![since], |row| {
            Ok(UsageRow {
                day: row.get(0)?,
                task: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                local: row.get(4)?,
                calls: row.get::<_, i64>(5)? as u64,
                input_tokens: row.get::<_, i64>(6)? as u64,
                output_tokens: row.get::<_, i64>(7)? as u64,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Starts counting again from nothing.
    pub fn clear_ai_usage(&self) -> Result<()> {
        self.conn.execute("DELETE FROM ai_usage", [])?;
        Ok(())
    }
}
