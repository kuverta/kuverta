//! Scanning started and stopped on the display: stopped, the camera is left
//! alone; the empty table can still be learnt.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

use anyhow::Result;
use scannerd::camera::Camera;
use scannerd::run::{Scanner, Turn};
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

const PIXELS: usize = 320 * 240;

/// A camera that counts how often it is asked, and always shows paper.
struct Counting(AtomicUsize);

impl Camera for Counting {
    fn preview(&self) -> Result<Vec<u8>> {
        self.0.fetch_add(1, SeqCst);
        Ok(vec![235; PIXELS])
    }

    fn capture(&self, path: &Path) -> Result<()> {
        std::fs::write(path, b"\xff\xd8a page")?;
        Ok(())
    }
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn stopped_the_camera_is_not_asked_for_a_frame_and_nothing_is_photographed() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-scanning-{}", std::process::id())));
    let spool = Spool::open(&dir.0).unwrap();
    // Nothing listens here: sending fails, which a paused turn must survive.
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let camera = Counting(AtomicUsize::new(0));
    let mut scanner = Scanner::new(3, Duration::from_secs(30));
    scanner.watch(false);

    for second in 0..10 {
        assert_eq!(
            scanner
                .turn(&camera, &spool, &uploader, 1_000 + second)
                .await,
            Turn::Paused
        );
    }
    assert_eq!(camera.0.load(SeqCst), 0);
    assert!(spool.pending().unwrap().is_empty());

    // Learning the table takes a frame for the purpose.
    assert!(scanner.learn_empty_now(&camera, 1_010));
    assert_eq!(camera.0.load(SeqCst), 1);
    assert!(scanner.has_baseline());

    scanner.watch(true);
    assert_eq!(
        scanner.turn(&camera, &spool, &uploader, 1_011).await,
        Turn::Watching
    );
    assert_eq!(camera.0.load(SeqCst), 2);
}
