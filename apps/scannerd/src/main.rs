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

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::Parser;
use scannerd::button::Button;
use scannerd::camera::{Camera, RpiCamera};
use scannerd::detect::State;
use scannerd::hub::{Command, Event, Hub, Queued, SettingsView};
use scannerd::locate::find_page;
use scannerd::run::{Letters, Scanner, Turn};
use scannerd::settings::Settings;
use scannerd::spool::Spool;
use scannerd::upload::Uploader;
use scannerd::web::{self, Web};

#[derive(Parser)]
#[command(name = "scannerd", about = "Capture post and hand it to Paperless-ngx")]
struct Args {
    /// The Paperless-ngx instance.
    #[arg(long, env = "PAPERLESS_URL", default_value = "http://localhost:8000")]
    url: String,

    /// An API token. From the environment by preference, so it stays out of
    /// the process list on a machine anyone on the network can see. May be
    /// left out when the setup page will set it.
    #[arg(long, env = "PAPERLESS_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// Where captures wait until Paperless has them. The setup page's
    /// settings and the learnt empty table are kept here too.
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

    /// The camera sensor's full width in pixels, which a crop's photograph is
    /// a share of. 2592×1944 is the Pi camera v1; the v2 is 3280×2464, the v3
    /// 4608×2592.
    #[arg(long, env = "SCANNERD_SENSOR_WIDTH", default_value_t = 2592)]
    sensor_width: u32,

    /// The camera sensor's full height in pixels.
    #[arg(long, env = "SCANNERD_SENSOR_HEIGHT", default_value_t = 1944)]
    sensor_height: u32,

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
    /// With neither this nor the setup page, every page is its own document.
    #[arg(long, env = "SCANNERD_BUTTON")]
    button: Option<PathBuf>,

    /// The key the button sends. 28 is Enter, which the overlay line in the
    /// readme configures.
    #[arg(long, default_value_t = scannerd::button::KEY_ENTER)]
    button_key: u16,

    /// Close a letter nobody closed this long after its last page.
    #[arg(long, default_value_t = 300)]
    letter_idle_secs: u64,

    /// Serve the setup page here, e.g. `0.0.0.0:8080`. Anything but a loopback
    /// address needs SCANNERD_UI_PASSWORD: the page shows photographs of post.
    #[arg(long, env = "SCANNERD_UI")]
    ui: Option<SocketAddr>,

    /// The setup page's password (any user name).
    #[arg(long, env = "SCANNERD_UI_PASSWORD", hide_env_values = true)]
    ui_password: Option<String>,

    /// Drain the spool and exit, without watching for pages. For a cron job,
    /// and for checking the other half works before there is a camera.
    #[arg(long)]
    drain_only: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scannerd=info".into()),
        )
        .init();

    let args = Args::parse();

    let spool = Spool::open(&args.spool)?;
    let mut settings = Settings::load(spool.dir())?;
    let mut uploader = uploader_for(&args, &settings)?;
    let mut scanner = Scanner::new(
        args.settle_frames,
        Duration::from_secs(args.drain_every_secs),
    )
    .keep_baseline_in(spool.dir().join("empty-table.gray"));

    tracing::info!(spool = %spool.dir().display(), "starting");

    // Whatever is already waiting goes first, before anything new is taken.
    // A daemon that photographs eagerly and uploads lazily is one that fills a
    // disk with post nobody can search.
    scanner.drain_now(&spool, &uploader, now()).await;

    if args.drain_only {
        return Ok(());
    }

    let mut camera = RpiCamera {
        program: args.camera.clone(),
        roi: settings.effective_roi(args.roi.as_deref()),
        sensor_width: args.sensor_width,
        sensor_height: args.sensor_height,
        ..RpiCamera::default()
    };
    let hub = Arc::new(Hub::new(camera.preview_width, camera.preview_height));

    if let Some(address) = args.ui {
        let password = args
            .ui_password
            .clone()
            .filter(|password| !password.is_empty());
        if password.is_none() && !address.ip().is_loopback() {
            bail!(
                "the setup page on {address} would show photographs of your post to anyone on \
                 the network: set SCANNERD_UI_PASSWORD, or serve it on 127.0.0.1"
            );
        }
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .with_context(|| format!("could not serve the setup page on {address}"))?;
        tracing::info!(%address, "setup page");
        let web = Web::new(hub.clone(), spool.dir().to_path_buf(), password);
        tokio::spawn(web::serve(listener, Arc::new(web)));
    }

    // Pages collect into letters whenever there is a way to finish one: the
    // button on the device, or the one on the page.
    let button = match &args.button {
        Some(device) if device.as_os_str() == "stdin" => Some(Button::stdin()),
        Some(device) => Some(Button::input_device(device, args.button_key)?),
        None if args.ui.is_some() => Some(Button::channel().1),
        None => None,
    };
    if let Some(button) = button {
        tracing::info!("collecting pages into letters");
        scanner = scanner.collecting(Letters {
            button,
            idle: Duration::from_secs(args.letter_idle_secs),
        });
    }

    let interval = Duration::from_millis(args.interval_ms);
    loop {
        tokio::time::sleep(interval).await;
        let now = now();

        // Between turns, never during one: the loop owns the camera.
        for command in hub.take_commands() {
            match command {
                Command::FinishLetter => {
                    if !scanner.request_close(now) {
                        hub.event(now, false, "pages are not being collected into letters");
                    }
                }
                Command::LearnEmpty => {
                    scanner.learn_empty(now);
                }
                Command::DeletePage(name) => {
                    // Numbered as the page showed it, before it goes.
                    let number = spool.open_pages().ok().and_then(|pages| {
                        pages
                            .iter()
                            .position(|page| page.file_name().is_some_and(|n| n == name.as_str()))
                            .map(|index| index + 1)
                    });
                    match spool.delete_page(&name) {
                        Ok(true) => {
                            let text = match number {
                                Some(number) => format!("deleted page {number}"),
                                None => "deleted a page".to_string(),
                            };
                            hub.event(now, true, text);
                        }
                        Ok(false) => hub.event(
                            now,
                            false,
                            "that page was already gone — the letter may have been sent",
                        ),
                        Err(err) => {
                            hub.event(now, false, format!("could not delete the page: {err:#}"))
                        }
                    }
                }
                Command::RetryNow => scanner.retry_all(&spool, &uploader, now).await,
                Command::FullView => {
                    // Found in a greyscale frame of the same view: there is no
                    // JPEG decoder in this binary, and needs to be none.
                    let suggestion = camera
                        .full_view()
                        .ok()
                        .and_then(|luma| {
                            find_page(
                                &luma,
                                camera.preview_width as usize,
                                camera.preview_height as usize,
                            )
                        })
                        .map(|area| area.to_roi());
                    match camera.snapshot() {
                        Ok(jpeg) => hub.set_full_view(jpeg, suggestion),
                        Err(err) => hub.event(
                            now,
                            false,
                            format!("could not take a picture of the whole view: {err:#}"),
                        ),
                    }
                }
                Command::CheckPaperless => {
                    let (ok, text) = match uploader.check().await {
                        Ok(text) => (true, text),
                        Err(err) => (false, format!("{err:#}")),
                    };
                    hub.update(|status| status.check = Some(Event { at: now, ok, text }));
                }
                Command::Settings(newer) => {
                    settings.merge(newer);
                    match settings.save(spool.dir()) {
                        Ok(()) => hub.event(now, true, "settings saved"),
                        Err(err) => {
                            hub.event(now, false, format!("could not save the settings: {err:#}"))
                        }
                    }
                    match uploader_for(&args, &settings) {
                        Ok(changed) => uploader = changed,
                        Err(err) => hub.event(now, false, format!("{err:#}")),
                    }
                    let roi = settings.effective_roi(args.roi.as_deref());
                    if roi != camera.roi {
                        camera.roi = roi;
                        // The camera now shows a different part of the table.
                        scanner.forget_baseline();
                        hub.event(
                            now,
                            true,
                            "crop changed: clear the table and learn it again",
                        );
                    }
                }
            }
        }

        let turn = scanner.turn(&camera, &spool, &uploader, now).await;

        if args.ui.is_some() {
            let view = SettingsView {
                url: settings.url.clone().unwrap_or_else(|| args.url.clone()),
                tags: settings.tags.clone().unwrap_or_else(|| args.tags.clone()),
                roi: camera.roi.clone(),
                token_set: settings.token.is_some()
                    || args.token.as_deref().is_some_and(|t| !t.is_empty()),
            };
            publish(&hub, &mut scanner, &spool, view, &turn);
        } else {
            scanner.take_events();
        }
    }
}

/// The uploader for the settings in force: the page's, then the env file's.
fn uploader_for(args: &Args, settings: &Settings) -> Result<Uploader> {
    let url = settings.url.clone().unwrap_or_else(|| args.url.clone());
    let token = settings
        .token
        .clone()
        .or_else(|| args.token.clone())
        .unwrap_or_default();
    let tags = settings.tags.clone().unwrap_or_else(|| args.tags.clone());
    Uploader::new(&url, &token, tags, args.title_prefix.clone())
}

/// Tells the page what the last turn saw.
fn publish(hub: &Hub, scanner: &mut Scanner, spool: &Spool, settings: SettingsView, turn: &Turn) {
    for event in scanner.take_events() {
        hub.event(event.at, event.ok, event.text);
    }
    if *turn != Turn::NoFrame {
        if let Some(frame) = scanner.last_frame() {
            hub.set_frame(frame.to_vec());
        }
    }

    let name = |path: &std::path::Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let open_pages = spool
        .open_pages()
        .unwrap_or_default()
        .iter()
        .map(|page| name(page))
        .collect();
    let queue = spool
        .pending()
        .unwrap_or_default()
        .iter()
        .map(|item| Queued {
            name: name(&item.path),
            attempts: item.attempts,
        })
        .collect();
    let (state, frames_still) = match (turn, scanner.state()) {
        (Turn::NoFrame, _) => ("no-camera", 0),
        (_, State::Waiting) => ("waiting", 0),
        (_, State::Settling { frames_still }) => ("settling", frames_still),
        (_, State::Spent) => ("photographed", 0),
    };

    let settle_frames = scanner.settle_frames();
    let has_baseline = scanner.has_baseline();
    let collecting = scanner.collecting_letters();
    let finishing = scanner.finishing();
    let (table_change, movement) = scanner.measure().unwrap_or((0.0, 0.0));
    hub.update(|status| {
        status.finishing = finishing;
        status.table_change = table_change;
        status.movement = movement;
        status.state = state.to_string();
        status.frames_still = frames_still;
        status.settle_frames = settle_frames;
        status.has_baseline = has_baseline;
        status.collecting = collecting;
        status.open_pages = open_pages;
        status.queue = queue;
        status.settings = settings;
    });
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
