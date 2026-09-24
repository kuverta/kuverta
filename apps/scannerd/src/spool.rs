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
//!
//! When pages are collected into letters, they wait in `open/` until the
//! letter is closed, and only the finished PDF joins the queue.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

/// Extension for a capture still being written.
///
/// Renamed into place once complete, because rename within a directory is
/// atomic and a copy is not: without it, a crash mid-write leaves a truncated
/// JPEG that looks exactly like a whole one.
const PARTIAL: &str = "partial";
/// Where pages and letters taken back go, instead of being deleted.
const DISCARDED: &str = "discarded";
/// How long they are kept there.
const KEEP_DISCARDED_SECS: u64 = 7 * 24 * 3600;
/// A single page, sent as it is.
const READY: &str = "jpg";
/// A letter of one or more pages.
const LETTER: &str = "pdf";
/// Where the pages of the letter being collected wait.
const OPEN: &str = "open";
/// The pages of the letter just sent, kept only so the setup page can still
/// show what went. Emptied when the next letter starts, which is the same
/// moment the text of those pages stops being shown — see
/// `Scanner::read_pages`. A page here is not in any queue and is never sent.
const SENT: &str = "sent";
/// Beside a letter waiting to be sent: the folders it was decided to go in,
/// one a line. See [`Spool::record_folders`].
const FOLDERS: &str = "folders";

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
    /// When the last of those was, in seconds since the epoch. `None` before
    /// the first failure, and for counters written before attempts were timed.
    pub last_attempt: Option<u64>,
}

impl Pending {
    /// Whether it is time to try this one again.
    ///
    /// Measured from the last failed attempt, not from when the page was
    /// photographed: measured from the capture, anything more than five
    /// minutes old was always due, and a spool that had been failing for an
    /// hour was retried on every pass — the backoff reduced to decoration.
    pub fn due(&self, now: u64) -> bool {
        let since = self.last_attempt.unwrap_or(self.captured_at);
        // A Pi has no real-time clock, and time can jump when NTP arrives. A
        // clock that went backwards makes `since` look like the future; retry
        // rather than wait for the clock to catch up.
        now < since || now - since >= retry_delay(self.attempts).as_secs()
    }
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
        let stem = unique_stem();
        (
            self.dir.join(format!("{stem}.{PARTIAL}")),
            self.dir.join(format!("{stem}.{READY}")),
        )
    }

    /// Like [`Spool::reserve`], for a page of the letter being collected.
    ///
    /// Pages wait in `open/` and nothing there is uploaded: a letter sent a
    /// page at a time is exactly what collecting them is for preventing.
    pub fn reserve_page(&self) -> Result<(PathBuf, PathBuf)> {
        let open = self.dir.join(OPEN);
        fs::create_dir_all(&open).with_context(|| format!("could not open {}", open.display()))?;
        let stem = unique_stem();
        Ok((
            open.join(format!("{stem}.{PARTIAL}")),
            open.join(format!("{stem}.{READY}")),
        ))
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
            let extension = path.extension().and_then(|e| e.to_str());
            if !matches!(extension, Some(READY) | Some(LETTER)) {
                continue;
            }
            found.push(Pending {
                captured_at: stamp_of(&path).unwrap_or(0),
                attempts: self.attempts(&path),
                last_attempt: self.last_attempt(&path),
                path,
            });
        }
        found.sort_by(|a, b| a.captured_at.cmp(&b.captured_at).then(a.path.cmp(&b.path)));
        Ok(found)
    }

    /// The pages of the letter being collected, in the order they were taken.
    pub fn open_pages(&self) -> Result<Vec<PathBuf>> {
        let open = self.dir.join(OPEN);
        if !open.exists() {
            return Ok(Vec::new());
        }
        let mut pages = Vec::new();
        for entry in fs::read_dir(&open)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some(READY) {
                pages.push(path);
            }
        }
        // Names are the capture time and then zero-padded nanoseconds, so
        // their order is the order they were photographed in.
        pages.sort();
        Ok(pages)
    }

    /// Deletes a page of the letter being collected — one photographed by
    /// mistake: a hand, the empty table, the same page twice.
    ///
    /// False when the page is not there any more: deleted already, or the
    /// letter was closed and sent in between. An error only for a name that is
    /// not a page's, or a file that cannot be removed.
    pub fn delete_page(&self, name: &str) -> Result<bool> {
        if !is_page_name(name) {
            bail!("{name:?} is not a page");
        }
        match fs::remove_file(self.dir.join(OPEN).join(name)) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("could not delete {name}")),
        }
    }

    /// Moves one capture — a page of the open letter, or one queued on its
    /// own — to `discarded/`.
    pub fn discard_capture(&self, path: &Path) -> Result<PathBuf> {
        self.discard(path)
    }

    /// Moves the newest page of the open letter to `discarded/`, and returns
    /// where it went — the display's "Undo last page".
    pub fn discard_last_page(&self) -> Result<Option<PathBuf>> {
        match self.open_pages()?.last() {
            Some(page) => self.discard(page).map(Some),
            None => Ok(None),
        }
    }

    /// Moves every page of the open letter to `discarded/`, and says how many
    /// there were — "Cancel letter".
    pub fn discard_open_letter(&self) -> Result<usize> {
        let pages = self.open_pages()?;
        for page in &pages {
            self.discard(page)?;
        }
        Ok(pages.len())
    }

    /// Takes a letter still waiting to be sent out of the queue, into
    /// `discarded/` — "Undo last letter" before it has gone. False when it is
    /// not waiting any more.
    pub fn discard_queued(&self, name: &str) -> Result<bool> {
        if name.contains('/') || name.contains("..") {
            bail!("{name:?} is not a letter in the queue");
        }
        let letter = self.dir.join(name);
        if !letter.exists() {
            return Ok(false);
        }
        self.discard(&letter)?;
        let _ = fs::remove_file(letter.with_extension("attempts"));
        Ok(true)
    }

    /// Moves a page or a letter to `discarded/`, where nothing reads it, rather
    /// than deleting it: a button pressed by mistake costs a trip to the Pi,
    /// not a letter. What has been there a week is deleted on the way.
    fn discard(&self, path: &Path) -> Result<PathBuf> {
        let bin = self.dir.join(DISCARDED);
        fs::create_dir_all(&bin).with_context(|| format!("could not make {}", bin.display()))?;
        let name = path.file_name().context("nothing to discard")?;
        let to = bin.join(name);
        fs::rename(path, &to).with_context(|| format!("could not discard {}", path.display()))?;
        // Renaming keeps a file's time; this is when it was thrown away.
        let _ = fs::File::options()
            .append(true)
            .open(&to)
            .and_then(|file| file.set_modified(std::time::SystemTime::now()));
        if let Ok(entries) = fs::read_dir(&bin) {
            let week_ago =
                std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_DISCARDED_SECS);
            for entry in entries.flatten() {
                let old = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|modified| modified < week_ago);
                if old {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(to)
    }

    /// When the newest page of the open letter was taken, if there is one.
    pub fn last_page_at(&self) -> Result<Option<u64>> {
        Ok(self.open_pages()?.last().and_then(|page| stamp_of(page)))
    }

    /// Makes the open letter one PDF in the queue, and returns its path.
    ///
    /// Written as `.partial` and renamed, like a capture, and the pages are
    /// removed only after the rename. A crash in between leaves the letter
    /// twice — as a PDF and as pages that make the same PDF again, byte for
    /// byte, which Paperless refuses by checksum. The other order could leave
    /// it nowhere.
    /// Closes the open letter without `keep`, which stays open as the first
    /// page of the next — the envelope that just told the letters apart.
    pub fn close_letter_except(&self, keep: &Path) -> Result<Option<PathBuf>> {
        let name = keep.file_name().context("a page with no name")?;
        // Beside the spool under a name nothing reads, for as long as the
        // letter takes to close.
        let aside = self.dir.join(format!("{}.aside", name.to_string_lossy()));
        fs::rename(keep, &aside)
            .with_context(|| format!("could not set {} aside", keep.display()))?;
        let closed = self.close_letter();
        fs::rename(&aside, keep)
            .with_context(|| format!("could not put {} back", keep.display()))?;
        closed
    }

    pub fn close_letter(&self) -> Result<Option<PathBuf>> {
        let mut jpegs = Vec::new();
        let mut used = Vec::new();
        for page in self.open_pages()? {
            let bytes =
                fs::read(&page).with_context(|| format!("could not read {}", page.display()))?;
            if crate::pdf::jpeg_info(&bytes).is_ok() {
                jpegs.push(bytes);
                used.push(page);
            } else {
                // Sent on its own rather than holding the letter back forever:
                // Paperless may make something of it, and if not it is in the
                // queue to be looked at, not stuck in a letter that can never
                // close.
                let aside = self
                    .dir
                    .join(page.file_name().context("a page with no name")?);
                self.commit(&page, &aside)?;
                tracing::warn!(page = %aside.display(), "a page could not go into a PDF; queued on its own");
            }
        }

        let Some(first) = used.first() else {
            return Ok(None);
        };
        let pdf = crate::pdf::from_jpegs(&jpegs)?;

        // Named for its first page: the letter arrived when it was put down,
        // not when the button was pressed.
        let stem = first
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("a page with no name")?;
        let partial = self.dir.join(format!("{stem}.{PARTIAL}"));
        let ready = self.dir.join(format!("{stem}.{LETTER}"));
        fs::write(&partial, &pdf)
            .with_context(|| format!("could not write {}", partial.display()))?;
        self.commit(&partial, &ready)?;

        // Into `sent/` rather than deleted: the page that went is what the
        // setup page goes on showing beside the text read from it, until the
        // next letter starts. Kept as a copy of what is already inside the
        // PDF, so losing it costs nothing.
        let sent = self.dir.join(SENT);
        let _ = fs::remove_dir_all(&sent);
        let kept = fs::create_dir_all(&sent).is_ok();
        for page in &used {
            let moved = kept
                && page
                    .file_name()
                    .is_some_and(|name| fs::rename(page, sent.join(name)).is_ok());
            if !moved {
                let _ = fs::remove_file(page);
            }
        }
        Ok(Some(ready))
    }

    /// Writes down which folders a letter was decided to go in, beside the
    /// letter itself.
    ///
    /// Beside it, and not in memory, because the decision has to survive a
    /// restart: it was made while somebody was holding the paper, and the
    /// paper is in a drawer now. A letter that waited overnight for a
    /// Paperless that was down must still be filed where the person put it.
    pub fn record_folders(&self, letter: &Path, folders: &[String]) {
        let path = letter.with_extension(FOLDERS);
        let text = folders.join("\n");
        if let Err(err) = fs::write(&path, text) {
            tracing::warn!(%err, file = %path.display(), "could not write down where the letter goes");
        }
    }

    /// Where a letter was decided to go, if it was.
    pub fn folders_decided(&self, letter: &Path) -> Vec<String> {
        fs::read_to_string(letter.with_extension(FOLDERS))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Lets go of the pages of the letter just sent. Called when the next
    /// letter starts, so what is on the setup page is always one letter.
    pub fn clear_sent(&self) {
        let _ = fs::remove_dir_all(self.dir.join(SENT));
    }

    /// Where a page of the letter just sent is, if it is still there.
    pub fn sent_page(&self, name: &str) -> PathBuf {
        self.dir.join(SENT).join(name)
    }

    /// Forgets a capture, once Paperless has it.
    pub fn done(&self, pending: &Pending) -> Result<()> {
        let _ = fs::remove_file(self.attempts_path(&pending.path));
        let _ = fs::remove_file(pending.path.with_extension(FOLDERS));
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
        // `<attempts> <when>`: the count for the backoff's size, the time for
        // where it starts.
        fs::write(
            self.attempts_path(&pending.path),
            format!("{attempts} {}", now_secs()),
        )?;
        Ok(attempts)
    }

    fn attempts_path(&self, capture: &Path) -> PathBuf {
        capture.with_extension("attempts")
    }

    fn attempts(&self, capture: &Path) -> u32 {
        self.sidecar_field(capture, 0).unwrap_or(0)
    }

    fn last_attempt(&self, capture: &Path) -> Option<u64> {
        self.sidecar_field(capture, 1)
    }

    /// One whitespace-separated field of the sidecar. A counter written before
    /// attempts were timed has only the first.
    fn sidecar_field<T: std::str::FromStr>(&self, capture: &Path, index: usize) -> Option<T> {
        fs::read_to_string(self.attempts_path(capture))
            .ok()?
            .split_whitespace()
            .nth(index)?
            .parse()
            .ok()
    }
}

/// `<seconds>-<nanoseconds>`: sortable, and unique for one camera.
///
/// The nanoseconds replace the process id this used to carry, which was the
/// same for every capture a daemon made — so two captures within one second
/// shared a name, and the rename of the second replaced the first.
fn unique_stem() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}-{:09}", elapsed.as_secs(), elapsed.subsec_nanos())
}

/// The capture time a file's name starts with.
fn stamp_of(path: &Path) -> Option<u64> {
    path.file_stem()?.to_str()?.split('-').next()?.parse().ok()
}

/// Whether `name` is the plain file name of a page: what the setup page may
/// name, and nothing that could climb out of `open/`.
pub fn is_page_name(name: &str) -> bool {
    name.ends_with(".jpg")
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
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
