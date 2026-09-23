//! A letter is finished once its last page has been taken away and the table
//! stays empty for a while.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use scannerd::button::Button;
use scannerd::camera::Camera;
use scannerd::run::{Letters, Scanner};
use scannerd::spool::Spool;
use scannerd::upload::Uploader;

const PIXELS: usize = 320 * 240;

fn desk() -> Vec<u8> {
    (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect()
}

fn page() -> Vec<u8> {
    let mut frame = desk();
    frame[..PIXELS / 2].fill(235);
    frame
}

/// Shows the frames it is given, one a turn, then the last one for ever.
struct Script(Mutex<Vec<Vec<u8>>>);

impl Camera for Script {
    fn preview(&self) -> Result<Vec<u8>> {
        let mut frames = self.0.lock().unwrap();
        Ok(if frames.len() > 1 {
            frames.remove(0)
        } else {
            frames[0].clone()
        })
    }

    fn capture(&self, path: &Path) -> Result<()> {
        // A real JPEG with lines of "text" on it: a letter's PDF is made of
        // its pages and leaves out what it cannot read, and a photograph with
        // nothing on it is taken for the table.
        let (w, h) = (160usize, 220usize);
        let page: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let printed =
                    (8..h - 8).contains(&y) && (8..w - 8).contains(&x) && y % 10 < 4 && x % 8 < 5;
                if printed {
                    40
                } else {
                    220
                }
            })
            .collect();
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 85).encode(
            &page,
            w as u16,
            h as u16,
            jpeg_encoder::ColorType::Luma,
        )?;
        std::fs::write(path, jpeg)?;
        Ok(())
    }
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `frames`, one a second, and says how many letters were finished.
async fn letters_finished(name: &str, frames: Vec<Vec<u8>>, seconds: u64) -> u64 {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-clear-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let camera = Script(Mutex::new(frames));
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600)).collecting(Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: Some(Duration::from_secs(8)),
    });
    // The spool stamps pages with the real clock; the loop is given the same.
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for second in 0..seconds {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    scanner.letters_closed()
}

#[tokio::test]
async fn the_last_page_taken_away_and_the_table_left_empty_finishes_the_letter() {
    let mut frames = vec![desk(), page(), page(), page(), page()];
    frames.extend(std::iter::repeat_n(desk(), 3));
    assert_eq!(letters_finished("done", frames, 20).await, 1);
}

#[tokio::test]
async fn the_next_page_laid_down_in_time_keeps_the_letter_open() {
    let mut frames = vec![desk(), page(), page(), page(), page()];
    // Taken away, and the next page down within a few seconds.
    frames.extend(std::iter::repeat_n(desk(), 4));
    frames.push(page());
    assert_eq!(letters_finished("next", frames, 20).await, 0);
}

#[tokio::test]
async fn with_nothing_photographed_an_empty_table_finishes_nothing() {
    assert_eq!(letters_finished("empty", vec![desk()], 20).await, 0);
}

// -- envelopes ----------------------------------------------------------------------------

/// Photographs as the rig takes them: a page filling the corners, or an
/// envelope — smaller paper on the table — whichever is next.
struct Photos {
    frames: Mutex<Vec<Vec<u8>>>,
    photos: Mutex<Vec<bool>>,
    /// Photographs too dark to be paper.
    dark: bool,
    /// Paper with nothing printed on it.
    blank: bool,
}

impl Camera for Photos {
    fn preview(&self) -> Result<Vec<u8>> {
        let mut frames = self.frames.lock().unwrap();
        Ok(if frames.len() > 1 {
            frames.remove(0)
        } else {
            frames[0].clone()
        })
    }

    fn capture(&self, path: &Path) -> Result<()> {
        let envelope = {
            let mut photos = self.photos.lock().unwrap();
            if photos.len() > 1 {
                photos.remove(0)
            } else {
                photos[0]
            }
        };
        let (w, h) = (200usize, 280usize);
        // The table: dark, and in the dark test nothing brighter on it.
        let mut rgb = vec![70u8; w * h * 3];
        let (x0, x1, y0, y1) = if envelope {
            (20, 180, 90, 170)
        } else {
            (2, 198, 2, 278)
        };
        let paper = if self.dark { 110 } else { 235 };
        let ink = if self.blank { paper } else { 40 };
        for y in y0..y1 {
            for x in x0..x1 {
                // Lines of "text" on it, unless it is meant to be blank.
                let printed = (y - y0) % 12 < 4 && (x - x0) % 9 < 6 && x > x0 + 8 && x < x1 - 8;
                let value = if printed { ink } else { paper };
                let i = (y * w + x) * 3;
                rgb[i..i + 3].copy_from_slice(&[value, value, value]);
            }
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 90).encode(
            &rgb,
            w as u16,
            h as u16,
            jpeg_encoder::ColorType::Rgb,
        )?;
        std::fs::write(path, jpeg)?;
        Ok(())
    }
}

/// A letter's page is photographed, then an envelope is put down on the
/// cleared table, then the table is left empty — with `frames` after the
/// first page. Says how many letters were finished and how many pages are
/// open.
async fn page_then_envelope(name: &str) -> (u64, usize) {
    let dir = TempDir(
        std::env::temp_dir().join(format!("scannerd-envelope-{name}-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let mut frames = vec![desk(), page(), page(), page(), page()];
    // Taken away, and the envelope put down within a couple of seconds.
    frames.extend([desk(), desk()]);
    frames.extend(std::iter::repeat_n(page(), 4));
    // The envelope taken away to be opened, and the table left empty.
    frames.push(desk());
    let camera = Photos {
        frames: Mutex::new(frames),
        photos: Mutex::new(vec![false, true]),
        dark: false,
        blank: false,
    };
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600)).collecting(Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: Some(Duration::from_secs(8)),
    });
    scanner.straighten_with(Some(scannerd::straighten::Corners::WHOLE));
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for second in 0..40 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    (scanner.letters_closed(), spool.open_pages().unwrap().len())
}

#[tokio::test]
async fn an_envelope_finishes_the_letter_before_it_and_starts_the_next() {
    let (closed, open) = page_then_envelope("next").await;
    // The page's letter was finished by the envelope, and the envelope —
    // alone on an empty table for half a minute — is still the open letter,
    // waiting for what is inside it.
    assert_eq!(closed, 1);
    assert_eq!(open, 1);
}

// -- taking pages and letters back ------------------------------------------------------

/// A scanner that has photographed `pages` pages of one letter, and its spool.
async fn with_pages(name: &str, pages: usize) -> (TempDir, Spool, Scanner, u64) {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-undo-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let mut frames = vec![desk()];
    for _ in 0..pages {
        frames.extend(std::iter::repeat_n(page(), 4));
        frames.push(desk());
    }
    let camera = Script(Mutex::new(frames));
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600)).collecting(Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: None,
    });
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for second in 0..(pages as u64 * 5 + 1) {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    assert_eq!(spool.open_pages().unwrap().len(), pages);
    (dir, spool, scanner, start + pages as u64 * 5 + 1)
}

fn discarded(dir: &TempDir) -> usize {
    std::fs::read_dir(dir.0.join("discarded"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn the_last_page_can_be_taken_back_and_is_kept_aside() {
    let (dir, spool, mut scanner, now) = with_pages("page", 2).await;
    assert!(scanner.undo_page(&spool, now));
    assert_eq!(spool.open_pages().unwrap().len(), 1);
    assert_eq!(discarded(&dir), 1);
}

#[tokio::test]
async fn a_letter_can_be_cancelled_and_is_kept_aside() {
    let (dir, spool, mut scanner, now) = with_pages("cancel", 3).await;
    assert_eq!(scanner.cancel_letter(&spool, now), 3);
    assert!(spool.open_pages().unwrap().is_empty());
    assert_eq!(discarded(&dir), 3);
}

#[tokio::test]
async fn a_letter_not_yet_sent_is_taken_back_out_of_the_queue() {
    let (dir, spool, mut scanner, now) = with_pages("letter", 1).await;
    // Finished, and Paperless is not there: it waits in the queue.
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    assert!(scanner.request_close(now));
    let camera = Script(Mutex::new(vec![desk()]));
    scanner.turn(&camera, &spool, &uploader, now + 1).await;
    assert_eq!(spool.pending().unwrap().len(), 1);
    assert!(scanner.can_undo_letter(now + 2));

    scanner.undo_letter(&spool, &uploader, now + 2).await;
    assert!(spool.pending().unwrap().is_empty());
    assert_eq!(discarded(&dir), 1);
    assert!(!scanner.can_undo_letter(now + 3), "once is enough");
}

#[tokio::test]
async fn a_dark_table_is_not_an_envelope() {
    // What the rig photographed at dusk: no paper, covering about the share
    // of the corners an envelope would.
    let dir = TempDir(std::env::temp_dir().join(format!("scannerd-dark-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let mut frames = vec![desk(), page(), page(), page(), page()];
    frames.extend(std::iter::repeat_n(desk(), 3));
    let camera = Photos {
        frames: Mutex::new(frames),
        // A sheet the size of an envelope, too dark to be paper.
        photos: Mutex::new(vec![true]),
        dark: true,
        blank: false,
    };
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600)).collecting(Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: None,
    });
    scanner.straighten_with(Some(scannerd::straighten::Corners::WHOLE));
    let start = 1_000;
    for second in 0..8 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    let last = scanner.last_photographed().expect("photographed");
    assert!(!last.envelope, "the dark table is not an envelope");
}

#[tokio::test]
async fn a_photograph_of_the_table_is_not_a_page_of_the_letter() {
    // The page is taken away and the table, which no longer looks like the one
    // that was learnt, is photographed: nothing on it.
    let dir = TempDir(std::env::temp_dir().join(format!("scannerd-table-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let mut frames = vec![desk(), page(), page(), page(), page()];
    frames.extend(std::iter::repeat_n(desk(), 3));
    let camera = Photos {
        frames: Mutex::new(frames),
        // A big sheet of nothing: bright, no text.
        photos: Mutex::new(vec![false]),
        dark: false,
        blank: true,
    };
    let (_press, button) = Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(3_600)).collecting(Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: None,
    });
    scanner.straighten_with(Some(scannerd::straighten::Corners::WHOLE));
    let start = 1_000;
    for second in 0..8 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    assert!(
        spool.open_pages().unwrap().is_empty(),
        "nothing went into the letter"
    );
    assert_eq!(discarded(&dir), 1, "it is kept aside, not deleted");
    let last = scanner.last_photographed().expect("photographed");
    assert!(!last.kept);
}
