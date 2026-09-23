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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::Parser;
use scannerd::button::Button;
use scannerd::camera::{Camera, RpiCamera};
use scannerd::detect::State;
use scannerd::display::{Confirming, Display, Facts, Screen};
use scannerd::folders::{self, Folder};
use scannerd::hub::{Command, Event, Hub, Queued, SettingsView};
use scannerd::lcd::Bands;
use scannerd::locate::{find_page, find_page_corners};
use scannerd::picture::Picture;
use scannerd::run::{Letters, Scanner, Turn};
use scannerd::settings::{Settings, View};
use scannerd::spool::Pending;
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

    /// For a camera that looks at the table at an angle: the four corners of
    /// a page lying where letters go, as `x,y` fractions of the whole view —
    /// top left, top right, bottom right, bottom left. Every photograph is
    /// straightened so they become a rectangle's, and the crop is the area
    /// they span.
    #[arg(long, env = "SCANNERD_CORNERS")]
    corners: Option<String>,

    /// Turn every photograph this many degrees clockwise — 90, 180 or 270 —
    /// for a camera mounted so that letters do not read upright.
    #[arg(long, env = "SCANNERD_ROTATE")]
    rotate: Option<u16>,

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

    /// How long between turns of the loop. With the preview stream each turn
    /// also waits for the stream's next frame.
    #[arg(long, default_value_t = 50)]
    interval_ms: u64,

    /// Preview frames a second, from a stream kept running between
    /// photographs (`rpicam-vid`). 0: a still for every frame, as before the
    /// stream — half a second each.
    #[arg(long, env = "SCANNERD_PREVIEW_FPS", default_value_t = 5)]
    preview_fps: u32,

    /// Consecutive still frames before a page is photographed: at five frames
    /// a second, a second of stillness.
    #[arg(long, default_value_t = 5)]
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
    #[arg(long, env = "SCANNERD_BUTTON_KEY", default_value_t = scannerd::button::KEY_ENTER)]
    button_key: u16,

    /// Finish a letter once its last page has been taken away and the table
    /// has stayed empty this long. 0: only the button, the display or the
    /// page finish one.
    #[arg(long, env = "SCANNERD_FINISH_WHEN_CLEAR_SECS", default_value_t = 8)]
    finish_when_clear_secs: u64,

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

    /// The folders on the shelf, which the display says a letter goes in once
    /// Paperless has read it — each a Paperless tag of the same name, which
    /// scannerd creates. `-` before a name marks the bin:
    /// `Car,House,Taxes,-Throw away`. The setup page sets words for each.
    #[arg(long, env = "SCANNERD_FOLDERS")]
    folders: Option<String>,

    /// What reads a page on the Pi for the preview — `tesseract` by default,
    /// where it is installed (`apt install tesseract-ocr tesseract-ocr-deu`).
    /// Empty for none: the preview then shows the page without its text, and
    /// the folder only once Paperless has said.
    #[arg(long, env = "SCANNERD_READER", default_value = "tesseract")]
    reader: String,

    /// The languages that reader is given, as it names them.
    #[arg(long, env = "SCANNERD_READER_LANGUAGES", default_value = "deu+eng")]
    reader_languages: String,

    /// The people in the household who get post, so the display can say who a
    /// letter is for as well as which folder it goes in:
    /// `Erika Mustermann,Max Mustermann`. Each is a Paperless tag marked as a
    /// person, matched on their name; the setup page can give someone more
    /// names than one.
    #[arg(long, env = "SCANNERD_PEOPLE")]
    people: Option<String>,

    /// Which folder is the bin, when the folders come from Paperless rather
    /// than from `--folders`: the name of one of them, e.g. `Werbung`.
    #[arg(long, env = "SCANNERD_BIN_FOLDER")]
    bin_folder: Option<String>,

    /// The exposure to start from, in stops from the camera's own choice —
    /// negative is darker. scannerd learns it from the photographs and keeps
    /// what it learnt, which wins over this.
    #[arg(long, env = "SCANNERD_EV", allow_hyphen_values = true)]
    ev: Option<f32>,

    /// Show what to do, whether the last photograph can be read and which
    /// folder a letter goes in: `epaper` for a Waveshare 2.13″ e-paper HAT
    /// (V3/V4; needs `dtparam=spi=on` and the `spi` and `gpio` groups), or a
    /// framebuffer such as `/dev/fb0` for an LCD the kernel drives (the 3.5″
    /// SPI panel with `dtoverlay=piscreen`; needs the `video` group).
    #[arg(long, env = "SCANNERD_DISPLAY")]
    display: Option<String>,

    /// The LCD's touchscreen, read directly: `/dev/spidev0.1` on the 3.5″
    /// panel, with the kernel's ads7846 driver kept off it (see the readme).
    /// With it, scanning is started and stopped on the display: the camera is
    /// only watched between **Start scanning** and the end of a letter.
    #[arg(long, env = "SCANNERD_TOUCH")]
    touch: Option<String>,

    /// Which touch channel follows the screen's rows, and what it reads at the
    /// top and the bottom: `channel:top:bottom`. The rig's panel is x:204:4000.
    #[arg(long, env = "SCANNERD_TOUCH_ROWS")]
    touch_rows: Option<String>,

    /// Turn the e-paper display's picture upside down, for a HAT mounted the
    /// other way round.
    #[arg(long, env = "SCANNERD_DISPLAY_FLIP")]
    display_flip: bool,

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

    let started_folders = folders::parse_env(args.folders.as_deref().unwrap_or_default());
    scannerd::settings::valid_folders(&started_folders).context("SCANNERD_FOLDERS")?;
    let started_people = folders::parse_env_people(args.people.as_deref().unwrap_or_default());
    scannerd::settings::valid_folders(&started_people).context("SCANNERD_PEOPLE")?;

    let spool = Spool::open(&args.spool)?;
    let mut settings = Settings::load(spool.dir())?;
    let mut uploader = uploader_for(&args, &settings)?;
    let mut scanner = Scanner::new(
        args.settle_frames,
        Duration::from_secs(args.drain_every_secs),
    )
    .keep_baseline_in(spool.dir().join("empty-table.gray"));

    // Reading a page here is for the preview only: Paperless is what files a
    // letter, and it reads the whole of it properly when the letter is sent.
    if !args.reader.trim().is_empty() {
        let reader = scannerd::read::Reader::new(args.reader.trim(), &args.reader_languages);
        if reader.available() {
            tracing::info!(program = %args.reader, languages = %args.reader_languages, "reading pages for the preview");
            scanner = scanner.reading_with(reader);
        } else {
            tracing::info!(
                program = %args.reader,
                "no reader: the preview shows the page without its text (apt install tesseract-ocr)"
            );
        }
    }

    tracing::info!(spool = %spool.dir().display(), "starting");

    // Whatever is already waiting goes first, before anything new is taken.
    // A daemon that photographs eagerly and uploads lazily is one that fills a
    // disk with post nobody can search.
    scanner.drain_now(&spool, &uploader, now()).await;

    if args.drain_only {
        return Ok(());
    }

    // Nobody may have said which folders there are — the desktop app's
    // assistant sets them up in Paperless, and the scanner is the other thing
    // that reads them.
    let mut adopted = if nothing_says_which_folders(&args, &settings) {
        adopt_folders(&args, &uploader).await
    } else {
        Vec::new()
    };
    let mut adopted_at = now();
    if !adopted.is_empty() {
        tracing::info!(count = adopted.len(), "folders taken from Paperless");
    }
    scanner.file_into(folders_for(&args, &settings, &adopted));
    scanner.expose_at(settings.ev.or(args.ev).unwrap_or(0.0));

    let mut view = view_for(&args, &settings)?;
    scanner.straighten_with(view.warp());
    let mut camera = RpiCamera::default();
    camera.program = args.camera.clone();
    camera.roi = view.roi.clone();
    camera.sensor_width = args.sensor_width;
    camera.sensor_height = args.sensor_height;
    // The stream is rpicam-vid's, beside rpicam-still. Another camera program
    // — the ffmpeg stand-in on a laptop — takes a still for every frame.
    if args.preview_fps > 0 && args.camera == "rpicam-still" {
        camera.video_program = Some("rpicam-vid".to_string());
        camera.stream_fps = args.preview_fps;
    }
    scanner.frame_width(camera.preview_width);
    let hub = Arc::new(Hub::new(camera.preview_width, camera.preview_height));
    let (display, bands) = match args
        .display
        .as_deref()
        .and_then(|kind| open_display(kind, args.display_flip))
    {
        Some((display, bands)) => (Some(display), bands),
        None => (None, None),
    };
    // A display you can touch starts and stops scanning; without one the
    // camera is always watched.
    let touch = match (&args.touch, bands) {
        (Some(device), Some((bands, height))) => open_touch(device, &args, bands, height, &hub),
        (Some(_), None) => {
            tracing::error!("no touchscreen: it needs the LCD (SCANNERD_DISPLAY=/dev/fb0)");
            false
        }
        (None, _) => false,
    };
    // Watching from the start, touch or not: a page put down after a boot is
    // photographed without anyone pressing anything. Stop scanning on the
    // display, or the page, stops it.
    scanner.watch(true);
    // What the last button that works at once came to, and when.
    let mut notice: Option<(u64, bool, &str)> = None;
    // A button on the display waiting for its second tap, and since when.
    let mut confirming: Option<(Confirming, u64)> = None;
    // Whether the display is showing what is waiting to be sent.
    let mut showing_queue = false;
    if let Some(display) = &display {
        display.show(Screen {
            headline: "Starting".into(),
            ..Screen::default()
        });
    }

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
            when_clear: (args.finish_when_clear_secs > 0)
                .then(|| Duration::from_secs(args.finish_when_clear_secs)),
        });
    }

    let interval = Duration::from_millis(args.interval_ms);
    loop {
        tokio::time::sleep(interval).await;
        let now = now();

        // Paperless may not have been up when the shelf was asked for — it
        // often is not, a minute after a power cut that took both machines
        // down. Asked for again now and then, until there is one.
        if adopted.is_empty()
            && nothing_says_which_folders(&args, &settings)
            && now.saturating_sub(adopted_at) >= ADOPT_FOLDERS_EVERY_SECS
        {
            adopted_at = now;
            adopted = adopt_folders(&args, &uploader).await;
            if !adopted.is_empty() {
                tracing::info!(count = adopted.len(), "folders taken from Paperless");
                hub.event(
                    now,
                    true,
                    format!("{} folders taken from Paperless", adopted.len()),
                );
                scanner.file_into(folders_for(&args, &settings, &adopted));
            }
        }

        // Between turns, never during one: the loop owns the camera.
        for command in hub.take_commands() {
            match command {
                Command::FinishLetter => {
                    if !scanner.request_close(now) {
                        hub.event(now, false, "pages are not being collected into letters");
                    }
                }
                Command::LearnEmpty => {
                    // From a frame taken now: while scanning is stopped there
                    // is no recent one.
                    notice = Some(if scanner.learn_empty_now(&camera, now) {
                        (now, true, "Learnt the empty table")
                    } else {
                        (now, false, "Could not see the table - no camera?")
                    });
                }
                Command::ShowQueue(showing) => {
                    showing_queue = showing;
                    confirming = None;
                }
                Command::DiscardQueued { which, confirmed } => {
                    let name = match &which {
                        scannerd::hub::Which::Named(name) => Some(name.clone()),
                        scannerd::hub::Which::At(index) => spool
                            .pending()
                            .ok()
                            .and_then(|pending| pending.get(*index).cloned())
                            .and_then(|item| {
                                item.path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                            }),
                    };
                    match (name, confirmed) {
                        (Some(_), false) => {
                            if let scannerd::hub::Which::At(index) = which {
                                confirming = Some((Confirming::Queued(index), now));
                            }
                        }
                        (Some(name), true) => {
                            confirming = None;
                            scanner.discard_queued(&spool, &name, now);
                            if spool.pending().map(|left| left.is_empty()).unwrap_or(true) {
                                showing_queue = false;
                            }
                        }
                        (None, _) => hub.event(now, false, "that letter is not waiting any more"),
                    }
                }
                Command::UndoPage => {
                    scanner.undo_page(&spool, now);
                }
                Command::CancelLetter { confirmed: false } => {
                    confirming = Some((Confirming::Cancel, now));
                }
                Command::CancelLetter { confirmed: true } => {
                    confirming = None;
                    scanner.cancel_letter(&spool, now);
                }
                Command::UndoLetter { confirmed: false } => {
                    confirming = Some((Confirming::UndoLetter, now));
                }
                Command::UndoLetter { confirmed: true } => {
                    confirming = None;
                    scanner.undo_letter(&spool, &uploader, now).await;
                }
                Command::StartScanning => {
                    if !scanner.watching() {
                        scanner.watch(true);
                        hub.event(now, true, "scanning started");
                    }
                }
                Command::StopScanning => {
                    if scanner.watching() {
                        scanner.watch(false);
                        hub.event(now, true, "scanning stopped");
                    }
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
                Command::Refile(into) => {
                    scanner.refile_letter(&uploader, into, now).await;
                }
                Command::RetryNow => scanner.retry_all(&spool, &uploader, now).await,
                Command::FullView => {
                    // Found in a greyscale frame of the same view: there is no
                    // JPEG decoder in this binary, and needs to be none.
                    let (width, height) = (
                        camera.preview_width as usize,
                        camera.preview_height as usize,
                    );
                    let luma = camera.full_view().ok();
                    let crop = luma
                        .as_deref()
                        .and_then(|luma| find_page(luma, width, height))
                        .map(|area| area.to_roi());
                    let corners = luma
                        .as_deref()
                        .and_then(|luma| find_page_corners(luma, width, height))
                        .map(|corners| corners.to_setting());
                    match camera.snapshot() {
                        Ok(jpeg) => hub.set_full_view(jpeg, crop, corners),
                        Err(err) => hub.event(
                            now,
                            false,
                            format!("could not take a picture of the whole view: {err:#}"),
                        ),
                    }
                }
                Command::CheckPaperless => {
                    let (ok, text) = match uploader.check().await {
                        Ok(text) if folders_for(&args, &settings, &adopted).is_empty() => {
                            (true, text)
                        }
                        // The folders' tags are made or brought up to date
                        // now, rather than when the next letter is sent.
                        Ok(text) => match scanner.sync_folders(&uploader).await {
                            Ok(count) => (true, format!("{text}; {count} folders set up")),
                            Err(err) => (false, format!("{text}; but the folders: {err:#}")),
                        },
                        Err(err) => (false, format!("{err:#}")),
                    };
                    hub.update(|status| status.check = Some(Event { at: now, ok, text }));
                }
                Command::Settings(newer) => {
                    if let Some(ev) = newer.ev {
                        scanner.expose_at(ev);
                    }
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
                    // Set up again before the next letter: the words, or the
                    // Paperless, may be new — including its shelf, when the
                    // folders are the ones Paperless holds.
                    if nothing_says_which_folders(&args, &settings) {
                        adopted = adopt_folders(&args, &uploader).await;
                        adopted_at = now;
                    } else {
                        adopted.clear();
                    }
                    scanner.file_into(folders_for(&args, &settings, &adopted));
                    let newer = match view_for(&args, &settings) {
                        Ok(newer) => newer,
                        Err(err) => {
                            hub.event(now, false, format!("{err:#}"));
                            view.clone()
                        }
                    };
                    scanner.straighten_with(newer.warp());
                    let moved = newer.roi != view.roi;
                    view = newer;
                    if moved {
                        camera.roi = view.roi.clone();
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

        // What the photographs taught about the exposure, kept for the next
        // run.
        let ev = scanner.exposure();
        if settings.ev.unwrap_or(0.0) != ev {
            settings.ev = Some(ev);
            if let Err(err) = settings.save(spool.dir()) {
                tracing::warn!("could not keep the exposure: {err:#}");
            }
        }

        // Stopped, the camera is left alone: no stream running for nobody.
        if !scanner.watching() {
            camera.stop_stream();
        }

        if let Some(display) = &display {
            let live = scanner
                .last_frame()
                .filter(|_| scanner.watching() && turn != Turn::NoFrame)
                .map(|frame| live_view(frame, &camera, &view));
            let waiting: Vec<Queued> = spool
                .pending()
                .unwrap_or_default()
                .iter()
                .map(queued)
                .collect();
            display.show(Screen::for_facts(&Facts {
                now,
                state: state_name(&turn, scanner.state()),
                finishing: scanner.finishing(),
                collecting: scanner.collecting_letters(),
                open_pages: spool.open_pages().map(|pages| pages.len()).unwrap_or(0),
                waiting: spool.pending().map(|pending| pending.len()).unwrap_or(0),
                last: scanner.last_photographed(),
                filing: scanner.filing(),
                touch,
                scanning: scanner.watching(),
                live,
                notice: notice
                    .filter(|(at, _, _)| now.saturating_sub(*at) < NOTICE_SECS)
                    .map(|(_, ok, words)| (ok, words)),
                finishing_in: scanner.finishing_in(now),
                can_undo_letter: scanner.can_undo_letter(now),
                can_refile: scanner.can_refile(now),
                queue: &waiting,
                showing_queue,
                confirming: confirming
                    .filter(|(_, at)| now.saturating_sub(*at) < CONFIRM_SECS)
                    .map(|(what, _)| what),
            }));
        }
        // The page says it too, whichever of them the button was on.
        {
            let fresh = notice.filter(|(at, _, _)| now.saturating_sub(*at) < NOTICE_SECS);
            hub.update(|status| {
                status.notice = fresh.map(|(at, ok, words)| Event {
                    at,
                    ok,
                    text: words.to_string(),
                })
            });
        }

        if args.ui.is_some() {
            let view = SettingsView {
                url: settings.url.clone().unwrap_or_else(|| args.url.clone()),
                tags: settings.tags.clone().unwrap_or_else(|| args.tags.clone()),
                roi: camera.roi.clone(),
                corners: view.corners.as_ref().map(|corners| corners.to_setting()),
                rotate: view.rotate,
                token_set: settings.token.is_some()
                    || args.token.as_deref().is_some_and(|t| !t.is_empty()),
                folders: folders_for(&args, &settings, &adopted),
                ev: scanner.exposure(),
            };
            publish(&hub, &mut scanner, &spool, view, &turn);
        } else {
            scanner.take_events();
        }
    }
}

/// What the camera looks at, for the settings in force: the page's, then the
/// env file's.
fn view_for(args: &Args, settings: &Settings) -> Result<View> {
    settings
        .effective_view(args.roi.as_deref(), args.corners.as_deref(), args.rotate)
        .context("the crop, corners or turn are not usable")
}

/// How often a scanner with no folders of its own asks Paperless for the
/// shelf again.
const ADOPT_FOLDERS_EVERY_SECS: u64 = 300;

/// The folders in force: the page's, then the env file's, then the shelf
/// Paperless already has.
///
/// An empty list saved on the page is a choice — follow letters into no
/// folder at all — and is not overruled by what Paperless holds; only a
/// scanner that was never told anything adopts.
fn folders_for(args: &Args, settings: &Settings, adopted: &[Folder]) -> Vec<Folder> {
    let mut started = folders::parse_env(args.folders.as_deref().unwrap_or_default());
    started.extend(folders::parse_env_people(
        args.people.as_deref().unwrap_or_default(),
    ));
    let folders = settings.effective_folders(&started);
    if folders.is_empty() && nothing_says_which_folders(args, settings) {
        return adopted.to_vec();
    }
    folders
}

/// Whether neither the env file nor the setup page names any folder.
fn nothing_says_which_folders(args: &Args, settings: &Settings) -> bool {
    settings.folders.is_none()
        && folders::parse_env(args.folders.as_deref().unwrap_or_default()).is_empty()
        && folders::parse_env_people(args.people.as_deref().unwrap_or_default()).is_empty()
}

/// The shelf Paperless already has, for a scanner nobody told which folders
/// there are — the desktop app's assistant sets them up there. Which of them
/// is the bin is the one thing a tag cannot say, so `--bin-folder` names it.
///
/// A Paperless that cannot be reached is not an error worth stopping for:
/// this is tried again, and until then letters are filed without a folder.
async fn adopt_folders(args: &Args, uploader: &Uploader) -> Vec<Folder> {
    let bin = args.bin_folder.as_deref().unwrap_or_default().trim();
    match uploader.folders_in_paperless().await {
        Ok(folders) => folders
            .into_iter()
            .map(|folder| Folder {
                discard: !bin.is_empty() && folder.name.eq_ignore_ascii_case(bin),
                ..folder
            })
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "could not read the folders from Paperless");
            Vec::new()
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
        .map(queued)
        .collect();
    let state = state_name(turn, scanner.state());
    let scanning = scanner.watching();
    let can_undo_letter = scanner.can_undo_letter(now());
    let frames_still = match scanner.state() {
        State::Settling { frames_still } => frames_still,
        _ => 0,
    };

    let read_pages = scanner.read_pages().to_vec();
    let guess = scanner
        .guessed()
        .iter()
        .map(scannerd::hub::GuessedFolder::from)
        .collect();
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
        status.read_pages = read_pages;
        status.guess = guess;
        status.settle_frames = settle_frames;
        status.has_baseline = has_baseline;
        status.collecting = collecting;
        status.scanning = scanning;
        status.can_undo_letter = can_undo_letter;
        status.open_pages = open_pages;
        status.queue = queue;
        status.settings = settings;
    });
}

/// What the camera sees, as the photograph will show it: in the crop's own
/// proportions, straightened by the page's corners when there are any, and
/// turned — so the live view and the last photograph beside it are the same
/// shape and the same way up.
fn live_view(frame: &[u8], camera: &RpiCamera, view: &View) -> Picture {
    let (width, height) = (
        camera.preview_width as usize,
        camera.preview_height as usize,
    );
    // The preview is 4:3 whatever the crop, so it is stretched; the crop's
    // own shape is the photograph's. Without corners the whole crop is the
    // page, which still puts it in shape and turns it.
    let (crop_w, crop_h) = camera
        .capture_size()
        .unwrap_or((camera.sensor_width, camera.sensor_height));
    let corners = view.warp().unwrap_or(scannerd::straighten::Corners::WHOLE);
    scannerd::straighten::straighten_luma(
        frame,
        width,
        height,
        crop_w as f64 / crop_h as f64,
        &corners,
        camera.preview_width,
        camera.preview_width,
    )
}

/// How long a button waits for its second tap.
const CONFIRM_SECS: u64 = 6;

/// How long the display says what came of a button.
const NOTICE_SECS: u64 = 10;

/// A letter waiting to be sent, as the page and the display see it.
fn queued(item: &Pending) -> Queued {
    Queued {
        name: item
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        attempts: item.attempts,
        at: item.captured_at,
    }
}

/// The loop's state as the page and the display name it.
fn state_name(turn: &Turn, state: State) -> &'static str {
    match (turn, state) {
        (Turn::NoFrame, _) => "no-camera",
        (Turn::Paused, _) => "paused",
        (_, State::Waiting) => "waiting",
        (_, State::Settling { .. }) => "settling",
        (_, State::Spent) => "photographed",
    }
}

/// The display, if it answers — and for an LCD, where its buttons are and how
/// tall it is, for the touchscreen. A display that does not answer is logged
/// and done without: it shows what the loop does, and is no reason to stop it.
fn open_display(kind: &str, flip: bool) -> Option<(Display, Option<(Bands, u32)>)> {
    let opened = match kind.trim() {
        "" | "false" | "none" => return None,
        // `true` is what the env file said when the e-paper was the only one.
        "epaper" | "true" => open_epaper(flip).map(|display| (display, None)),
        path if path.starts_with('/') => {
            scannerd::lcd::Framebuffer::open(Path::new(path)).map(|framebuffer| {
                let touch = (framebuffer.bands(), framebuffer.height());
                (Display::spawn(Box::new(framebuffer)), Some(touch))
            })
        }
        other => Err(anyhow::anyhow!(
            "{other:?} is not a display: `epaper`, or a framebuffer such as /dev/fb0"
        )),
    };
    match opened {
        Ok(display) => {
            tracing::info!(display = kind, "display");
            Some(display)
        }
        Err(err) => {
            tracing::error!("no display: {err:#}");
            None
        }
    }
}

/// The touchscreen, each tap sent to the loop as whatever button it fell on.
/// False, and logged, when it cannot be read: the display still shows, and
/// the camera is watched all the time as without one.
fn open_touch(device: &str, args: &Args, bands: Bands, height: u32, hub: &Arc<Hub>) -> bool {
    let rows = match args.touch_rows.as_deref() {
        Some(text) => match scannerd::touch::Rows::parse(text) {
            Ok(rows) => rows,
            Err(err) => {
                tracing::error!("no touchscreen: SCANNERD_TOUCH_ROWS: {err:#}");
                return false;
            }
        },
        None => scannerd::touch::Rows::RIG,
    };
    let hub = hub.clone();
    let tapped = move |row: i32| {
        let action = scannerd::touch::hit(&bands.lock().unwrap(), row);
        tracing::info!(row, ?action, "tap");
        let command = match action {
            Some(scannerd::display::Action::StartScanning) => Command::StartScanning,
            Some(scannerd::display::Action::StopScanning) => Command::StopScanning,
            Some(scannerd::display::Action::FinishLetter(_)) => Command::FinishLetter,
            Some(scannerd::display::Action::LearnEmpty) => Command::LearnEmpty,
            Some(scannerd::display::Action::UndoPage) => Command::UndoPage,
            Some(scannerd::display::Action::CancelLetter(_)) => {
                Command::CancelLetter { confirmed: false }
            }
            Some(scannerd::display::Action::ConfirmCancel(_)) => {
                Command::CancelLetter { confirmed: true }
            }
            Some(scannerd::display::Action::UndoLetter) => Command::UndoLetter { confirmed: false },
            Some(scannerd::display::Action::ConfirmUndoLetter) => {
                Command::UndoLetter { confirmed: true }
            }
            Some(scannerd::display::Action::FileNowhere) => {
                Command::Refile(scannerd::folders::Refiling::Nowhere)
            }
            Some(scannerd::display::Action::FileInBin) => {
                Command::Refile(scannerd::folders::Refiling::Bin)
            }
            Some(scannerd::display::Action::OpenQueue(_)) => Command::ShowQueue(true),
            Some(scannerd::display::Action::Back) => Command::ShowQueue(false),
            Some(scannerd::display::Action::QueuedLetter(index)) => Command::DiscardQueued {
                which: scannerd::hub::Which::At(index),
                confirmed: false,
            },
            Some(scannerd::display::Action::ConfirmQueued(index)) => Command::DiscardQueued {
                which: scannerd::hub::Which::At(index),
                confirmed: true,
            },
            None => return,
        };
        hub.send(command);
    };
    match watch_touch(device, rows, height, tapped) {
        Ok(()) => {
            tracing::info!(device, "touchscreen");
            true
        }
        Err(err) => {
            tracing::error!("no touchscreen: {err:#}");
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn watch_touch(
    device: &str,
    rows: scannerd::touch::Rows,
    height: u32,
    tapped: impl FnMut(i32) + Send + 'static,
) -> Result<()> {
    scannerd::touch::watch(device, rows, height, tapped)
}

#[cfg(not(target_os = "linux"))]
fn watch_touch(
    _device: &str,
    _rows: scannerd::touch::Rows,
    _height: u32,
    _tapped: impl FnMut(i32) + Send + 'static,
) -> Result<()> {
    bail!("the touchscreen is read on Linux only")
}

#[cfg(target_os = "linux")]
fn open_epaper(flip: bool) -> Result<Display> {
    let panel = scannerd::epaper::Ssd1680::open()?;
    Ok(Display::spawn(Box::new(scannerd::display::Epaper::new(
        Box::new(panel),
        flip,
    ))))
}

#[cfg(not(target_os = "linux"))]
fn open_epaper(_flip: bool) -> Result<Display> {
    bail!("the e-paper HAT is driven on Linux only")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
