//! Pages collected into letters, closed by the button or by being left alone.
//!
//! The loop turned by hand, as in `run.rs`, with a button that is a channel.
//! What matters: a letter of several pages reaches Paperless as one PDF, and
//! only once it is finished; a forgotten press delays post but never keeps it;
//! and the press that comes with putting the last page down does not close the
//! letter a moment before that page is photographed.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use scannerd::button::Button;
use scannerd::camera::Camera;
use scannerd::run::{Letters, Scanner, Turn};
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

const PIXELS: usize = 320 * 240;

fn desk() -> Vec<u8> {
    (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect()
}

fn page() -> Vec<u8> {
    let mut frame = desk();
    for pixel in frame.iter_mut().take(PIXELS / 2) {
        *pixel = 235;
    }
    frame
}

/// Put down, and still for long enough to be photographed.
fn a_page() -> Vec<Vec<u8>> {
    vec![page(), page(), page(), page()]
}

/// Taken away again.
fn lifted() -> Vec<Vec<u8>> {
    vec![desk(), desk()]
}

/// A frame header and nothing else: all a PDF needs to read from a page.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 0xFF, 0xD9]);
    bytes
}

struct Script {
    frames: RefCell<VecDeque<Vec<u8>>>,
    captures: Cell<usize>,
}

impl Script {
    fn new() -> Self {
        Self {
            frames: RefCell::new(VecDeque::from([desk()])),
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
        std::fs::write(path, jpeg(2480, 3508))?;
        Ok(())
    }
}

/// What the fake Paperless has been sent.
#[derive(Clone, Default)]
struct Received {
    uploads: Arc<AtomicUsize>,
    pdfs: Arc<AtomicUsize>,
    /// Pages in the most recent PDF.
    pages: Arc<AtomicUsize>,
}

fn paperless() -> (String, Received) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let received = Received::default();
    let record = received.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
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
            let body = String::from_utf8_lossy(&body);

            record.uploads.fetch_add(1, SeqCst);
            if body.contains("Content-Type: application/pdf") {
                record.pdfs.fetch_add(1, SeqCst);
                record
                    .pages
                    .store(body.matches("/Type /Page ").count(), SeqCst);
            }
            let reply = "\"task\"";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });

    (base, received)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scannerd-letters-{}-{name}", std::process::id()));
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Rig {
    _dir: TempDir,
    spool: Spool,
    uploader: Uploader,
    received: Received,
    camera: Script,
    scanner: Scanner,
    press: Sender<()>,
    clock: u64,
}

impl Rig {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let spool = Spool::open(&dir.0).unwrap();
        let (base, received) = paperless();
        let (press, button) = Button::channel();
        let scanner = Scanner::new(3, Duration::from_secs(30)).collecting(Letters {
            button,
            idle: Duration::from_secs(300),
        });
        Self {
            uploader: Uploader::new(&base, "token", vec![], "Post ".into()).unwrap(),
            _dir: dir,
            spool,
            received,
            camera: Script::new(),
            scanner,
            press,
            clock: now(),
        }
    }

    /// Turns until the script runs out, a second of clock per turn.
    async fn run(&mut self) -> Vec<Turn> {
        let mut turns = Vec::new();
        while !self.camera.frames.borrow().is_empty() {
            turns.push(self.step().await);
        }
        turns
    }

    async fn step(&mut self) -> Turn {
        self.clock += 1;
        self.scanner
            .turn(&self.camera, &self.spool, &self.uploader, self.clock)
            .await
    }
}

#[tokio::test]
async fn pages_wait_for_the_button_and_go_as_one_pdf() {
    let mut rig = Rig::new("button");
    rig.camera
        .then([a_page(), lifted(), a_page(), lifted()].concat());
    rig.run().await;

    assert_eq!(rig.camera.captures.get(), 2);
    assert_eq!(rig.spool.open_pages().unwrap().len(), 2);
    assert!(
        rig.spool.pending().unwrap().is_empty(),
        "nothing queued yet"
    );
    assert_eq!(
        rig.received.uploads.load(SeqCst),
        0,
        "nothing sent a page at a time"
    );

    rig.press.send(()).unwrap();
    rig.camera.then([desk()]);
    rig.run().await;

    assert_eq!(rig.received.uploads.load(SeqCst), 1);
    assert_eq!(rig.received.pdfs.load(SeqCst), 1);
    assert_eq!(rig.received.pages.load(SeqCst), 2);
    assert!(rig.spool.open_pages().unwrap().is_empty());
    assert!(rig.spool.pending().unwrap().is_empty());
}

#[tokio::test]
async fn a_letter_nobody_closes_is_sent_once_left_alone() {
    let mut rig = Rig::new("idle");
    rig.camera.then([a_page(), lifted()].concat());
    rig.run().await;

    rig.clock += 100;
    rig.camera.then([desk()]);
    rig.run().await;
    assert_eq!(
        rig.received.uploads.load(SeqCst),
        0,
        "a hundred seconds is not idle"
    );

    rig.clock += 300;
    rig.camera.then([desk()]);
    rig.run().await;
    assert_eq!(rig.received.pdfs.load(SeqCst), 1);
    assert_eq!(rig.received.pages.load(SeqCst), 1);
}

#[tokio::test]
async fn a_press_while_the_last_page_is_settling_waits_for_that_page() {
    let mut rig = Rig::new("settling");
    rig.camera.then([a_page(), lifted()].concat());
    rig.run().await;

    // The last page goes down, and the button is pressed before it is still.
    rig.camera.then([page()]);
    rig.run().await;
    rig.press.send(()).unwrap();
    rig.camera.then([page(), page()]);
    assert_eq!(rig.run().await, vec![Turn::Watching, Turn::Watching]);
    assert_eq!(
        rig.received.uploads.load(SeqCst),
        0,
        "not closed mid-settle"
    );

    rig.camera.then([page(), desk()]);
    let turns = rig.run().await;
    assert!(matches!(turns[0], Turn::Captured(_)), "{turns:?}");

    assert_eq!(rig.received.pdfs.load(SeqCst), 1);
    assert_eq!(rig.received.pages.load(SeqCst), 2, "the last page went in");
}

#[tokio::test]
async fn a_press_with_nothing_photographed_does_not_close_the_next_letter_early() {
    let mut rig = Rig::new("empty-press");
    rig.press.send(()).unwrap();
    rig.camera.then([desk()]);
    rig.run().await;
    assert_eq!(rig.received.uploads.load(SeqCst), 0);

    rig.camera.then([a_page(), lifted()].concat());
    rig.run().await;
    assert_eq!(
        rig.spool.open_pages().unwrap().len(),
        1,
        "still open, waiting for more"
    );
    assert_eq!(rig.received.uploads.load(SeqCst), 0);
}

#[test]
fn a_page_that_is_not_a_readable_jpeg_is_queued_on_its_own_rather_than_blocking_the_letter() {
    let dir = TempDir::new("bad-page");
    let spool = Spool::open(&dir.0).unwrap();
    for bytes in [
        jpeg(100, 140),
        b"half a photograph".to_vec(),
        jpeg(100, 140),
    ] {
        let (partial, ready) = spool.reserve_page().unwrap();
        std::fs::write(&partial, bytes).unwrap();
        spool.commit(&partial, &ready).unwrap();
    }

    let letter = spool
        .close_letter()
        .unwrap()
        .expect("the readable pages make a letter");

    assert!(spool.open_pages().unwrap().is_empty());
    let queued: Vec<String> = spool
        .pending()
        .unwrap()
        .iter()
        .map(|item| {
            item.path
                .extension()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(queued.len(), 2, "{queued:?}");
    assert!(queued.contains(&"pdf".to_string()) && queued.contains(&"jpg".to_string()));
    let pdf = std::fs::read(letter).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&pdf)
            .matches("/Type /Page ")
            .count(),
        2
    );
}

#[test]
fn closing_with_no_pages_makes_nothing() {
    let dir = TempDir::new("no-pages");
    let spool = Spool::open(&dir.0).unwrap();
    assert_eq!(spool.close_letter().unwrap(), None);
    assert!(spool.pending().unwrap().is_empty());
}
