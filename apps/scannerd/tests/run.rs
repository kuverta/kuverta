//! The capture loop, turned by hand.
//!
//! A scripted camera, a spool in a temporary directory and a Paperless on
//! loopback, with the clock passed in. What this checks that the parts' own
//! tests cannot: that a page put down ends up in Paperless, and that a letter
//! photographed while Paperless was unreachable gets there without waiting for
//! the next one.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use scannerd::camera::Camera;
use scannerd::run::{Scanner, Turn};
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

const PIXELS: usize = 320 * 240;

/// An empty desk: mid-grey, with a little sensor noise.
fn desk() -> Vec<u8> {
    (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect()
}

/// A sheet of paper covering half of it.
fn page() -> Vec<u8> {
    let mut frame = desk();
    for pixel in frame.iter_mut().take(PIXELS / 2) {
        *pixel = 235;
    }
    frame
}

/// A camera that shows the frames it is given, in order, and then fails.
struct Script {
    frames: RefCell<VecDeque<Vec<u8>>>,
    captures: Cell<usize>,
}

impl Script {
    fn new(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            frames: RefCell::new(frames.into_iter().collect()),
            captures: Cell::new(0),
        }
    }

    fn then(&self, frames: impl IntoIterator<Item = Vec<u8>>) {
        self.frames.borrow_mut().extend(frames);
    }
}

impl Camera for Script {
    fn preview(&self) -> Result<Vec<u8>> {
        match self.frames.borrow_mut().pop_front() {
            Some(frame) => Ok(frame),
            None => bail!("the script has run out"),
        }
    }

    fn capture(&self, path: &Path) -> Result<()> {
        self.captures.set(self.captures.get() + 1);
        std::fs::write(path, b"\xff\xd8a photograph of a letter")?;
        Ok(())
    }
}

/// A Paperless that refuses the first `failures` uploads and accepts the rest.
/// Returns its URL and how many uploads it has accepted.
fn paperless(failures: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();

    std::thread::spawn(move || {
        let mut failures = failures;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let mut request_line = String::new();
            reader.read_line(&mut request_line).ok();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).ok();

            // Counted before answering, so a send that has returned is counted.
            let (status, reply) = if failures > 0 {
                failures -= 1;
                ("500 Internal Server Error", "consumer is down")
            } else {
                counter.fetch_add(1, SeqCst);
                ("200 OK", "\"task\"")
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });

    (base, accepted)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("scannerd-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Real time, because the spool stamps captures and failures with it.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn uploader(base: &str) -> Uploader {
    Uploader::new(base, "token", vec![], "Post ".into()).unwrap()
}

fn scanner() -> Scanner {
    Scanner::new(3, Duration::from_secs(30))
}

#[tokio::test]
async fn a_page_put_down_ends_up_in_paperless_and_leaves_the_spool() {
    let dir = TempDir::new("page");
    let spool = Spool::open(&dir.0).unwrap();
    let (base, accepted) = paperless(0);
    let uploader = uploader(&base);
    // The empty desk, then a page that is still for three frames.
    let camera = Script::new([desk(), page(), page(), page(), page()]);
    let mut scanner = scanner();
    let start = now();

    let mut turns = Vec::new();
    for second in 0..5 {
        turns.push(
            scanner
                .turn(&camera, &spool, &uploader, start + second)
                .await,
        );
    }

    assert!(
        turns[..4].iter().all(|turn| *turn == Turn::Watching),
        "{turns:?}"
    );
    assert!(matches!(turns[4], Turn::Captured(_)), "{turns:?}");
    assert_eq!(camera.captures.get(), 1);
    assert_eq!(accepted.load(SeqCst), 1);
    assert!(spool.pending().unwrap().is_empty());
}

#[tokio::test]
async fn a_letter_captured_while_paperless_was_down_is_sent_without_waiting_for_the_next_one() {
    // Found reading the loop: the spool was drained at start and after a
    // capture, and at no other time.
    let dir = TempDir::new("retry");
    let spool = Spool::open(&dir.0).unwrap();
    let (base, accepted) = paperless(1);
    let uploader = uploader(&base);
    let camera = Script::new([desk(), page(), page(), page(), page()]);
    let mut scanner = scanner();
    let start = now();

    for second in 0..5 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    assert_eq!(accepted.load(SeqCst), 0);
    assert_eq!(spool.pending().unwrap().len(), 1, "kept for a retry");

    // The page is lifted, and nothing else is ever put down.
    camera.then([desk(), desk()]);

    scanner.turn(&camera, &spool, &uploader, start + 10).await;
    assert_eq!(accepted.load(SeqCst), 0, "not before the drain interval");

    scanner.turn(&camera, &spool, &uploader, start + 40).await;
    assert_eq!(accepted.load(SeqCst), 1, "sent on the timer");
    assert!(spool.pending().unwrap().is_empty());
    assert_eq!(camera.captures.get(), 1);
}

#[tokio::test]
async fn a_broken_camera_does_not_keep_photographed_post_from_being_sent() {
    let dir = TempDir::new("broken-camera");
    let spool = Spool::open(&dir.0).unwrap();
    std::fs::write(dir.0.join(format!("{}-1.jpg", now() - 100)), b"jpeg").unwrap();
    let (base, accepted) = paperless(0);
    let uploader = uploader(&base);
    let camera = Script::new(Vec::<Vec<u8>>::new());
    let mut scanner = scanner();

    let turn = scanner.turn(&camera, &spool, &uploader, now()).await;

    assert_eq!(turn, Turn::NoFrame);
    assert_eq!(accepted.load(SeqCst), 1);
    assert!(spool.pending().unwrap().is_empty());
}

/// A camera whose photographs are real JPEGs, `width`×`height`.
struct Photographer {
    frames: Script,
    width: u32,
    height: u32,
}

impl Camera for Photographer {
    fn preview(&self) -> Result<Vec<u8>> {
        self.frames.preview()
    }

    fn capture(&self, path: &Path) -> Result<()> {
        // With lines of "text" on it: a photograph with nothing on it is
        // taken for the table and put aside rather than kept.
        let picture = image::RgbImage::from_fn(self.width, self.height, |x, y| {
            let printed = y % 10 < 4 && x % 8 < 5;
            let value = if printed { 40 } else { 210 };
            image::Rgb([value, value, value])
        });
        picture.save_with_format(path, image::ImageFormat::Jpeg)?;
        Ok(())
    }
}

#[tokio::test]
async fn a_page_is_straightened_before_it_is_kept() {
    let dir = TempDir::new("straighten");
    let spool = Spool::open(&dir.0).unwrap();
    let (base, _accepted) = paperless(0);
    let uploader = uploader(&base);
    let camera = Photographer {
        frames: Script::new([desk(), page(), page(), page(), page()]),
        width: 400,
        height: 300,
    };
    let mut scanner = scanner().collecting(scannerd::run::Letters {
        button: scannerd::button::Button::channel().1,
        idle: Duration::from_secs(300),
        when_clear: None,
    });
    let corners = "0.25,0,0.75,0,1,1,0,1";
    scanner.straighten_with(Some(scannerd::straighten::Corners::parse(corners).unwrap()));

    let start = now();
    let mut last = Turn::Watching;
    for second in 0..5 {
        last = scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }

    let Turn::Captured(path) = last else {
        panic!("no capture: {last:?}");
    };
    let (width, height) = image::image_dimensions(&path).unwrap();
    // Top 200 and bottom 400 wide; sides hypot(100, 300) long. The picture
    // is as wide as the *widest* edge: the bottom of the page is nearest the
    // camera and has the most pixels on it, and those are the ones the OCR
    // reads.
    assert_eq!((width, height), (400, 100f64.hypot(300.0).round() as u32));
    assert!(
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .all(|entry| entry.unwrap().path().extension().unwrap() == "jpg"),
        "nothing half-done is left beside it"
    );
}

#[tokio::test]
async fn a_page_that_cannot_be_straightened_is_kept_as_taken() {
    let dir = TempDir::new("straighten-fails");
    let spool = Spool::open(&dir.0).unwrap();
    let (base, accepted) = paperless(0);
    let uploader = uploader(&base);
    // Its photographs are not really JPEGs.
    let camera = Script::new([desk(), page(), page(), page(), page()]);
    let mut scanner = scanner();
    scanner.straighten_with(Some(
        scannerd::straighten::Corners::parse("0.25,0,0.75,0,1,1,0,1").unwrap(),
    ));

    let start = now();
    for second in 0..5 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }

    assert_eq!(accepted.load(SeqCst), 1, "sent anyway");
    let events = scanner.take_events();
    assert!(
        events
            .iter()
            .any(|event| !event.ok && event.text.contains("straighten")),
        "{events:?}"
    );
}
