//! The Paperless client against a Paperless that is not there.
//!
//! A canned instance on a loopback socket, because the real one is a Redis and
//! a Django and a first-run index build, and none of that makes these
//! assertions any truer. What it cannot check is whether Paperless really
//! answers in these shapes — that is what `kuverta paper check` against the
//! dev stack is for.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use core_paper::{Paperless, Selector, ShelfFolder};

/// A loopback Paperless.
///
/// It paginates, filters by tag and answers the lookup endpoints, because the
/// things worth testing here are exactly the things a server that ignored
/// `page` and `page_size` could not show — an offset that does not land on a
/// page boundary silently returning the wrong window, for one.
///
/// Returns the base URL and a receiver of every request line, so a test can
/// assert on what was asked for and not only on what came back. Most of the
/// interesting bugs here live in the query string.
fn serve() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let mut request = String::new();
            reader.read_line(&mut request).ok();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut sent = vec![0u8; length];
            if length > 0 {
                std::io::Read::read_exact(&mut reader, &mut sent).ok();
            }
            // The request line and, for a write, what was written: a PATCH
            // that sends the wrong fields is otherwise indistinguishable from
            // one that sends the right ones.
            let request = format!("{} {}", request.trim(), String::from_utf8_lossy(&sent))
                .trim()
                .to_string();
            let _ = tx.send(request.clone());

            let body = answer(&request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });

    (base, rx)
}

fn answer(request: &str) -> String {
    // A write comes back as the row Paperless made; nothing here reads it.
    if request.starts_with("POST") || request.starts_with("PATCH") {
        return "{}".to_string();
    }
    if request.contains("/api/correspondents/") {
        return CORRESPONDENTS.to_string();
    }
    if request.contains("/api/tags/") {
        return TAGS.to_string();
    }

    // Only documents tagged `home` exist, so a selector naming anything else
    // matches nothing — which is the case the preflight is for.
    let matching: Vec<&str> = if request.contains("tags__name__iexact=")
        && !request.contains("tags__name__iexact=home")
    {
        vec![]
    } else {
        vec![DOC_41, DOC_40]
    };

    let page: usize = param(request, "page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let size: usize = param(request, "page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);

    let window: Vec<&str> = matching
        .iter()
        .skip((page - 1) * size)
        .take(size)
        .copied()
        .collect();

    format!(
        "{{\"count\":{},\"results\":[{}]}}",
        matching.len(),
        window.join(",")
    )
}

fn param(request: &str, key: &str) -> Option<String> {
    request
        .split(['?', '&'])
        .skip(1)
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .map(|value| value.split(' ').next().unwrap_or("").to_string())
}

const CORRESPONDENTS: &str =
    r#"{"count":2,"results":[{"id":7,"name":"Stadtwerke München"},{"id":9,"name":"Finanzamt"}]}"#;

/// `home` is a plain label and no folder: it matches nothing by itself, which
/// is how a selector tag is set up. The other two are folders on a shelf, one
/// with words and one Paperless learns.
const TAGS: &str = r#"{"count":4,"results":[
   {"id":1,"name":"home","matching_algorithm":0,"match":""},
   {"id":2,"name":"rechnung","matching_algorithm":1,"match":"Rechnung Mahnung \"Offener Betrag\""},
   {"id":3,"name":"Steuern","matching_algorithm":6,"match":""},
   {"id":4,"name":"Person: Erika Mustermann","matching_algorithm":1,"match":"\"Erika Mustermann\""}]}"#;

const DOC_41: &str = r#"{"id":41,"title":"Abschlagszahlung 2026","correspondent":7,
   "created":"2026-03-04T00:00:00+01:00","added":"2026-03-06T09:12:44.120000Z","tags":[1,2],
   "content":"Ihr monatlicher Abschlag beträgt ab April 84,00 EUR.","page_count":2}"#;

const DOC_40: &str = r#"{"id":40,"title":"","correspondent":null,"created":"2026-02-11",
   "added":"2026-02-12T08:00:00Z","tags":[],"content":"   ","page_count":null}"#;

fn client(base: &str) -> Paperless {
    Paperless::new(base, "secret-token").unwrap()
}

#[tokio::test]
async fn a_document_reads_like_a_message() {
    let (base, _requests) = serve();

    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    assert_eq!(page.total, 2);
    let first = &page.documents[0];

    // Sender, subject, date, body — the four things a list row needs, from a
    // correspondent, a title, a created date and some OCR text.
    assert_eq!(first.correspondent.as_deref(), Some("Stadtwerke München"));
    assert_eq!(first.title, "Abschlagszahlung 2026");
    assert!(first.content.as_deref().unwrap().contains("Abschlag"));
    assert_eq!(first.tags, vec!["home", "rechnung"]);
    assert_eq!(first.page_count, Some(2));
    assert!(first.download_path.ends_with("/api/documents/41/download/"));
}

#[tokio::test]
async fn an_untitled_document_is_not_a_document_that_failed_to_load() {
    // The paper equivalent of a message with no subject. Legal, and it happens
    // whenever Paperless cannot get a title out of the scan.
    let (base, _requests) = serve();

    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    let second = &page.documents[1];
    assert_eq!(second.title, "(no title)");
    assert_eq!(second.correspondent, None);
    // Whitespace-only OCR is no body, not an empty one.
    assert_eq!(second.content, None);
    assert!(second.tags.is_empty());
}

#[tokio::test]
async fn dates_come_back_as_timestamps_in_both_shapes() {
    let (base, _requests) = serve();

    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    // A full timestamp and a bare date. Both have to parse, or the list sorts
    // half the post to 1970.
    //
    // And the offset counts: Paperless answers in the time zone it is set to,
    // so `2026-03-04T00:00:00+01:00` is 23:00 the day before in UTC. Read as
    // if it were UTC, every scanned letter showed in the inbox an hour — in
    // summer two — after it was scanned.
    assert_eq!(page.documents[0].created_utc, Some(1_772_578_800));
    assert_eq!(page.documents[0].added_utc, Some(1_772_788_364), "Z is UTC");
    assert_eq!(
        page.documents[1].created_utc,
        Some(1_770_768_000),
        "a bare date"
    );
    assert_eq!(page.documents[1].added_utc, Some(1_770_883_200));
    assert!(page.documents[0].added_utc.unwrap() > page.documents[0].created_utc.unwrap());
}

#[tokio::test]
async fn the_selector_becomes_a_filter_and_the_token_is_sent() {
    let (base, requests) = serve();

    client(&base)
        .documents(&Selector::Tag("home".into()), 0, 20, None)
        .await
        .unwrap();

    let first = requests.recv().unwrap();
    assert!(first.contains("tags__name__iexact=home"), "got {first}");
    // Newest first by the date on the letter, not the date it was scanned.
    assert!(first.contains("ordering=-created"), "got {first}");
}

#[tokio::test]
async fn a_name_with_a_space_or_an_ampersand_survives_the_query_string() {
    // An under-encoded ampersand does not fail: it splits one filter into two
    // and quietly returns the wrong documents.
    let (base, requests) = serve();

    client(&base)
        .documents(&Selector::Correspondent("Meier & Sohn".into()), 0, 20, None)
        .await
        .ok();

    let first = requests.recv().unwrap();
    assert!(first.contains("Meier%20%26%20Sohn"), "got {first}");
    assert!(!first.contains("Meier & Sohn"));
}

#[tokio::test]
async fn a_search_asks_paperless_rather_than_filtering_here() {
    // The OCR index already exists upstream; rebuilding it locally would be
    // both slower and worse.
    let (base, requests) = serve();

    client(&base)
        .documents(&Selector::Everything, 0, 20, Some("rechnung"))
        .await
        .ok();

    let first = requests.recv().unwrap();
    assert!(first.contains("query=rechnung"), "got {first}");
}

#[tokio::test]
async fn an_offset_that_is_not_a_page_boundary_still_lands_on_the_right_document() {
    let (base, _requests) = serve();

    // Offset 1 of a 1-per-page window: page 2 in Paperless's counting, with the
    // first result skipped.
    let page = client(&base)
        .documents(&Selector::Everything, 1, 1, None)
        .await
        .unwrap();

    assert_eq!(page.offset, 1);
    assert_eq!(page.documents.len(), 1);
    assert_eq!(page.documents[0].id, 40);
}

#[tokio::test]
async fn a_selector_that_matches_nothing_says_so_instead_of_looking_empty() {
    // The §3.6 lesson applied to paper. A mailbox showing nothing because of a
    // mistyped tag looks exactly like an address that has had no post.
    let (base, _requests) = serve();

    let report = client(&base)
        .check(&Selector::Tag("hausverwaltung".into()))
        .await
        .unwrap();

    assert_eq!(report.documents_total, 2);
    assert_eq!(report.documents_matching, 0);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("hausverwaltung")),
        "should name the tag that matched nothing: {:?}",
        report.notes
    );
    assert!(
        report.notes.iter().any(|note| note.contains("home")),
        "and should list the tags that do exist: {:?}",
        report.notes
    );
}

#[tokio::test]
async fn a_tag_that_exists_but_matches_nothing_is_told_apart_from_a_typo() {
    let (base, _requests) = serve();

    // The canned server matches tag names exactly, so "HOME" finds nothing
    // although a tag "home" exists — which Paperless itself would not do, but
    // it is the case to word: a known tag with no documents yet.
    let report = client(&base)
        .check(&Selector::Tag("HOME".into()))
        .await
        .unwrap();

    assert_eq!(report.documents_matching, 0);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note == "no document carries the tag HOME yet"),
        "a tag that exists is not called a typo: {:?}",
        report.notes
    );
}

#[tokio::test]
async fn a_url_without_a_scheme_is_refused_before_any_request() {
    assert!(Paperless::new("localhost:8000", "t").is_err());
    assert!(Paperless::new("http://localhost:8000/", "t").is_ok());
}

// -- post, read by the mail classifier --------------------------------------

#[tokio::test]
async fn post_is_filed_by_the_same_rules_as_mail() {
    // The point of the framing. No paper-specific classifier and no second
    // taxonomy: a letter has a sender, a subject and some text, which is
    // everything the rules read.
    let (base, _requests) = serve();

    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    let verdict = core_rules::Classifier::without_history().classify(&page.documents[0].facts());

    // "Abschlagszahlung" is a transactional keyword, and the PDF counts as the
    // attachment it is.
    assert_eq!(verdict.category, core_rules::Category::Transactional);
    assert!(
        verdict
            .reasons
            .iter()
            .any(|r| r.rule == "attachment.with_transactional_subject"),
        "the document itself is the attachment: {:?}",
        verdict.reasons
    );
}

#[tokio::test]
async fn a_correction_on_post_can_be_remembered_against_the_correspondent() {
    // Paper has no address to key on, so the correspondent carries it —
    // "everything from the Stadtwerke is transactional" being exactly the rule
    // someone wants to teach.
    let (base, _requests) = serve();
    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    let learned = core_rules::Learned::new()
        .with_sender("Stadtwerke München", core_rules::Category::Notification);
    let verdict = core_rules::Classifier::new(learned).classify(&page.documents[0].facts());

    assert_eq!(verdict.category, core_rules::Category::Notification);
    assert_eq!(verdict.reasons[0].rule, "learned.sender");
}

#[tokio::test]
async fn a_document_with_no_correspondent_still_classifies() {
    // The paper equivalent of mail with no From. It must not panic and must
    // not be filed on a name that is not there.
    let (base, _requests) = serve();
    let page = client(&base)
        .documents(&Selector::Everything, 0, 20, None)
        .await
        .unwrap();

    let verdict = core_rules::Classifier::without_history().classify(&page.documents[1].facts());
    assert_eq!(verdict.category, core_rules::Category::Unknown);
}

#[tokio::test]
async fn the_folders_on_the_shelf_are_the_tags_that_match_themselves() {
    let (base, _requests) = serve();

    let folders = client(&base).folders().await.unwrap();

    // `home` is left out: a tag that matches nothing is a label someone puts
    // on by hand, not a folder the post falls into.
    assert_eq!(folders.len(), 3, "{folders:?}");
    assert_eq!(folders[0].name, "rechnung");
    // Back in the shape the window edits: commas between the words, and the
    // quotes Paperless keeps around a phrase taken off again.
    assert_eq!(folders[0].words, "Rechnung, Mahnung, Offener Betrag");
    assert_eq!(folders[1].name, "Steuern");
    assert_eq!(
        folders[1].words, "",
        "learnt, so it has no words of its own"
    );
    // Somebody in the household, with the mark taken off their name again.
    assert_eq!(folders[2].name, "Erika Mustermann");
    assert!(folders[2].person);
    assert!(!folders[0].person);
}

#[tokio::test]
async fn setting_the_folders_up_changes_what_is_there_and_makes_what_is_not() {
    let (base, requests) = serve();

    client(&base)
        .set_folders(&[
            ShelfFolder {
                name: "Rechnung".into(),
                words: "Rechnung, Mahnung, Offener Betrag".into(),
                person: false,
            },
            ShelfFolder {
                name: "Auto".into(),
                words: String::new(),
                person: false,
            },
            ShelfFolder {
                name: "Erika Mustermann".into(),
                words: String::new(),
                person: true,
            },
            ShelfFolder {
                name: "   ".into(),
                words: "nothing".into(),
                person: false,
            },
        ])
        .await
        .unwrap();

    let asked: Vec<String> = requests.try_iter().collect();
    assert_eq!(
        asked.len(),
        4,
        "the tags read once, then three writes: {asked:?}"
    );
    assert!(asked[0].starts_with("GET /api/tags/"));

    // `Rechnung` is the `rechnung` that is already there — a folder is not
    // made twice because someone typed it with a capital.
    let changed = &asked[1];
    assert!(changed.starts_with("PATCH /api/tags/2/"), "{changed}");
    assert!(
        !changed.contains("\"name\""),
        "an existing tag keeps its name"
    );
    assert!(changed.contains("\"matching_algorithm\":1"), "{changed}");
    assert!(
        changed.contains("Rechnung Mahnung \\\"Offener Betrag\\\""),
        "a phrase is quoted for Paperless: {changed}"
    );

    // `Auto` is new, and has no words: Paperless learns it from what is filed
    // in it by hand.
    let made = &asked[2];
    assert!(made.starts_with("POST /api/tags/"), "{made}");
    assert!(made.contains("\"name\":\"Auto\""), "{made}");
    assert!(made.contains("\"matching_algorithm\":6"), "{made}");

    // Somebody in the household is a tag of their own, marked as a person so
    // that a folder and a person of the same name stay two things, and looked
    // for by their name when they have no other spelling.
    // She is already there under her marked name, so she is brought up to
    // date rather than made a second time.
    let who = &asked[3];
    assert!(who.starts_with("PATCH /api/tags/4/"), "{who}");
    assert!(
        who.contains("\\\"Erika Mustermann\\\""),
        "looked for by name: {who}"
    );
    assert!(who.contains("\"matching_algorithm\":1"), "{who}");

    // The blank row the window leaves behind is not a tag called nothing.
    assert!(!asked.iter().any(|line| line.contains("\"nothing\"")));
}
