//! Finishing a letter: acknowledged at once, and never waiting for ever.
//!
//! Found on the Pi: with the room in view something was always moving, a
//! finish waits while a page settles, and so the press on the page did nothing
//! — silently — until the letter closed itself five minutes later.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use scannerd::button::Button;
use scannerd::camera::Camera;
use scannerd::detect::State;
use scannerd::run::{Letters, Scanner, FINISH_WAIT_SECS};
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

/// A page with a hand moving over part of it.
fn hand() -> Vec<u8> {
    let mut frame = page();
    for pixel in frame.iter_mut().skip(PIXELS / 10).take(PIXELS / 5) {
        *pixel = 30;
    }
    frame
}

/// A frame header and nothing else: all the PDF needs from a page.
fn jpeg() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    bytes.extend_from_slice(&3508u16.to_be_bytes());
    bytes.extend_from_slice(&2480u16.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 0xFF, 0xD9]);
    bytes
}

struct Script(RefCell<VecDeque<Vec<u8>>>);

impl Camera for Script {
    fn preview(&self) -> Result<Vec<u8>> {
        match self.0.borrow_mut().pop_front() {
            Some(frame) => Ok(frame),
            None => bail!("the script has run out"),
        }
    }

    fn capture(&self, path: &Path) -> Result<()> {
        std::fs::write(path, jpeg())?;
        Ok(())
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scannerd-finishing-{}-{name}", std::process::id()));
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

#[tokio::test]
async fn a_finish_is_acknowledged_at_once_and_given_up_waiting_on_movement() {
    let dir = TempDir::new("movement");
    let spool = Spool::open(&dir.0).unwrap();
    // Nowhere to send to: the letter stays queued, which is what is checked.
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], String::new()).unwrap();
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3600)).collecting(Letters {
        button,
        idle: Duration::from_secs(300),
    });

    // One page photographed and lifted, then a hand that never keeps still.
    let camera = Script(RefCell::new(
        [desk(), page(), page(), page(), page(), desk()]
            .into_iter()
            .chain((0..40).map(|i| if i % 2 == 0 { page() } else { hand() }))
            .collect(),
    ));
    // Real time, not a made-up clock: pages are named for when they were
    // taken, and a letter whose last page looks years old is "left alone" and
    // closes on the next turn — which is what this test's first run did.
    let mut clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for _ in 0..7 {
        clock += 1;
        scanner.turn(&camera, &spool, &uploader, clock).await;
    }
    assert_eq!(spool.open_pages().unwrap().len(), 1);
    assert!(matches!(scanner.state(), State::Settling { .. }));
    scanner.take_events();

    let asked = clock;
    assert!(scanner.request_close(asked));
    assert!(scanner.finishing());
    let events = scanner.take_events();
    assert!(
        events.iter().any(|e| e.text.contains("finishing")),
        "{events:?}"
    );

    while clock < asked + FINISH_WAIT_SECS - 1 {
        clock += 1;
        scanner.turn(&camera, &spool, &uploader, clock).await;
        assert_eq!(
            spool.open_pages().unwrap().len(),
            1,
            "waiting at {}s",
            clock - asked
        );
        assert!(scanner.finishing());
    }

    clock += 1;
    scanner.turn(&camera, &spool, &uploader, clock).await;
    assert!(
        spool.open_pages().unwrap().is_empty(),
        "closed after {}s",
        clock - asked
    );
    assert!(!scanner.finishing());
    let queued = spool.pending().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].path.extension().unwrap(), "pdf");
}

#[tokio::test]
async fn finishing_without_letters_says_no() {
    let mut scanner = Scanner::new(3, Duration::from_secs(3600));
    assert!(!scanner.request_close(1));
    assert!(!scanner.finishing());
}
