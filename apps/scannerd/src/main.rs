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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::Parser;
use scannerd::button::Button;
use scannerd::camera::RpiCamera;
use scannerd::run::{Letters, Scanner};
use scannerd::spool::Spool;
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
    /// Or `SCANNERD_TAGS=a,b`, which is how a tag with a space in it gets
    /// through a systemd environment file.
    #[arg(long = "tag", env = "SCANNERD_TAGS", value_delimiter = ',')]
    tags: Vec<String>,

    /// Prefix for document titles.
    #[arg(long, default_value = "Post ")]
    title_prefix: String,

    /// Crop, as `x,y,w,h` fractions of the sensor. Applied by the camera, to
    /// the frames it watches as well as to the photograph.
    #[arg(long, env = "SCANNERD_ROI")]
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

    /// How often to retry uploads that are waiting, when nothing new is being
    /// captured. Each capture's own backoff still applies.
    #[arg(long, default_value_t = 30)]
    drain_every_secs: u64,

    /// Collect pages into one letter until this button is pressed: an input
    /// device such as the one the `gpio-key` overlay makes
    /// (`/dev/input/by-path/…`), or `stdin` to press Enter in the terminal.
    /// Without it, every page is its own document.
    #[arg(long, env = "SCANNERD_BUTTON")]
    button: Option<PathBuf>,

    /// The key the button sends. 28 is Enter, which the overlay line in the
    /// readme configures.
    #[arg(long, default_value_t = scannerd::button::KEY_ENTER)]
    button_key: u16,

    /// Close a letter nobody closed this long after its last page.
    #[arg(long, default_value_t = 300)]
    letter_idle_secs: u64,

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
    let mut scanner = Scanner::new(
        args.settle_frames,
        Duration::from_secs(args.drain_every_secs),
    );

    tracing::info!(spool = %spool.dir().display(), paperless = %args.url, "starting");

    // Whatever is already waiting goes first, before anything new is taken.
    // A daemon that photographs eagerly and uploads lazily is one that fills a
    // disk with post nobody can search.
    scanner.drain_now(&spool, &uploader, now()).await;

    if args.drain_only {
        return Ok(());
    }

    // After the drain: a button that is not there must not stop post that is
    // already photographed from being sent.
    if let Some(device) = &args.button {
        let button = if device.as_os_str() == "stdin" {
            Button::stdin()
        } else {
            Button::input_device(device, args.button_key)?
        };
        tracing::info!(button = %device.display(), "collecting pages into letters");
        scanner = scanner.collecting(Letters {
            button,
            idle: Duration::from_secs(args.letter_idle_secs),
        });
    }

    let camera = RpiCamera {
        program: args.camera.clone(),
        roi: args.roi.clone(),
        ..RpiCamera::default()
    };
    let interval = Duration::from_millis(args.interval_ms);
    loop {
        tokio::time::sleep(interval).await;
        scanner.turn(&camera, &spool, &uploader, now()).await;
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
