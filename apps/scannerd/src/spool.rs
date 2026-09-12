//! Captures on disk, until Paperless has them.
//!
//! The §3.5 rule — queue the intent before acting, not after — applied to
//! paper. A photograph is written to the spool *before* any upload is
//! attempted and removed only once Paperless has confirmed it, so a crash, a
//! flat battery or a Wi-Fi drop between the two loses nothing and the retry
//! finds the file exactly where it was left.
//!
//! The spool is a directory of files, not a database. A Pi that has been
//! unplugged mid-write should be recoverable with `ls`, and a half-written
//! photograph should be obvious rather than a row claiming a file that is not
//! there.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Extension for a capture still being written.
///
/// Renamed into place once complete, because rename within a directory is
/// atomic and a copy is not: without it, a crash mid-write leaves a truncated
/// JPEG that looks exactly like a whole one.
const PARTIAL: &str = "partial";
const READY: &str = "jpg";

pub struct Spool {
    dir: PathBuf,
}

/// One capture waiting to be uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub path: PathBuf,
    /// Seconds since the epoch, from the filename rather than the filesystem —
    /// copying a spool between machines must not change when a letter arrived.
    pub captured_at: u64,
    /// How many uploads have been tried and failed.
    pub attempts: u32,
}

impl Spool {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("could not open the spool at {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A path to write a capture to, and the path to move it to afterwards.
    ///
    /// The camera writes to `.partial`; [`Spool::commit`] renames it. Nothing
    /// reads a `.partial` file, so an interrupted capture is skipped rather
    /// than uploaded half-formed.
    pub fn reserve(&self) -> (PathBuf, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        // The counter disambiguates two captures within one second, which
        // happens when a page is replaced quickly.
        let unique = std::process::id();
        let stem = format!("{stamp}-{unique}");
        (
            self.dir.join(format!("{stem}.{PARTIAL}")),
            self.dir.join(format!("{stem}.{READY}")),
        )
    }

    pub fn commit(&self, partial: &Path, ready: &Path) -> Result<()> {
        fs::rename(partial, ready).with_context(|| {
            format!(
                "could not move {} into the spool as {}",
                partial.display(),
                ready.display()
            )
        })
    }

    /// Everything waiting, oldest first.
    ///
    /// Oldest first because post is read in the order it arrived, and because
    /// a spool that drains newest-first leaves the oldest item to be retried
    /// forever behind a queue that keeps growing.
    pub fn pending(&self) -> Result<Vec<Pending>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(READY) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            found.push(Pending {
                captured_at: stem
                    .split('-')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                attempts: self.attempts(&path),
                path,
            });
        }
        found.sort_by(|a, b| a.captured_at.cmp(&b.captured_at).then(a.path.cmp(&b.path)));
        Ok(found)
    }

    /// Forgets a capture, once Paperless has it.
    pub fn done(&self, pending: &Pending) -> Result<()> {
        let _ = fs::remove_file(self.attempts_path(&pending.path));
        fs::remove_file(&pending.path)
            .with_context(|| format!("could not clear {}", pending.path.display()))
    }

    /// Records that an upload was tried and failed.
    ///
    /// Counted from what is on disk rather than from the caller's `Pending`,
    /// which may have been read some time ago. Taking the caller's word would
    /// mean two failures against one stale handle both recording "attempt 1",
    /// and a backoff that never backs off.
    ///
    /// Kept beside the capture as a sidecar rather than in the filename, so a
    /// retry never renames the thing it is retrying — a rename that failed
    /// halfway would be a capture that exists twice or not at all.
    pub fn failed(&self, pending: &Pending) -> Result<u32> {
        let attempts = self.attempts(&pending.path).saturating_add(1);
        fs::write(self.attempts_path(&pending.path), attempts.to_string())?;
        Ok(attempts)
    }

    fn attempts_path(&self, capture: &Path) -> PathBuf {
        capture.with_extension("attempts")
    }

    fn attempts(&self, capture: &Path) -> u32 {
        fs::read_to_string(self.attempts_path(capture))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }
}

/// How long to wait before retrying, given how many attempts have failed.
///
/// Backs off to a ceiling rather than growing without bound: a Pi that has
/// been offline overnight should try again within the minute once the network
/// returns, not in four hours because it gave up politely.
pub fn retry_delay(attempts: u32) -> std::time::Duration {
    let seconds = match attempts {
        0 => 0,
        1 => 5,
        2 => 15,
        3 => 60,
        _ => 300,
    };
    std::time::Duration::from_secs(seconds)
}
