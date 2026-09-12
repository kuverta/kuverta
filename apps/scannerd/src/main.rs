//! `scannerd` — the capture half of the paper pipeline.
//!
//! Watches a surface through a camera, and when a page is put down and stops
//! moving, photographs it and hands it to Paperless-ngx. Paperless does the
//! OCR, the deskew, the rotation, the tagging and the archive; this does the
//! part it cannot, which is noticing that a letter has arrived.
//!
//! That division is the whole design, and it is why this is a few hundred
//! lines rather than a few thousand. The temptation is to do image processing
//! here — find the page, straighten it, crop it — and every bit of that either
//! already exists upstream or reduces to configuration once the camera is
//! bolted above a fixed spot.
//!
//! Nothing is lost. A capture is written to the spool before any upload is
//! attempted and removed only once Paperless has confirmed it, so a crash, a
//! flat battery or a Wi-Fi drop in between means a retry rather than a letter
//! nobody has.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use scannerd::camera::{Camera, RpiCamera};
use scannerd::detect::{Detector, Step, Thresholds};
use scannerd::spool::{self, retry_delay, Spool};
use scannerd::upload::Uploader;

#[derive(Parser)]
#[command(name = "scannerd", about = "Capture post and hand it to Paperless-ngx")]
struct Args {
    /// The Paperless-ngx instance.
    #[arg(long, env = "PAPERLESS_URL", default_value = "http://localhost:8000")]
    url: String,

    /// An API token. From the environment by preference, so it stays out of
    /// the process list on a machine anyone on the network can see.
    #[arg(long, env = "PAPERLESS_TOKEN")]
    token: String,

    /// Where captures wait until Paperless has them.
    #[arg(
        long,
        env = "SCANNERD_SPOOL",
        default_value = "/var/lib/scannerd/spool"
    )]
    spool: PathBuf,

    /// Tag every upload with these, so post from this rig is identifiable —
    /// and so one Paperless can serve several addresses.
    #[arg(long = "tag")]
    tags: Vec<String>,

    /// Prefix for document titles.
    #[arg(long, default_value = "Post ")]
    title_prefix: String,

    /// Crop, as `x,y,w,h` fractions of the sensor. Applied by the camera.
    #[arg(long)]
    roi: Option<String>,

    /// The capture program.
    #[arg(long, default_value = "rpicam-still")]
    camera: String,

    /// How long between preview frames.
    #[arg(long, default_value_t = 400)]
    interval_ms: u64,

    /// Consecutive still frames before a page is photographed.
    #[arg(long, default_value_t = 3)]
    settle_frames: u8,

    /// Drain the spool and exit, without watching for pages. For a cron job,
    /// and for checking the other half works before there is a camera.
    #[arg(long)]
    drain_only: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scannerd=info".into()),
        )
        .init();

    let args = Args::parse();

    let spool = Spool::open(&args.spool)?;
    let uploader = Uploader::new(
        &args.url,
        &args.token,
        args.tags.clone(),
        args.title_prefix.clone(),
    )?;

    tracing::info!(spool = %spool.dir().display(), paperless = %args.url, "starting");

    // Whatever is already waiting goes first, before anything new is taken.
    // A daemon that photographs eagerly and uploads lazily is one that fills a
    // disk with post nobody can search.
    drain(&spool, &uploader).await;

    if args.drain_only {
        return Ok(());
    }

    let camera = RpiCamera {
        program: args.camera.clone(),
        roi: args.roi.clone(),
        ..RpiCamera::default()
    };
    watch(&args, &camera, &spool, &uploader).await
}

/// The capture loop.
async fn watch(args: &Args, camera: &dyn Camera, spool: &Spool, uploader: &Uploader) -> Result<()> {
    let mut detector = Detector::new(Thresholds::default(), args.settle_frames);
    let interval = Duration::from_millis(args.interval_ms);
    let mut failures: u32 = 0;

    loop {
        tokio::time::sleep(interval).await;

        let frame = match camera.preview() {
            Ok(frame) => {
                failures = 0;
                frame
            }
            Err(err) => {
                failures += 1;
                // A camera that has been unplugged should say so once and keep
                // trying, not fill a log at four lines a second.
                if failures <= 3 || failures.is_multiple_of(50) {
                    tracing::warn!(%err, failures, "no preview frame");
                }
                continue;
            }
        };

        if detector.observe(&frame) == Step::Capture {
            match capture(camera, spool) {
                Ok(()) => drain(spool, uploader).await,
                Err(err) => tracing::error!(%err, "capture failed"),
            }
        }
    }
}

/// Photographs the page and puts it in the spool.
fn capture(camera: &dyn Camera, spool: &Spool) -> Result<()> {
    let (partial, ready) = spool.reserve();

    camera
        .capture(&partial)
        .with_context(|| "could not photograph the page")?;

    // Only now is it a capture. Until the rename it is a file that may be
    // half-written, and nothing reads it.
    spool.commit(&partial, &ready)?;
    tracing::info!(file = %ready.display(), "captured");
    Ok(())
}

/// Sends everything waiting, oldest first.
async fn drain(spool: &Spool, uploader: &Uploader) {
    let pending = match spool.pending() {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(%err, "could not read the spool");
            return;
        }
    };

    if pending.is_empty() {
        return;
    }
    tracing::info!(waiting = pending.len(), "draining the spool");

    for item in pending {
        // A capture that has failed recently is left for later rather than
        // retried immediately: a Paperless that is down should not be asked
        // forty times a minute.
        let wait = retry_delay(item.attempts);
        if wait > Duration::ZERO && !older_than(&item, wait) {
            continue;
        }

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

/// Whether enough time has passed since a capture to try it again.
fn older_than(item: &spool::Pending, wait: Duration) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    now.saturating_sub(item.captured_at) >= wait.as_secs()
}
