//! The empty table: learnt on purpose, kept between runs, and never guessed.
//!
//! Found on the Pi: started with a page already under the camera, scannerd
//! took the page for the table. Lifting it was photographed as a page, and the
//! detector then waited for that page to come back before it would photograph
//! anything else.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Result};
use scannerd::camera::Camera;
use scannerd::detect::{Detector, State, Step, Thresholds};
use scannerd::run::{Scanner, Turn};
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

const PIXELS: usize = 320 * 240;

fn desk(level: u8) -> Vec<u8> {
    (0..PIXELS).map(|i| level + (i % 3) as u8).collect()
}

fn page() -> Vec<u8> {
    let mut frame = desk(90);
    for pixel in frame.iter_mut().take(PIXELS / 2) {
        *pixel = 235;
    }
    frame
}

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
        std::fs::write(path, b"\xff\xd8photo")?;
        Ok(())
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scannerd-baseline-{}-{name}", std::process::id()));
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

/// Nowhere to upload to: these tests are about the camera, and a refused
/// connection fails fast.
fn uploader() -> Uploader {
    Uploader::new("http://127.0.0.1:9", "token", vec![], String::new()).unwrap()
}

fn scanner(dir: &Path) -> Scanner {
    Scanner::new(3, Duration::from_secs(3600)).keep_baseline_in(dir.join("empty-table.gray"))
}

async fn run(scanner: &mut Scanner, camera: &Script, spool: &Spool) -> Vec<Turn> {
    let uploader = uploader();
    let mut turns = Vec::new();
    let mut clock = 2_000_000_000;
    while !camera.frames.borrow().is_empty() {
        clock += 1;
        turns.push(scanner.turn(camera, spool, &uploader, clock).await);
    }
    turns
}

#[tokio::test]
async fn a_page_on_the_table_at_start_is_photographed_once_the_empty_table_is_kept() {
    let dir = TempDir::new("kept");
    let spool = Spool::open(dir.0.join("spool")).unwrap();

    // First run: the table is empty, and someone says so.
    let camera = Script::new([desk(90)]);
    let mut first = scanner(&dir.0);
    run(&mut first, &camera, &spool).await;
    assert!(!first.has_baseline(), "a first frame is only a guess");
    assert!(first.learn_empty(1));
    assert!(dir.0.join("empty-table.gray").exists());

    // Second run starts with a page lying there.
    let camera = Script::new([page(), page(), page(), page()]);
    let mut second = scanner(&dir.0);
    assert!(second.has_baseline());
    let turns = run(&mut second, &camera, &spool).await;

    assert!(matches!(turns.last(), Some(Turn::Captured(_))), "{turns:?}");
    assert_eq!(camera.captures.get(), 1);
}

#[tokio::test]
async fn a_guessed_empty_table_is_not_kept() {
    let dir = TempDir::new("guess");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    let camera = Script::new([page(), page()]);
    let mut scanner = scanner(&dir.0);

    run(&mut scanner, &camera, &spool).await;

    assert!(!scanner.has_baseline());
    assert!(!dir.0.join("empty-table.gray").exists());
}

#[tokio::test]
async fn learning_the_empty_table_gets_a_stuck_detector_going_again() {
    // The failure from the Pi, reproduced: started on a page, then the page
    // is lifted.
    let dir = TempDir::new("stuck");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    let camera = Script::new([page(), desk(90), desk(90), desk(90), desk(90)]);
    let mut scanner = scanner(&dir.0);
    run(&mut scanner, &camera, &spool).await;

    assert_eq!(
        camera.captures.get(),
        1,
        "the empty table photographed as a page"
    );
    assert_eq!(scanner.state(), State::Spent);
    camera.then([desk(90), desk(90), page(), page(), page(), page()]);
    run(&mut scanner, &camera, &spool).await;
    assert_eq!(
        camera.captures.get(),
        1,
        "and stuck: a real page is not seen"
    );

    // "The table is empty now."
    camera.then([desk(90)]);
    run(&mut scanner, &camera, &spool).await;
    assert!(scanner.learn_empty(3));

    camera.then([page(), page(), page(), page()]);
    let turns = run(&mut scanner, &camera, &spool).await;
    assert!(matches!(turns.last(), Some(Turn::Captured(_))), "{turns:?}");
    assert_eq!(camera.captures.get(), 2);
}

#[tokio::test]
async fn the_table_seen_clear_after_a_photograph_is_kept_as_it_looks_now() {
    // Daylight moves across a table over an afternoon; the kept table follows.
    let dir = TempDir::new("relearn");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    let camera = Script::new([desk(90)]);
    let mut scanner = scanner(&dir.0);
    run(&mut scanner, &camera, &spool).await;
    scanner.learn_empty(1);

    camera.then([page(), page(), page(), page(), desk(100)]);
    run(&mut scanner, &camera, &spool).await;

    assert_eq!(
        std::fs::read(dir.0.join("empty-table.gray")).unwrap(),
        desk(100)
    );
    assert!(scanner.has_baseline());
}

#[tokio::test]
async fn forgetting_the_table_removes_what_was_kept() {
    let dir = TempDir::new("forget");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    let camera = Script::new([desk(90)]);
    let mut scanner = scanner(&dir.0);
    run(&mut scanner, &camera, &spool).await;
    scanner.learn_empty(1);

    scanner.forget_baseline();

    assert!(!scanner.has_baseline());
    assert!(!dir.0.join("empty-table.gray").exists());
}

#[tokio::test]
async fn learning_with_no_picture_yet_says_so() {
    let dir = TempDir::new("no-frame");
    let mut scanner = scanner(&dir.0);
    assert!(!scanner.learn_empty(1));
    let events = scanner.take_events();
    assert_eq!(events.len(), 1);
    assert!(!events[0].ok);
}

#[tokio::test]
async fn what_happened_is_told_as_it_happens() {
    let dir = TempDir::new("events");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    let camera = Script::new([desk(90), page(), page(), page(), page()]);
    let mut scanner = scanner(&dir.0);

    run(&mut scanner, &camera, &spool).await;

    let events = scanner.take_events();
    assert!(
        events
            .iter()
            .any(|e| e.ok && e.text.contains("photographed")),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| !e.ok && e.text.contains("could not send")),
        "an upload to nowhere fails and says so: {events:?}"
    );
    assert!(scanner.take_events().is_empty(), "taken once");
}

#[test]
fn a_kept_table_of_another_size_is_replaced_rather_than_trusted_forever() {
    // `changed_fraction` answers 0 for frames of different sizes, so a stale
    // baseline would be a detector that never sees anything.
    let mut detector = Detector::new(Thresholds::default(), 1);
    detector.learn_baseline(&[90; 100]);

    detector.observe(&desk(90));
    assert_eq!(detector.baseline().map(<[u8]>::len), Some(PIXELS));
    assert_eq!(detector.observe(&page()), Step::Wait);
    assert!(matches!(detector.state(), State::Settling { .. }));
}
