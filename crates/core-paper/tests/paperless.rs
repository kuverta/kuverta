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

use core_paper::{Paperless, Selector};

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
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let request = request.trim().to_string();
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

const TAGS: &str = r#"{"count":2,"results":[{"id":1,"name":"home"},{"id":2,"name":"rechnung"}]}"#;

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
    assert_eq!(page.documents[0].created_utc, Some(1_772_582_400));
    assert_eq!(page.documents[1].created_utc, Some(1_770_768_000));
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
