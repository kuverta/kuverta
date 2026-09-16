//! One turn of the capture loop.
//!
//! Split out of the binary so the loop itself can be driven by a test: a
//! scripted camera, a spool in a temporary directory and a Paperless on
//! loopback, with the clock passed in rather than read. `main` is then only
//! arguments, the setup page's commands, and a sleep.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::button::Button;
use crate::camera::Camera;
use crate::detect::{Detector, State, Step, Thresholds};
use crate::hub::Event;
use crate::spool::Spool;
use crate::straighten::{straighten, Corners};
use crate::upload::Uploader;

/// What one turn did.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    Watching,
    Captured(PathBuf),
    /// The camera gave no frame. The spool was still looked at.
    NoFrame,
}

/// Collecting pages into letters, closed by a button or by being left alone.
pub struct Letters {
    pub button: Button,
    /// A letter nobody closed is closed this long after its last page, so a
    /// forgotten press delays post rather than keeping it forever.
    pub idle: Duration,
}

/// Activity lines kept for the page. Capped, because without a page nobody
/// takes them.
const EVENTS: usize = 50;

/// How long a finish waits for something that keeps moving.
pub const FINISH_WAIT_SECS: u64 = 15;

pub struct Scanner {
    detector: Detector,
    /// How often the spool is looked at when nothing is being captured.
    drain_every: Duration,
    last_drain: Option<u64>,
    preview_failures: u32,
    /// `None`: every page is its own document.
    letters: Option<Letters>,
    /// When finishing was asked for. The letter closes as soon as no page is
    /// settling, or after [`FINISH_WAIT_SECS`] if something never stops moving.
    close_requested_at: Option<u64>,
    /// The camera's last frame: what the page shows, and what "learn empty
    /// table" learns.
    last_frame: Option<Vec<u8>>,
    /// Where the empty table is kept between runs.
    baseline_file: Option<PathBuf>,
    /// Which baseline the file holds.
    saved_generation: u64,
    /// Whether the baseline is known to be the empty table, rather than
    /// whatever the first frame happened to show.
    trusted_baseline: bool,
    /// Where the page's corners are in a photograph, when the camera looks at
    /// the table at an angle.
    straighten: Option<Corners>,
    events: Vec<Event>,
}

impl Scanner {
    pub fn new(settle_frames: u8, drain_every: Duration) -> Self {
        Self {
            detector: Detector::new(Thresholds::default(), settle_frames),
            drain_every,
            last_drain: None,
            preview_failures: 0,
            letters: None,
            close_requested_at: None,
            last_frame: None,
            baseline_file: None,
            saved_generation: 0,
            trusted_baseline: false,
            straighten: None,
            events: Vec::new(),
        }
    }

    /// Collects pages into letters instead of sending each on its own.
    pub fn collecting(mut self, letters: Letters) -> Self {
        self.letters = Some(letters);
        self
    }

    /// Keeps the empty table in `path` between runs, loading it now if there
    /// is one.
    ///
    /// Learnt from the first frame, "empty" is whatever lay under the camera at
    /// start. A page lying there becomes the table; lifting it reads as a page
    /// and is photographed, and the detector then waits for that page to come
    /// back before it will photograph anything else. Kept on disk, a restart
    /// with a letter on the table sees the letter.
    pub fn keep_baseline_in(mut self, path: PathBuf) -> Self {
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => {
                self.detector.learn_baseline(&bytes);
                self.trusted_baseline = true;
                tracing::info!(file = %path.display(), "remembered the empty table");
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(%err, "could not read the saved empty table"),
        }
        self.saved_generation = self.detector.baseline_generation();
        self.baseline_file = Some(path);
        self
    }

    /// Warps every photograph so that these corners, as fractions of it,
    /// become its own, in order — which is also how a photograph is turned.
    /// `None` keeps photographs as the camera takes them.
    pub fn straighten_with(&mut self, corners: Option<Corners>) {
        self.straighten = corners;
    }

    pub fn collecting_letters(&self) -> bool {
        self.letters.is_some()
    }

    pub fn state(&self) -> State {
        self.detector.state()
    }

    pub fn settle_frames(&self) -> u8 {
        self.detector.settle_frames()
    }

    /// Whether the empty table has really been seen, as opposed to guessed.
    pub fn has_baseline(&self) -> bool {
        self.trusted_baseline && self.detector.has_baseline()
    }

    pub fn last_frame(&self) -> Option<&[u8]> {
        self.last_frame.as_deref()
    }

    /// What happened since this was last asked, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The page's "Finish letter". Goes through the same wait as the button,
    /// so a page still settling is not left out, and says so at once. False
    /// when pages are not being collected.
    pub fn request_close(&mut self, now: u64) -> bool {
        if self.letters.is_none() {
            return false;
        }
        if self.close_requested_at.is_none() {
            self.close_requested_at = Some(now);
            self.event(now, true, "finishing the letter");
        }
        true
    }

    /// Whether finishing was asked for and the letter is not closed yet.
    pub fn finishing(&self) -> bool {
        self.close_requested_at.is_some()
    }

    /// The last frame's change from the empty table and from the frame before.
    pub fn measure(&self) -> Option<(f32, f32)> {
        self.detector.last_measure()
    }

    /// "The table is empty now": learns the last frame as the empty table and
    /// arms again. False when there is no frame yet to learn from.
    pub fn learn_empty(&mut self, now: u64) -> bool {
        let Some(frame) = self.last_frame.clone() else {
            self.event(
                now,
                false,
                "no picture from the camera yet to learn the table from",
            );
            return false;
        };
        self.detector.learn_empty(&frame);
        self.trusted_baseline = true;
        self.save_baseline();
        self.event(now, true, "learnt the empty table");
        true
    }

    /// Forgets the empty table, as when the crop changes and the camera no
    /// longer shows the same part of it.
    pub fn forget_baseline(&mut self) {
        self.detector.forget_baseline();
        self.trusted_baseline = false;
        if let Some(path) = &self.baseline_file {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Looks at one frame, captures if a page has settled, closes the letter if
    /// it is finished, and sends whatever is due. `now` is seconds since the
    /// epoch.
    pub async fn turn(
        &mut self,
        camera: &dyn Camera,
        spool: &Spool,
        uploader: &Uploader,
        now: u64,
    ) -> Turn {
        if self.close_if_due(spool, now) {
            // The letter someone just finished goes now, not on the timer.
            self.drain_now(spool, uploader, now).await;
        }

        let collecting = self.letters.is_some();
        let turn = match camera.preview() {
            Ok(frame) => {
                self.preview_failures = 0;
                let before = (self.detector.state(), self.detector.baseline_generation());
                let step = self.detector.observe(&frame);
                if self.detector.baseline_generation() != before.1 {
                    // Relearnt because the table was seen clear after a
                    // photograph: that is the empty table. Learnt from a first
                    // frame, or because the size changed: a guess, not kept.
                    self.trusted_baseline = before.0 == State::Spent;
                    self.save_baseline();
                }
                self.last_frame = Some(frame);

                if step == Step::Capture {
                    match capture(camera, spool, collecting, self.straighten.as_ref()) {
                        Ok((path, warning)) => {
                            if let Some(warning) = warning {
                                self.event(now, false, warning);
                            }
                            let text = if collecting {
                                let pages =
                                    spool.open_pages().map(|pages| pages.len()).unwrap_or(0);
                                format!("photographed page {pages} of this letter")
                            } else {
                                "photographed a page".to_string()
                            };
                            self.event(now, true, text);
                            // A single page goes straight away: it is the
                            // letter someone is standing next to. A page of a
                            // letter waits for the rest.
                            if !collecting {
                                self.drain_now(spool, uploader, now).await;
                            }
                            return Turn::Captured(path);
                        }
                        Err(err) => {
                            tracing::error!(%err, "capture failed");
                            self.event(
                                now,
                                false,
                                format!("could not photograph the page: {err:#}"),
                            );
                            Turn::Watching
                        }
                    }
                } else {
                    Turn::Watching
                }
            }
            Err(err) => {
                self.preview_failures += 1;
                // A camera that has been unplugged should say so once and keep
                // trying, not fill a log at four lines a second.
                if self.preview_failures <= 3 || self.preview_failures.is_multiple_of(50) {
                    tracing::warn!(%err, failures = self.preview_failures, "no preview frame");
                }
                if self.preview_failures == 1 {
                    self.event(now, false, format!("the camera gave no picture: {err:#}"));
                }
                Turn::NoFrame
            }
        };

        // On a timer as well as after a capture. Draining only after a capture
        // meant a letter photographed while the network was down waited for
        // the next letter — or a restart — however soon the network came
        // back. And whatever the camera is doing: a broken camera is no reason
        // to keep post that is already photographed.
        if self.drain_due(now) {
            self.drain_now(spool, uploader, now).await;
        }
        turn
    }

    /// Sends everything that is due, now.
    pub async fn drain_now(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        self.last_drain = Some(now);
        drain(spool, uploader, now, false, &mut self.events).await;
        self.cap_events();
    }

    /// Sends everything waiting, whatever its backoff says — for someone who
    /// has just fixed the network and does not want to wait five minutes to
    /// see whether it worked.
    pub async fn retry_all(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        self.last_drain = Some(now);
        drain(spool, uploader, now, true, &mut self.events).await;
        self.cap_events();
    }

    fn drain_due(&self, now: u64) -> bool {
        match self.last_drain {
            None => true,
            // A clock that went backwards (no RTC on a Pi) must not postpone
            // retries until it catches up.
            Some(last) => now < last || now - last >= self.drain_every.as_secs(),
        }
    }

    fn event(&mut self, at: u64, ok: bool, text: impl Into<String>) {
        self.events.push(Event {
            at,
            ok,
            text: text.into(),
        });
        self.cap_events();
    }

    fn cap_events(&mut self) {
        if self.events.len() > EVENTS {
            let extra = self.events.len() - EVENTS;
            self.events.drain(..extra);
        }
    }

    /// Writes the baseline if it is trusted and not already on disk.
    fn save_baseline(&mut self) {
        let generation = self.detector.baseline_generation();
        if !self.trusted_baseline || generation == self.saved_generation {
            return;
        }
        let (Some(path), Some(baseline)) = (&self.baseline_file, self.detector.baseline()) else {
            return;
        };
        // Written whole and renamed: a half-written baseline would be one of
        // the wrong size, which the detector would then have to throw away.
        let partial = path.with_extension("partial");
        match std::fs::write(&partial, baseline).and_then(|()| std::fs::rename(&partial, path)) {
            Ok(()) => self.saved_generation = generation,
            Err(err) => tracing::warn!(%err, "could not save the empty table"),
        }
    }

    /// Closes the open letter if finishing was asked for or it has been left
    /// alone, and says whether a letter was closed.
    fn close_if_due(&mut self, spool: &Spool, now: u64) -> bool {
        let (pressed, idle) = match &self.letters {
            Some(letters) => (letters.button.pressed(), letters.idle),
            None => return false,
        };
        if pressed && self.close_requested_at.is_none() {
            self.close_requested_at = Some(now);
        }

        // Not while a page is settling. Putting the last page down and
        // pressing the button is one movement, and the press arrives before
        // the page has been still long enough to photograph — closing then
        // would send the letter without its last page.
        //
        // But not for ever. On the Pi, with the room in view, something was
        // always moving, and a press on the page did nothing at all until the
        // letter closed itself five minutes later. After a while the press
        // wins; anything not yet photographed starts the next letter.
        if matches!(self.detector.state(), State::Settling { .. }) {
            let waited = self
                .close_requested_at
                .is_some_and(|at| now.saturating_sub(at) >= FINISH_WAIT_SECS);
            if !waited {
                return false;
            }
        }

        let left_alone = match spool.last_page_at() {
            // Not when the clock went backwards: the button still works, and
            // closing a letter early is worse than closing it late.
            Ok(Some(at)) => now >= at && now - at >= idle.as_secs(),
            Ok(None) => false,
            Err(err) => {
                tracing::error!(%err, "could not read the open letter");
                false
            }
        };
        let requested = self.close_requested_at.is_some();
        if !requested && !left_alone {
            return false;
        }
        self.close_requested_at = None;

        let pages = spool.open_pages().map(|pages| pages.len()).unwrap_or(0);
        match spool.close_letter() {
            Ok(Some(letter)) => {
                let how = if requested { "finished" } else { "left alone" };
                tracing::info!(letter = %letter.display(), how, "letter closed");
                let plural = if pages == 1 { "" } else { "s" };
                self.event(
                    now,
                    true,
                    format!("letter of {pages} page{plural} {how}, sending"),
                );
                true
            }
            Ok(None) => {
                tracing::info!("finishing asked for with no pages photographed; nothing to close");
                self.event(now, true, "nothing to finish: no pages photographed yet");
                false
            }
            Err(err) => {
                tracing::error!(%err, "could not close the letter; its pages are kept");
                self.event(
                    now,
                    false,
                    format!("could not finish the letter (its pages are kept): {err:#}"),
                );
                false
            }
        }
    }
}

/// Photographs the page, straightens it if the camera is at an angle, and
/// puts it in the spool — in the open letter when collecting, or straight in
/// the queue. Also says what went wrong short of losing the page.
fn capture(
    camera: &dyn Camera,
    spool: &Spool,
    collecting: bool,
    corners: Option<&Corners>,
) -> Result<(PathBuf, Option<String>)> {
    let (partial, ready) = if collecting {
        spool.reserve_page()?
    } else {
        spool.reserve()
    };

    camera
        .capture(&partial)
        .with_context(|| "could not photograph the page")?;

    // A page that cannot be straightened is still kept as it was taken: a
    // slanted letter is better than none.
    let mut warning = None;
    if let Some(corners) = corners {
        let started = std::time::Instant::now();
        match straighten_file(&partial, corners) {
            Ok(()) => tracing::info!(ms = started.elapsed().as_millis() as u64, "straightened"),
            Err(err) => {
                tracing::warn!(%err, "could not straighten the page; keeping it as taken");
                warning = Some(format!(
                    "could not straighten the page, kept it as taken: {err:#}"
                ));
            }
        }
    }

    // Only now is it a capture. Until the rename it is a file that may be
    // half-written, and nothing reads it.
    spool.commit(&partial, &ready)?;
    tracing::info!(file = %ready.display(), "captured");
    Ok((ready, warning))
}

/// Replaces a photograph with its straightened self — whole, by rename, so a
/// crash leaves one or the other and never half of each.
fn straighten_file(path: &std::path::Path, corners: &Corners) -> Result<()> {
    let straight = straighten(&std::fs::read(path)?, corners)?;
    let next = path.with_extension("straight");
    std::fs::write(&next, straight)?;
    std::fs::rename(&next, path)?;
    Ok(())
}

/// Sends everything waiting — and due, unless `force` — oldest first.
async fn drain(spool: &Spool, uploader: &Uploader, now: u64, force: bool, events: &mut Vec<Event>) {
    let pending = match spool.pending() {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(%err, "could not read the spool");
            return;
        }
    };

    let due: Vec<_> = pending
        .into_iter()
        .filter(|item| force || item.due(now))
        .collect();
    if due.is_empty() {
        return;
    }
    tracing::info!(waiting = due.len(), "draining the spool");

    for item in due {
        let filename = item
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "post.jpg".to_string());

        let bytes = match std::fs::read(&item.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!(%err, file = %item.path.display(), "could not read a capture");
                continue;
            }
        };

        match uploader.send(&filename, bytes).await {
            Ok(task) => {
                tracing::info!(file = %filename, %task, "uploaded");
                events.push(Event {
                    at: now,
                    ok: true,
                    text: format!("sent {filename} to Paperless"),
                });
                if let Err(err) = spool.done(&item) {
                    // The upload succeeded, so this is not a failure of the
                    // capture — but it will be uploaded again next time, and
                    // Paperless will deduplicate it by checksum.
                    tracing::warn!(%err, "uploaded but could not clear the spool");
                }
            }
            Err(err) => {
                let attempts = spool.failed(&item).unwrap_or(item.attempts + 1);
                tracing::warn!(%err, file = %filename, attempts, "upload failed; kept");
                events.push(Event {
                    at: now,
                    ok: false,
                    text: format!("could not send {filename} (kept, will retry): {err:#}"),
                });
            }
        }
    }
}
