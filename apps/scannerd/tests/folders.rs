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
    folders_among, guess, parse_env, parse_env_people, words_from_match, Filings, Folder, Outcome,
    Refiling, CHECK_EVERY_SECS, GIVE_UP_SECS,
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
        person: false,
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
                // The shelf as Paperless holds it: `home` matches nothing —
                // a label put on by hand — and the other two are folders.
                "GET /api/tags/?page_size=500" => r#"{"count":3,"results":[
                     {"id":1,"name":"home","matching_algorithm":0,"match":""},
                     {"id":3,"name":"Car","matching_algorithm":1,"match":"Kfz \"Grüne Karte\""},
                     {"id":8,"name":"Werbung","matching_algorithm":6,"match":""},
                     {"id":9,"name":"Person: Erika Mustermann","matching_algorithm":1,
                      "match":"\"Erika Mustermann\""}]}"#
                    .into(),
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
                r if r.starts_with("PATCH /api/documents/") => "{}".into(),
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

#[test]
fn a_tag_match_reads_back_as_words_with_phrases_kept_whole() {
    // The other direction of `paperless_match`, for folders that came from
    // Paperless rather than from the setup page.
    assert_eq!(
        words_from_match("Hyundai Kfz \"Allianz Versicherung\""),
        "Hyundai, Kfz, Allianz Versicherung"
    );
    assert_eq!(words_from_match(""), "");
}

#[test]
fn the_preview_guesses_a_folder_from_the_words_on_the_page() {
    let shelf = [
        folder("Car", "Kfz, Kennzeichen, Grüne Karte", false),
        folder("Taxes", "Finanzamt, Steuernummer", false),
        // No words of its own: Paperless learns this one, and nothing here
        // can copy what it learnt.
        Folder::named("Sonstiges"),
        Folder::person("Erika Mustermann"),
    ];
    let letter = "Finanzamt Hamburg-Mitte\nFrau Erika Mustermann\nIhre Steuernummer: 12/345";

    let guessed: Vec<&str> = guess(letter, &shelf)
        .into_iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(guessed, vec!["Taxes", "Erika Mustermann"]);

    // Whatever the case, and phrases as one word.
    assert_eq!(
        guess("die grüne karte liegt bei", &shelf)
            .into_iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Car"]
    );
    // A word inside a longer one is not that word: "Kfz" is not in
    // "Kfzversicherung".
    assert!(guess("Kfzversicherungsvertrag", &shelf).is_empty());
    assert!(
        guess("Kfz-Versicherung", &shelf).len() == 1,
        "a hyphen ends a word"
    );
    assert!(guess("", &shelf).is_empty());
}

#[test]
fn a_person_is_a_tag_of_their_own_and_is_looked_for_by_name() {
    let who = Folder::person("Erika Mustermann");
    // Marked in Paperless, so a folder and a person of the same name are two
    // tags and not one.
    assert_eq!(who.tag_name(), "Person: Erika Mustermann");
    assert_eq!(Folder::named("Taxes").tag_name(), "Taxes");
    // With no words of their own, their name is what is looked for: it is
    // what stands on the letter.
    assert_eq!(who.paperless_match(), "\"Erika Mustermann\"");
    // Somebody written to in more than one way can say so.
    let also = Folder {
        words: "Erika Mustermann, E. Mustermann".into(),
        ..Folder::person("Erika Mustermann")
    };
    assert_eq!(
        also.paperless_match(),
        "\"Erika Mustermann\" \"E. Mustermann\""
    );
    assert_eq!(
        parse_env_people(" Erika Mustermann , Max Mustermann ,"),
        vec![
            Folder::person("Erika Mustermann"),
            Folder::person("Max Mustermann"),
        ]
    );
}

#[test]
fn a_person_and_a_folder_may_share_a_name_but_nobody_is_the_bin() {
    assert!(valid_folders(&[Folder::named("Taxes"), Folder::person("Taxes")]).is_ok());
    assert!(valid_folders(&[Folder::person("Erika"), Folder::person("erika")]).is_err());
    assert!(valid_folders(&[Folder {
        discard: true,
        ..Folder::person("Erika")
    }])
    .is_err());
}

#[tokio::test]
async fn the_shelf_can_be_taken_from_paperless_when_nothing_here_names_one() {
    let (base, _log) = paperless(0);

    let folders = uploader(&base).folders_in_paperless().await.unwrap();

    // `home` is left out: a tag that matches nothing is never given to a
    // letter, so it is no folder to sort into.
    assert_eq!(
        folders,
        vec![
            folder("Car", "Kfz, Grüne Karte", false),
            folder("Werbung", "", false),
            // Marked as a person in Paperless, so it comes back as one — with
            // the mark taken off the name again.
            Folder {
                words: "Erika Mustermann".into(),
                ..Folder::person("Erika Mustermann")
            },
        ]
    );
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
        // A real JPEG with lines of "text" on it: the pages of a letter go
        // into a PDF, which needs a JPEG that can be read.
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
async fn a_letter_can_be_filed_by_hand_in_no_folder_or_in_the_bin() {
    // Paperless cannot tell that a letter is worth keeping but belongs on no
    // shelf, or that the paper can go. The buttons on the display say so, and
    // the letter stays in Paperless either way.
    let dir = TempDir(std::env::temp_dir().join(format!("scannerd-refile-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let spool = Spool::open(&dir.0).unwrap();
    let (base, log) = paperless(0);
    let uploader = uploader(&base);
    let mut frames = vec![desk(), page(), page(), page(), page(), page()];
    frames.extend((0..40).map(|_| desk()));
    let camera = Script(Mutex::new(frames));
    // Pages collect into letters, as they do on the rig: a letter has to be
    // finished before there is one to file by hand.
    let (_press, button) = scannerd::button::Button::channel();
    let mut scanner = Scanner::new(3, Duration::from_secs(30)).collecting(scannerd::run::Letters {
        button,
        idle: Duration::from_secs(3_600),
        when_clear: Some(Duration::from_secs(3)),
    });
    scanner.file_into(vec![
        Folder::named("Car"),
        folder("Throw away", "", true),
        Folder::person("Erika Mustermann"),
    ]);
    let start = now();

    // Second by second, as the loop runs: the page goes down, the table is
    // clear again, the letter closes itself and Paperless reads it.
    for second in 0..40 {
        scanner
            .turn(&camera, &spool, &uploader, start + second)
            .await;
    }
    let now = start + 40;
    let events: Vec<String> = scanner
        .take_events()
        .iter()
        .map(|event| event.text.clone())
        .collect();
    assert!(
        scanner.can_refile(now),
        "a letter was finished and sent: {events:?}"
    );

    scanner
        .refile_letter(&uploader, Refiling::Nowhere, now)
        .await;
    assert_eq!(
        scanner.filing().map(|f| &f.outcome),
        Some(&Outcome::Nowhere)
    );

    // The folders' tags come off; what is not a folder — the address, who the
    // post is for — stays.
    let patch = requests(&log)
        .into_iter()
        .filter(|(request, _)| request.starts_with("PATCH /api/documents/"))
        .next_back()
        .expect("the document was changed");
    let sent: serde_json::Value = serde_json::from_str(&patch.1).unwrap();
    assert_eq!(sent["tags"], serde_json::json!([9]), "9 is not a folder");

    // And into the bin: the bin's tag goes on instead.
    scanner
        .refile_letter(&uploader, Refiling::Bin, now + 1)
        .await;
    let patch = requests(&log)
        .into_iter()
        .filter(|(request, _)| request.starts_with("PATCH /api/documents/"))
        .next_back()
        .unwrap();
    let sent: serde_json::Value = serde_json::from_str(&patch.1).unwrap();
    assert_eq!(
        sent["tags"],
        serde_json::json!([9, 11]),
        "11 is the bin's tag"
    );
    assert_eq!(
        scanner.filing().map(|f| &f.outcome),
        Some(&Outcome::Folders(vec![folder("Throw away", "", true)]))
    );
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
