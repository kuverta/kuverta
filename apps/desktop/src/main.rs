//! Spike: does a Tauri v2 webview render a mail-sized message list smoothly?
//!
//! This is a risk test, not the beginning of the UI. The plan (§6) says to run
//! it before writing any UI chrome, because if the answer is no, the whole
//! "Rust core + web shell" bet is wrong and it is far cheaper to learn that in
//! week one than in month four.
//!
//! It measures two things:
//!
//! 1. **IPC cost** — how long it takes to move N rows across the Rust/JS
//!    bridge. Tauri serialises commands as JSON, which is the known bottleneck
//!    and the thing most likely to bite a 100k-message mailbox.
//! 2. **Scroll frame times** — p50/p95/worst during a programmatic scroll of
//!    the whole list, plus how many frames missed 60fps.
//!
//! Run with `make spike` (or `cargo run -p fuckmail-desktop`). Set
//! `FUCKMAIL_SPIKE_ROWS` to change the row count and `FUCKMAIL_SPIKE_AUTOEXIT=1`
//! to close the window once the numbers are in.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};

/// One row as the list renders it. Deliberately the same shape the real triage
/// list would need, so the measured payload size is representative.
#[derive(Debug, Clone, Serialize)]
struct Row {
    id: i64,
    /// Pre-formatted; date formatting in JS for every row would measure the
    /// wrong thing.
    date: String,
    sender: String,
    subject: String,
    list_id: Option<String>,
    has_attachments: bool,
    unread: bool,
}

#[derive(Debug, Deserialize)]
struct Metrics {
    rows: usize,
    ipc_ms: f64,
    payload_bytes: usize,
    first_paint_ms: f64,
    frames: usize,
    p50_ms: f64,
    p95_ms: f64,
    worst_ms: f64,
    /// The display's frame budget, taken as the median frame time rather than
    /// assumed. WKWebView clamps `performance.now()` to 1ms, so a real 60fps
    /// frame reports as 17ms; a hardcoded 16.7ms threshold would call every
    /// healthy frame late.
    budget_ms: f64,
    /// Frames that took at least 1.5 budgets — i.e. actually missed a vsync.
    dropped: usize,
    achieved_fps: f64,
    elapsed_ms: f64,
    dom_nodes: usize,
}

const SENDERS: &[&str] = &[
    "Anna Weber",
    "Buchhaltung",
    "Stadtwerke München",
    "Hosting AG",
    "Rust Weekly",
    "Shop Deals",
    "Tom Fisher",
    "Legacy Cron",
    "DHL Paket",
    "Finanzamt München",
    "GitHub",
    "Sparkasse",
    "Dr. Schneider",
    "Team Standup",
    "noreply@portal.example",
];

const SUBJECTS: &[&str] = &[
    "Re: Termin nächste Woche",
    "Ihre Rechnung 4471182 für September 2026",
    "Abschlagszahlung für März wurde geändert",
    "Rust Weekly #612 — async traits everywhere",
    "20% off everything this week only",
    "Re: Q3 roadmap",
    "nightly backup completed",
    "Ihre Sendung wurde zugestellt",
    "Bescheid über Einkommensteuer 2025",
    "[fuckmail] CI failed on main",
    "Kontoauszug verfügbar",
    "Terminbestätigung 14:30",
];

/// Generates a representative list. Synthetic rather than read from the store
/// because the question here is render cost, and the store is already known to
/// return 50k rows in single-digit milliseconds.
#[tauri::command]
fn load_rows(count: usize) -> Vec<Row> {
    (0..count)
        .map(|i| {
            let day = 1 + (i % 28);
            let month = 1 + (i % 12);
            let hour = i % 24;
            let minute = (i * 7) % 60;
            Row {
                id: i as i64,
                date: format!("2026-{month:02}-{day:02} {hour:02}:{minute:02}"),
                sender: SENDERS[i % SENDERS.len()].to_string(),
                subject: format!("{} #{}", SUBJECTS[i % SUBJECTS.len()], i),
                list_id: (i % 5 == 0).then(|| "news.rustweekly.example".to_string()),
                has_attachments: i % 11 == 0,
                unread: i % 3 == 0,
            }
        })
        .collect()
}

#[tauri::command]
fn spike_config() -> serde_json::Value {
    let rows: usize = std::env::var("FUCKMAIL_SPIKE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000);
    let autoexit = std::env::var_os("FUCKMAIL_SPIKE_AUTOEXIT").is_some();
    serde_json::json!({ "rows": rows, "autoexit": autoexit })
}

/// Receives the measurements and prints a verdict to the terminal, so the spike
/// produces a number rather than an impression.
#[tauri::command]
fn report(metrics: Metrics, app: tauri::AppHandle) {
    let Metrics {
        rows,
        ipc_ms,
        payload_bytes,
        first_paint_ms,
        frames,
        p50_ms,
        p95_ms,
        worst_ms,
        budget_ms,
        dropped,
        achieved_fps,
        elapsed_ms,
        dom_nodes,
    } = metrics;

    let mib = payload_bytes as f64 / (1024.0 * 1024.0);
    let dropped_pct = 100.0 * dropped as f64 / frames.max(1) as f64;
    let refresh_hz = 1000.0 / budget_ms.max(0.001);

    println!("\n=== Tauri v2 virtualized list spike ===");
    println!("rows                {rows}");
    println!("IPC (Rust -> JS)    {ipc_ms:.0} ms for {mib:.1} MiB of JSON");
    println!("first paint         {first_paint_ms:.0} ms");
    println!("DOM nodes in list   {dom_nodes}  (virtualized, not {rows})");
    println!(
        "--- scroll, {frames} frames over {:.1}s ---",
        elapsed_ms / 1000.0
    );
    println!("display refresh     ~{refresh_hz:.0} Hz (budget {budget_ms:.0} ms)");
    println!("achieved            {achieved_fps:.1} fps");
    println!("p50 / p95 / worst   {p50_ms:.0} / {p95_ms:.0} / {worst_ms:.0} ms");
    println!("dropped frames      {dropped} ({dropped_pct:.1}%)   [>= 1.5x budget]");
    println!("note                WKWebView clamps timers to 1ms; treat these as +/-1ms");

    // Judge on dropped frames, not on absolute milliseconds: what a user
    // perceives as jank is a missed vsync, and the 1ms timer clamp makes exact
    // millisecond thresholds meaningless.
    let verdict = if dropped_pct <= 2.0 {
        "PASS — holds the display's refresh rate. The Rust core + web shell bet is sound."
    } else if dropped_pct <= 10.0 {
        "MARGINAL — occasional visible hitches. Tune before building on it."
    } else {
        "FAIL — visibly janky. Reconsider the UI stack before month 4."
    };
    println!("\nverdict: {verdict}\n");

    if std::env::var_os("FUCKMAIL_SPIKE_AUTOEXIT").is_some() {
        app.exit(0);
    }
}

/// Reports a benchmark that could not complete, so a hung run says why.
#[tauri::command]
fn report_failure(message: String, app: tauri::AppHandle) {
    eprintln!("\n=== Tauri v2 virtualized list spike ===");
    eprintln!("the run did not finish: {message}");
    eprintln!("keep the window frontmost and the display awake, then retry.\n");
    if std::env::var_os("FUCKMAIL_SPIKE_AUTOEXIT").is_some() {
        app.exit(1);
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_rows,
            spike_config,
            report,
            report_failure
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the spike window");
}
