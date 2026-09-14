//! One turn of the capture loop.
//!
//! Split out of the binary so the loop itself can be driven by a test: a
//! scripted camera, a spool in a temporary directory and a Paperless on
//! loopback, with the clock passed in rather than read. `main` is then only
//! arguments and a sleep.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::camera::Camera;
use crate::detect::{Detector, Step, Thresholds};
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

pub struct Scanner {
    detector: Detector,
    /// How often the spool is looked at when nothing is being captured.
    drain_every: Duration,
    last_drain: Option<u64>,
    preview_failures: u32,
}

impl Scanner {
    pub fn new(settle_frames: u8, drain_every: Duration) -> Self {
        Self {
            detector: Detector::new(Thresholds::default(), settle_frames),
            drain_every,
            last_drain: None,
            preview_failures: 0,
        }
    }

    /// Looks at one frame, captures if a page has settled, and sends whatever
    /// is due. `now` is seconds since the epoch.
    pub async fn turn(
        &mut self,
        camera: &dyn Camera,
        spool: &Spool,
        uploader: &Uploader,
        now: u64,
    ) -> Turn {
        let turn = match camera.preview() {
            Ok(frame) => {
                self.preview_failures = 0;
                if self.detector.observe(&frame) == Step::Capture {
                    match capture(camera, spool) {
                        Ok(path) => {
                            // Straight away, not on the timer: this is the
                            // letter someone is standing next to.
                            self.drain_now(spool, uploader, now).await;
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
}

/// Photographs the page and puts it in the spool.
fn capture(camera: &dyn Camera, spool: &Spool) -> Result<PathBuf> {
    let (partial, ready) = spool.reserve();

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
