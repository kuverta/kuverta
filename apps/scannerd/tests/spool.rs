//! The spool, which is where "nothing is lost" is actually implemented.
//!
//! §3.5 says queue the intent before acting. Here the intent is a photograph
//! and the acting is an upload, so the capture is on disk before Paperless is
//! ever contacted and stays there until Paperless confirms. Everything below
//! is a way of asking whether that survives the things that actually happen to
//! a Pi: a power cut mid-write, a network that is down for a day, a restart.

use std::fs;
use std::path::PathBuf;

use scannerd::spool::{retry_delay, Pending, Spool};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("scannerd-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn capture(spool: &Spool, bytes: &[u8]) -> PathBuf {
    let (partial, ready) = spool.reserve();
    fs::write(&partial, bytes).unwrap();
    spool.commit(&partial, &ready).unwrap();
    ready
}

#[test]
fn a_committed_capture_is_pending() {
    let dir = TempDir::new("commit");
    let spool = Spool::open(&dir.0).unwrap();

    capture(&spool, b"jpeg");

    let pending = spool.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 0);
    assert_eq!(fs::read(&pending[0].path).unwrap(), b"jpeg");
}

#[test]
fn a_half_written_capture_is_not_uploaded() {
    // The power-cut case. A `.partial` file is a photograph that may be
    // truncated, and a truncated JPEG looks exactly like a whole one to
    // anything that only checks the extension.
    let dir = TempDir::new("partial");
    let spool = Spool::open(&dir.0).unwrap();

    let (partial, _ready) = spool.reserve();
    fs::write(&partial, b"half a jp").unwrap();

    assert!(spool.pending().unwrap().is_empty());
    assert!(partial.exists(), "and it is still there to be looked at");
}

#[test]
fn captures_drain_oldest_first() {
    // Post is read in the order it arrived. A spool that drained newest-first
    // would also leave the oldest item retried forever behind a growing queue.
    let dir = TempDir::new("order");
    let spool = Spool::open(&dir.0).unwrap();

    for (stamp, body) in [(100u64, "first"), (300, "third"), (200, "second")] {
        let path = dir.0.join(format!("{stamp}-1.jpg"));
        fs::write(&path, body).unwrap();
    }

    let bodies: Vec<String> = spool
        .pending()
        .unwrap()
        .iter()
        .map(|item| fs::read_to_string(&item.path).unwrap())
        .collect();
    assert_eq!(bodies, vec!["first", "second", "third"]);
}

#[test]
fn a_capture_survives_a_restart() {
    // The whole point. A daemon that came back up and could not find what it
    // had already photographed would lose post silently.
    let dir = TempDir::new("restart");
    {
        let spool = Spool::open(&dir.0).unwrap();
        capture(&spool, b"jpeg");
    }

    let reopened = Spool::open(&dir.0).unwrap();
    assert_eq!(reopened.pending().unwrap().len(), 1);
}

#[test]
fn a_failed_upload_keeps_the_capture_and_counts_the_attempt() {
    let dir = TempDir::new("failed");
    let spool = Spool::open(&dir.0).unwrap();
    capture(&spool, b"jpeg");

    let item = spool.pending().unwrap().remove(0);
    assert_eq!(spool.failed(&item).unwrap(), 1);

    let again = spool.pending().unwrap();
    assert_eq!(again.len(), 1, "the capture must not have gone anywhere");
    assert_eq!(again[0].attempts, 1);

    assert_eq!(spool.failed(&again[0]).unwrap(), 2);
    assert_eq!(spool.pending().unwrap()[0].attempts, 2);
}

#[test]
fn attempts_survive_a_restart_too() {
    // Otherwise a daemon that crashes on every upload retries instantly
    // forever, and the backoff is decoration.
    let dir = TempDir::new("attempts-restart");
    {
        let spool = Spool::open(&dir.0).unwrap();
        capture(&spool, b"jpeg");
        let item = spool.pending().unwrap().remove(0);
        spool.failed(&item).unwrap();
        spool.failed(&item).unwrap();
    }

    let reopened = Spool::open(&dir.0).unwrap();
    assert_eq!(reopened.pending().unwrap()[0].attempts, 2);
}

#[test]
fn a_confirmed_capture_is_cleared_along_with_its_bookkeeping() {
    let dir = TempDir::new("done");
    let spool = Spool::open(&dir.0).unwrap();
    capture(&spool, b"jpeg");

    let item = spool.pending().unwrap().remove(0);
    spool.failed(&item).unwrap();
    spool.done(&item).unwrap();

    assert!(spool.pending().unwrap().is_empty());
    // The sidecar goes too; a directory of orphaned counters is a directory
    // nobody can read.
    let left: Vec<_> = fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(left.is_empty(), "spool should be empty, found {left:?}");
}

#[test]
fn the_backoff_rises_and_then_stops_rising() {
    // A Pi that has been offline overnight should try again within the minute
    // once the network returns, not in four hours because it gave up politely.
    assert_eq!(retry_delay(0).as_secs(), 0);
    assert!(retry_delay(1) < retry_delay(2));
    assert!(retry_delay(2) < retry_delay(3));
    assert_eq!(retry_delay(9), retry_delay(400));
    assert!(retry_delay(400).as_secs() <= 300);
}

#[test]
fn a_capture_time_comes_from_the_name_not_the_filesystem() {
    // Copying a spool off a dying SD card must not change when the post
    // arrived.
    let dir = TempDir::new("stamp");
    let spool = Spool::open(&dir.0).unwrap();
    fs::write(dir.0.join("1757600000-7.jpg"), b"jpeg").unwrap();

    let pending: Vec<Pending> = spool.pending().unwrap();
    assert_eq!(pending[0].captured_at, 1_757_600_000);
}

#[test]
fn the_backoff_is_counted_from_the_last_attempt_not_from_the_capture() {
    // Counted from the capture, anything photographed more than five minutes
    // ago was always due, and a spool that had been failing for an hour was
    // retried on every pass.
    let dir = TempDir::new("backoff-from-attempt");
    let spool = Spool::open(&dir.0).unwrap();
    fs::write(dir.0.join("1000-1.jpg"), b"jpeg").unwrap();
    for _ in 0..3 {
        let item = spool.pending().unwrap().remove(0);
        spool.failed(&item).unwrap();
    }

    let item = spool.pending().unwrap().remove(0);
    assert_eq!(item.attempts, 3);
    let tried = item.last_attempt.expect("a failed attempt is timed");
    assert!(!item.due(tried + 10), "a minute's backoff, ten seconds in");
    assert!(item.due(tried + 60));
}

#[test]
fn a_counter_written_before_attempts_were_timed_still_reads() {
    let dir = TempDir::new("old-sidecar");
    let spool = Spool::open(&dir.0).unwrap();
    fs::write(dir.0.join("1000-1.jpg"), b"jpeg").unwrap();
    fs::write(dir.0.join("1000-1.attempts"), "2").unwrap();

    let item = spool.pending().unwrap().remove(0);
    assert_eq!(item.attempts, 2);
    assert_eq!(item.last_attempt, None);
    // Falls back to the capture time: fifteen seconds for a second retry.
    assert!(!item.due(1014));
    assert!(item.due(1015));
}

#[test]
fn a_clock_that_went_backwards_does_not_hold_a_retry_back() {
    // A Pi has no real-time clock. Booted before NTP, it can believe it is
    // earlier than the last attempt it recorded.
    let dir = TempDir::new("clock-back");
    let spool = Spool::open(&dir.0).unwrap();
    fs::write(dir.0.join("5000-1.jpg"), b"jpeg").unwrap();
    fs::write(dir.0.join("5000-1.attempts"), "4 5000").unwrap();

    assert!(spool.pending().unwrap()[0].due(100));
}
