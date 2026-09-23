//! Correcting the exposure: a page too bright to read is taken again darker
//! while it is still lying there, and the next page starts where that ended.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use jpeg_encoder::{ColorType, Encoder};
use scannerd::camera::Camera;
use scannerd::quality::{assess, exposure, Exposure, Problem, Quality};
use scannerd::run::Scanner;
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

fn quality(ink: u8, paper: u8, problems: Vec<Problem>) -> Quality {
    Quality {
        ink,
        paper,
        sharpness: Some(0.9),
        problems,
    }
}

#[test]
fn too_bright_is_taken_again_a_stop_darker() {
    let washed = quality(220, 255, vec![Problem::WashedOut]);
    assert_eq!(exposure(0.0, &washed), Exposure::Retake(-1.0));
    assert_eq!(exposure(-1.0, &washed), Exposure::Retake(-2.0));
    // Not past the darkest.
    assert_eq!(exposure(-4.0, &washed), Exposure::Keep(-4.0));
}

#[test]
fn too_dark_is_taken_again_brighter_only_when_there_is_text() {
    let dark_page = quality(20, 100, vec![Problem::NoPage]);
    assert_eq!(exposure(-3.0, &dark_page), Exposure::Retake(-2.0));
    // A dark middle with nothing in it is the table: not a reason to brighten.
    let table = Quality {
        sharpness: None,
        ..quality(20, 100, vec![Problem::NoPage])
    };
    assert_eq!(exposure(-3.0, &table), Exposure::Keep(-3.0));
}

#[test]
fn a_readable_page_nudges_the_next_toward_comfortable_white() {
    assert_eq!(
        exposure(-1.0, &quality(60, 250, vec![])),
        Exposure::Keep(-1.5)
    );
    assert_eq!(
        exposure(-2.0, &quality(40, 160, vec![])),
        Exposure::Keep(-1.5)
    );
    assert_eq!(
        exposure(-2.0, &quality(84, 210, vec![])),
        Exposure::Keep(-2.0)
    );
}

// -- the loop ---------------------------------------------------------------------------

const W: usize = 600;
const H: usize = 800;
const PIXELS: usize = 320 * 240;

/// A page as the rig's camera took it at `ev`: at its own exposure the paper
/// is pure white and the text pale, and two stops down both are right.
fn page_at(ev: f32) -> Vec<u8> {
    let (paper, ink) = match ev {
        e if e > -0.5 => (255, 220),
        e if e > -1.5 => (255, 180),
        e if e > -2.5 => (210, 60),
        _ => (110, 20),
    };
    (0..W * H)
        .map(|i| {
            let (x, y) = (i % W, i / W);
            if (y % 30) < 10 && (x % 40) < 30 && (x % 12) < 6 {
                ink
            } else {
                paper
            }
        })
        .collect()
}

/// Shows a page to the detector, and photographs it at whatever exposure it
/// is asked for, remembering which.
struct Rig {
    frames: Mutex<Vec<Vec<u8>>>,
    asked: Mutex<Vec<f32>>,
}

impl Camera for Rig {
    fn preview(&self) -> Result<Vec<u8>> {
        let mut frames = self.frames.lock().unwrap();
        Ok(if frames.len() > 1 {
            frames.remove(0)
        } else {
            frames[0].clone()
        })
    }

    fn capture(&self, path: &Path) -> Result<()> {
        self.capture_at(path, 0.0)
    }

    fn capture_at(&self, path: &Path, ev: f32) -> Result<()> {
        self.asked.lock().unwrap().push(ev);
        let mut jpeg = Vec::new();
        Encoder::new(&mut jpeg, 95).encode(&page_at(ev), W as u16, H as u16, ColorType::Luma)?;
        std::fs::write(path, jpeg)?;
        Ok(())
    }
}

fn frames() -> Vec<Vec<u8>> {
    let desk: Vec<u8> = (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect();
    let mut page = desk.clone();
    page[..PIXELS / 2].fill(235);
    vec![desk, page.clone(), page.clone(), page.clone(), page]
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn a_washed_out_page_is_retaken_darker_and_the_next_starts_there() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-exposure-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let rig = Rig {
        frames: Mutex::new(frames()),
        asked: Mutex::new(Vec::new()),
    };
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600));

    for second in 0..5 {
        scanner.turn(&rig, &spool, &uploader, 1_000 + second).await;
    }

    assert_eq!(*rig.asked.lock().unwrap(), vec![0.0, -1.0, -2.0]);
    let last = scanner.last_photographed().expect("photographed");
    assert!(last.quality.as_ref().unwrap().ok(), "{:?}", last.quality);
    assert_eq!(scanner.exposure(), -2.0);
    // One photograph in the spool, the readable one; the retakes are gone.
    let files: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(".attempts") && name != "open")
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let (luma, width, height) =
        scannerd::quality::read_luma(&std::fs::read(dir.0.join(&files[0])).unwrap()).unwrap();
    assert!(assess(&luma, width, height).ok());
    assert!(scanner
        .take_events()
        .iter()
        .any(|event| event.text.contains("2 more times") && event.text.contains("-2.0 EV")));
}

#[tokio::test]
async fn at_the_learnt_exposure_the_page_is_taken_once() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("scannerd-exposure-once-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let rig = Rig {
        frames: Mutex::new(frames()),
        asked: Mutex::new(Vec::new()),
    };
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600));
    scanner.expose_at(-2.0);

    for second in 0..5 {
        scanner.turn(&rig, &spool, &uploader, 1_000 + second).await;
    }
    assert_eq!(*rig.asked.lock().unwrap(), vec![-2.0]);
    assert_eq!(scanner.exposure(), -2.0);
}
