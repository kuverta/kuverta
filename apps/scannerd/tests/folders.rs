//! Which folder a letter goes in: the folders, the Paperless calls that set
//! their tags up and read what a letter got, and the loop following a letter
//! from upload to folder — against a Paperless on loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use scannerd::camera::Camera;
use scannerd::folders::{
    folders_among, parse_env, Filings, Folder, Outcome, CHECK_EVERY_SECS, GIVE_UP_SECS,
};
use scannerd::run::Scanner;
use scannerd::settings::{valid_folders, Settings};
use scannerd::spool::Spool;
use scannerd::upload::{Task, Uploader};

fn folder(name: &str, words: &str, discard: bool) -> Folder {
    Folder {
        name: name.into(),
        words: words.into(),
        discard,
    }
}

// -- the folders -----------------------------------------------------------------

#[test]
fn the_env_file_names_folders_and_marks_the_bin_with_a_dash() {
    assert_eq!(
        parse_env(" Car, House ,,-Throw away"),
        vec![
            Folder::named("Car"),
            Folder::named("House"),
            folder("Throw away", "", true)
        ]
    );
    assert!(parse_env("").is_empty());
}

#[test]
fn words_become_paperless_any_word_matching_with_phrases_quoted() {
    assert_eq!(
        folder("Car", "Hyundai, Kfz ,, Allianz Versicherung", false).paperless_match(),
        "Hyundai Kfz \"Allianz Versicherung\""
    );
    assert_eq!(folder("Taxes", "", false).paperless_match(), "");
}

#[test]
fn folders_must_be_told_apart_and_have_one_bin_at_most() {
    assert!(valid_folders(&[Folder::named("Car"), Folder::named("House")]).is_ok());
    assert!(valid_folders(&[Folder::named("Car"), Folder::named("car")]).is_err());
    assert!(valid_folders(&[folder("A", "", true), folder("B", "", true)]).is_err());
}

#[test]
fn the_page_replaces_the_folders_whole_and_drops_nameless_rows() {
    let mut settings = Settings::default();
    assert_eq!(
        settings.effective_folders(&[Folder::named("Env")]),
        vec![Folder::named("Env")]
    );
    settings.merge(Settings {
        folders: Some(vec![folder(" Car ", " Kfz ", false), Folder::named("  ")]),
        ..Settings::default()
    });
    assert_eq!(
        settings.effective_folders(&[Folder::named("Env")]),
        vec![folder("Car", "Kfz", false)]
    );
    // An empty list is no folders, not "use the env file's".
    settings.merge(Settings {
        folders: Some(vec![]),
        ..Settings::default()
    });
    assert!(settings
        .effective_folders(&[Folder::named("Env")])
        .is_empty());
}

#[test]
fn a_document_goes_in_the_folders_whose_tags_it_has_in_their_order() {
    let tags = [(3, Folder::named("Car")), (5, Folder::named("House"))];
    assert_eq!(
        folders_among(&[9, 5, 3], &tags),
        vec![Folder::named("Car"), Folder::named("House")]
    );
    assert!(folders_among(&[9], &tags).is_empty());
}

#[test]
fn a_letter_is_asked_about_every_few_seconds_and_given_up_on_eventually() {
    let mut filings = Filings::default();
    filings.sent("letter.pdf", "task-1", 1_000);
    assert_eq!(filings.next_to_check(1_000).unwrap().task, "task-1");
    assert!(filings.next_to_check(1_001).is_none(), "asked a moment ago");
    assert!(filings.next_to_check(1_000 + CHECK_EVERY_SECS).is_some());

    assert!(filings.next_to_check(1_000 + GIVE_UP_SECS).is_none());
    assert_eq!(filings.latest().unwrap().outcome, Outcome::TimedOut);
}

#[test]
fn a_decided_letter_is_not_asked_about_again() {
    let mut filings = Filings::default();
    filings.sent("a.pdf", "a", 1_000);
    filings.sent("b.pdf", "b", 1_001);
    filings.decide("a", Outcome::Duplicate, 1_002);
    assert_eq!(filings.next_to_check(1_010).unwrap().task, "b");
    assert_eq!(filings.latest().unwrap().name, "b.pdf");
}

// -- a Paperless on loopback -------------------------------------------------------

/// Every request as `METHOD path` and its body, in order.
type Log = Arc<Mutex<Vec<(String, String)>>>;

/// A Paperless with a tag "Car" (3) and "House" (5), that makes new tags
/// as 11, takes uploads as task-1, and has finished reading task-1 as document
/// 42, tagged Car, once it has been asked about it `reading` times.
fn paperless(reading: usize) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let record = log.clone();
    let asked = AtomicUsize::new(0);

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
            let request: String = line.split(' ').take(2).collect::<Vec<_>>().join(" ");
            record
                .lock()
                .unwrap()
                .push((request.clone(), String::from_utf8_lossy(&body).into_owned()));

            let reply = match request.as_str() {
                "GET /api/tags/?name__iexact=Car" => r#"{"results":[{"id":3,"name":"Car"}]}"#.into(),
                "GET /api/tags/?name__iexact=House" => r#"{"results":[{"id":5,"name":"House"}]}"#.into(),
                r if r.starts_with("GET /api/tags/") => r#"{"results":[]}"#.into(),
                "POST /api/tags/" => r#"{"id":11}"#.into(),
                r if r.starts_with("PATCH /api/tags/") => "{}".into(),
                "POST /api/documents/post_document/" => r#""task-1""#.into(),
                "GET /api/tasks/?task_id=task-1" => {
                    if asked.fetch_add(1, SeqCst) < reading {
                        "[]".into()
                    } else {
                        r#"[{"task_id":"task-1","status":"SUCCESS","result":"Success","related_document":"42"}]"#.into()
                    }
                }
                "GET /api/tasks/?task_id=task-dup" => {
                    r#"[{"task_id":"task-dup","status":"FAILURE","result":"Not consuming letter.pdf: It is a duplicate of Post 1 (#7)."}]"#.into()
                }
                // Paperless 3: lower case, the document in `result_data`.
                "GET /api/tasks/?task_id=task-3" => {
                    r#"{"count":1,"results":[{"task_id":"task-3","status":"success","result_data":{"document_id":15},"related_document_ids":[15]}]}"#.into()
                }
                "GET /api/tasks/?task_id=task-3-dup" => {
                    r#"{"count":1,"results":[{"task_id":"task-3-dup","status":"failure","result_data":{"error":"It is a duplicate of Post 1 (#7)."},"related_document_ids":[]}]}"#.into()
                }
                "GET /api/documents/42/" => r#"{"id":42,"tags":[9,3]}"#.to_string(),
                _ => String::new(),
            };
            let status = if reply.is_empty() {
                "404 Not Found"
            } else {
                "200 OK"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, log)
}

fn uploader(base: &str) -> Uploader {
    Uploader::new(base, "token", vec![], "Post ".into()).unwrap()
}

fn requests(log: &Log) -> Vec<(String, String)> {
    log.lock().unwrap().clone()
}

#[tokio::test]
async fn folders_tags_are_made_or_brought_up_to_date_in_paperless() {
    let (base, log) = paperless(0);
    let tags = uploader(&base)
        .sync_folders(&[
            folder("Car", "Hyundai, Kfz", false),
            folder("Throw away", "", true),
        ])
        .await
        .unwrap();
    assert_eq!(
        tags,
        vec![
            (3, folder("Car", "Hyundai, Kfz", false)),
            (11, folder("Throw away", "", true))
        ]
    );

    let requests = requests(&log);
    let (_, patch) = requests
        .iter()
        .find(|(request, _)| request == "PATCH /api/tags/3/")
        .expect("the existing tag is updated");
    let patch: serde_json::Value = serde_json::from_str(patch).unwrap();
    assert_eq!(patch["matching_algorithm"], 1, "any word");
    assert_eq!(patch["match"], "Hyundai Kfz");

    let (_, created) = requests
        .iter()
        .find(|(request, _)| request == "POST /api/tags/")
        .expect("the missing tag is made");
    let created: serde_json::Value = serde_json::from_str(created).unwrap();
    assert_eq!(created["name"], "Throw away");
    assert_eq!(
        created["matching_algorithm"], 6,
        "no words: Paperless learns"
    );
    assert!(requests
        .iter()
        .any(|(r, _)| r == "GET /api/tags/?name__iexact=Throw%20away"));
}

#[tokio::test]
async fn a_task_is_working_until_paperless_lists_it_then_done_as_its_document() {
    let (base, _) = paperless(1);
    let uploader = uploader(&base);
    assert_eq!(uploader.task("task-1").await.unwrap(), Task::Working);
    assert_eq!(uploader.task("task-1").await.unwrap(), Task::Done(42));
    assert_eq!(uploader.document_tags(42).await.unwrap(), vec![9, 3]);
}

#[tokio::test]
async fn a_duplicate_is_told_from_a_failure() {
    let (base, _) = paperless(0);
    assert_eq!(
        uploader(&base).task("task-dup").await.unwrap(),
        Task::Duplicate
    );
}

// -- the loop, from a page put down to a folder ---------------------------------------

const PIXELS: usize = 320 * 240;

fn desk() -> Vec<u8> {
    (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect()
}

fn page() -> Vec<u8> {
    let mut frame = desk();
    frame[..PIXELS / 2].fill(235);
    frame
}

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
        std::fs::write(path, b"\xff\xd8a photograph of a letter")?;
        Ok(())
    }
}

struct TempDir(PathBuf);

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

#[tokio::test]
async fn a_page_sent_to_paperless_is_followed_until_its_folder_is_known() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-folders-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let (base, log) = paperless(2);
    let uploader = uploader(&base);
    let camera = Script(Mutex::new(vec![desk(), page(), page(), page(), page()]));
    let mut scanner = Scanner::new(3, Duration::from_secs(30));
    scanner.file_into(vec![Folder::named("House"), Folder::named("Car")]);
    let start = now();

    let mut second = 0;
    while second < 60
        && !matches!(
            scanner.filing().map(|f| &f.outcome),
            Some(Outcome::Folders(_))
        )
    {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
        second += 1;
    }

    let filing = scanner.filing().expect("the page was sent and followed");
    assert_eq!(filing.outcome, Outcome::Folders(vec![Folder::named("Car")]));

    let requests: Vec<String> = requests(&log).into_iter().map(|(r, _)| r).collect();
    let upload = requests
        .iter()
        .position(|r| r == "POST /api/documents/post_document/")
        .unwrap();
    // The tags were set up before the letter went, so Paperless could give
    // them to it as it read it.
    assert!(requests[..upload]
        .iter()
        .any(|r| r.starts_with("PATCH /api/tags/3/")));
    // Asked every few seconds, not every turn.
    let asked = requests
        .iter()
        .filter(|r| r.starts_with("GET /api/tasks/"))
        .count();
    assert_eq!(asked, 3, "{requests:?}");
    assert!(scanner
        .take_events()
        .iter()
        .any(|event| event.ok && event.text.ends_with("goes in Car")));
}

#[tokio::test]
async fn a_paperless_that_stays_down_is_said_once_not_every_thirty_seconds() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("scannerd-folders-down-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let (partial, ready) = spool.reserve();
    std::fs::write(&partial, b"\xff\xd8a page").unwrap();
    spool.commit(&partial, &ready).unwrap();
    // Nothing listens here.
    let uploader = Uploader::new("http://127.0.0.1:9", "token", vec![], "Post ".into()).unwrap();
    let mut scanner = Scanner::new(3, Duration::from_secs(30));
    scanner.file_into(vec![Folder::named("Car")]);

    for minute in 0..5 {
        scanner
            .retry_all(&spool, &uploader, 1_000 + minute * 60)
            .await;
    }
    let said = scanner
        .take_events()
        .iter()
        .filter(|event| event.text.starts_with("could not set up the folders"))
        .count();
    assert_eq!(said, 1);
}

#[tokio::test]
async fn paperless_3_tasks_are_read_too() {
    let (base, _) = paperless(0);
    let uploader = uploader(&base);
    assert_eq!(uploader.task("task-3").await.unwrap(), Task::Done(15));
    assert_eq!(uploader.task("task-3-dup").await.unwrap(), Task::Duplicate);
}

#[test]
fn words_paperless_would_refuse_are_refused_here() {
    let many = (0..40)
        .map(|i| format!("Wort{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let err = valid_folders(&[folder("Car", &many, false)]).unwrap_err();
    assert!(err.to_string().contains("Car"), "{err}");
    assert!(valid_folders(&[folder("Car", "Kfz, Fahrzeug, Kennzeichen", false)]).is_ok());
}
