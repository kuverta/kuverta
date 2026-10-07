//! What the models cost, in tokens: counted as they answer, shown in the
//! settings.
//!
//! Every client [`Core::ai_for`] hands out carries a [`StoreMeter`], so each
//! request any job makes — the assistant, sorting, reading a scan, judging
//! urgency, a task — adds to a tally by day, job and model. Only counts are
//! kept: never what was asked or answered.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::{Core, Result};

/// Adds each request to the store's tally. Opens a connection of its own per
/// request: a request takes seconds, a connection microseconds, and this way
/// a meter can be carried off to any thread without a lock.
pub struct StoreMeter {
    pub(crate) db: PathBuf,
    pub(crate) task: String,
    pub(crate) provider: String,
    pub(crate) local: bool,
}

impl core_ai::Meter for StoreMeter {
    fn record(&self, model: &str, usage: core_ai::Usage) {
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        let recorded = core_store::Store::open(&self.db).and_then(|store| {
            store.record_ai_usage(
                &day,
                &self.task,
                &self.provider,
                model,
                self.local,
                usage.input_tokens,
                usage.output_tokens,
            )
        });
        if let Err(err) = recorded {
            tracing::warn!(%err, "token usage could not be recorded");
        }
    }
}

/// Tokens and requests, summed.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct UsageTotals {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl UsageTotals {
    fn add(&mut self, row: &core_store::UsageRow) {
        self.calls += row.calls;
        self.input_tokens += row.input_tokens;
        self.output_tokens += row.output_tokens;
    }
}

/// One model's or one job's share.
#[derive(Debug, Clone, Serialize)]
pub struct UsageShare {
    /// The model, or the job.
    pub name: String,
    /// For a model: where it runs.
    pub provider: Option<String>,
    pub local: bool,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// One day's sum.
#[derive(Debug, Clone, Serialize)]
pub struct UsageDay {
    pub day: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// What the settings show.
#[derive(Debug, Clone, Serialize)]
pub struct UsageStats {
    pub today: UsageTotals,
    pub this_month: UsageTotals,
    pub all_time: UsageTotals,
    /// Since the first of the month, which the shares are of.
    pub by_model: Vec<UsageShare>,
    pub by_task: Vec<UsageShare>,
    /// The last thirty days, oldest first, days without requests included.
    pub days: Vec<UsageDay>,
    /// The day counting began, if anything was counted.
    pub since: Option<String>,
}

impl Core {
    pub fn usage_stats(&self) -> Result<UsageStats> {
        Ok(summarise(
            &self.store.ai_usage(None)?,
            chrono::Local::now().date_naive(),
        ))
    }

    pub fn clear_usage_stats(&self) -> Result<()> {
        Ok(self.store.clear_ai_usage()?)
    }
}

fn summarise(rows: &[core_store::UsageRow], today: chrono::NaiveDate) -> UsageStats {
    let today_text = today.format("%Y-%m-%d").to_string();
    let month = today.format("%Y-%m-").to_string();
    let first_day = today - chrono::Duration::days(29);

    let mut stats = UsageStats {
        today: UsageTotals::default(),
        this_month: UsageTotals::default(),
        all_time: UsageTotals::default(),
        by_model: Vec::new(),
        by_task: Vec::new(),
        days: Vec::new(),
        since: rows.iter().map(|row| row.day.clone()).min(),
    };
    let mut models: BTreeMap<(String, String), UsageShare> = BTreeMap::new();
    let mut tasks: BTreeMap<String, UsageShare> = BTreeMap::new();
    let mut days: BTreeMap<String, UsageTotals> = (0..30)
        .map(|back| {
            (
                (first_day + chrono::Duration::days(back))
                    .format("%Y-%m-%d")
                    .to_string(),
                UsageTotals::default(),
            )
        })
        .collect();

    for row in rows {
        stats.all_time.add(row);
        if row.day == today_text {
            stats.today.add(row);
        }
        if let Some(day) = days.get_mut(&row.day) {
            day.add(row);
        }
        if !row.day.starts_with(&month) {
            continue;
        }
        stats.this_month.add(row);
        models
            .entry((row.model.clone(), row.provider.clone()))
            .or_insert_with(|| UsageShare {
                name: row.model.clone(),
                provider: Some(row.provider.clone()),
                local: row.local,
                totals: UsageTotals::default(),
            })
            .totals
            .add(row);
        tasks
            .entry(row.task.clone())
            .or_insert_with(|| UsageShare {
                name: row.task.clone(),
                provider: None,
                local: false,
                totals: UsageTotals::default(),
            })
            .totals
            .add(row);
    }

    let by_size = |a: &UsageShare, b: &UsageShare| {
        (b.totals.input_tokens + b.totals.output_tokens)
            .cmp(&(a.totals.input_tokens + a.totals.output_tokens))
    };
    stats.by_model = models.into_values().collect();
    stats.by_model.sort_by(by_size);
    stats.by_task = tasks.into_values().collect();
    stats.by_task.sort_by(by_size);
    stats.days = days
        .into_iter()
        .map(|(day, totals)| UsageDay { day, totals })
        .collect();
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: &str, task: &str, model: &str, input: u64, output: u64) -> core_store::UsageRow {
        core_store::UsageRow {
            day: day.into(),
            task: task.into(),
            provider: "Ollama".into(),
            model: model.into(),
            local: true,
            calls: 1,
            input_tokens: input,
            output_tokens: output,
        }
    }

    #[test]
    fn the_tally_is_summed_by_day_month_model_and_job() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        let stats = summarise(
            &[
                row("2026-09-30", "chat", "llama3.2:3b", 1000, 10),
                row("2026-10-01", "assistant", "qwen3:8b", 500, 50),
                row("2026-10-07", "assistant", "qwen3:8b", 200, 20),
                row("2026-10-07", "chat", "llama3.2:3b", 100, 1),
            ],
            today,
        );
        assert_eq!(stats.today.input_tokens, 300);
        assert_eq!(stats.today.calls, 2);
        assert_eq!(stats.this_month.input_tokens, 800);
        assert_eq!(stats.all_time.input_tokens, 1800);
        assert_eq!(stats.by_model[0].name, "qwen3:8b");
        assert_eq!(stats.by_model[0].totals.output_tokens, 70);
        assert_eq!(stats.by_task[0].name, "assistant");
        assert_eq!(stats.days.len(), 30);
        assert_eq!(stats.days.last().unwrap().day, "2026-10-07");
        assert_eq!(stats.days.last().unwrap().totals.input_tokens, 300);
        assert_eq!(stats.since.as_deref(), Some("2026-09-30"));
    }
}
