//! One turn of the capture loop.
//!
//! Split out of the binary so the loop itself can be driven by a test: a
//! scripted camera, a spool in a temporary directory and a Paperless on
//! loopback, with the clock passed in rather than read. `main` is then only
//! arguments and a sleep.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::button::Button;
use crate::camera::Camera;
use crate::detect::{Detector, State, Step, Thresholds};
use crate::spool::Spool;
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

pub struct Scanner {
    detector: Detector,
    /// How often the spool is looked at when nothing is being captured.
    drain_every: Duration,
    last_drain: Option<u64>,
    preview_failures: u32,
    /// `None`: every page is its own document.
    letters: Option<Letters>,
    /// The button was pressed, and the letter closes as soon as no page is
    /// settling.
    close_requested: bool,
}

impl Scanner {
    pub fn new(settle_frames: u8, drain_every: Duration) -> Self {
        Self {
            detector: Detector::new(Thresholds::default(), settle_frames),
            drain_every,
            last_drain: None,
            preview_failures: 0,
            letters: None,
            close_requested: false,
        }
    }

    /// Collects pages into letters instead of sending each on its own.
    pub fn collecting(mut self, letters: Letters) -> Self {
        self.letters = Some(letters);
        self
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
                if self.detector.observe(&frame) == Step::Capture {
                    match capture(camera, spool, collecting) {
                        Ok(path) => {
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
        drain(spool, uploader, now).await;
    }

    fn drain_due(&self, now: u64) -> bool {
        match self.last_drain {
            None => true,
            // A clock that went backwards (no RTC on a Pi) must not postpone
            // retries until it catches up.
            Some(last) => now < last || now - last >= self.drain_every.as_secs(),
        }
    }

    /// Closes the open letter if the button was pressed or it has been left
    /// alone, and says whether a letter was closed.
    fn close_if_due(&mut self, spool: &Spool, now: u64) -> bool {
        let (pressed, idle) = match &self.letters {
            Some(letters) => (letters.button.pressed(), letters.idle),
            None => return false,
        };
        if pressed {
            self.close_requested = true;
        }

        // Not while a page is settling. Putting the last page down and
        // pressing the button is one movement, and the press arrives before
        // the page has been still long enough to photograph — closing then
        // would send the letter without its last page.
        if matches!(self.detector.state(), State::Settling { .. }) {
            return false;
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
        if !self.close_requested && !left_alone {
            return false;
        }
        self.close_requested = false;

        match spool.close_letter() {
            Ok(Some(letter)) => {
                let how = if pressed || !left_alone {
                    "button"
                } else {
                    "left alone"
                };
                tracing::info!(letter = %letter.display(), how, "letter closed");
                true
            }
            Ok(None) => {
                tracing::info!("button pressed with no pages photographed; nothing to close");
                false
            }
            Err(err) => {
                tracing::error!(%err, "could not close the letter; its pages are kept");
                false
            }
        }
    }
}

/// Photographs the page and puts it in the spool — in the open letter when
/// collecting, or straight in the queue.
fn capture(camera: &dyn Camera, spool: &Spool, collecting: bool) -> Result<PathBuf> {
    let (partial, ready) = if collecting {
        spool.reserve_page()?
    } else {
        spool.reserve()
    };

    camera
        .capture(&partial)
        .with_context(|| "could not photograph the page")?;

    // Only now is it a capture. Until the rename it is a file that may be
    // half-written, and nothing reads it.
    spool.commit(&partial, &ready)?;
    tracing::info!(file = %ready.display(), "captured");
    Ok(ready)
}

/// Sends everything waiting and due, oldest first.
async fn drain(spool: &Spool, uploader: &Uploader, now: u64) {
    let pending = match spool.pending() {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(%err, "could not read the spool");
            return;
        }
    };

    let due: Vec<_> = pending.into_iter().filter(|item| item.due(now)).collect();
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
            }
        }
    }
}
