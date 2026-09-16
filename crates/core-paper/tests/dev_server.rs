//! The Paperless client against the real Paperless from `docker/`.
//!
//! `paperless.rs` checks the client against canned answers; this checks the
//! answers are really shaped that way. Written to hold on an empty instance as
//! well as a used one, so it asserts on agreement — between the report, the
//! listing and each document — rather than on what happens to be stored.
//!
//! Start the stack with `make dev-up`. Skips when Paperless is not answering;
//! `KUVERTA_REQUIRE_DEV_SERVER=1` makes that a failure.

use core_paper::{PaperError, Paperless, Selector};

const BASE: &str = "http://localhost:8000";

fn available() -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:8000".parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();
    if !reachable {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev Paperless on {BASE} is required but not reachable");
        }
        eprintln!("skipping: dev Paperless not running (`make dev-up`)");
    }
    reachable
}

/// A token for the admin user baked into docker-compose.yml.
///
/// Asked for rather than configured, so the test needs nothing but the stack.
async fn token() -> String {
    let body = reqwest::Client::new()
        .post(format!("{BASE}/api/token/"))
        .header("Content-Type", "application/json")
        .body(r#"{"username":"admin","password":"admin"}"#)
        .send()
        .await
        .expect("paperless is up but /api/token/ did not answer")
        .text()
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_else(|_| panic!("not a token response (still starting?): {body}"));
    value["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token in {body}"))
        .to_string()
}

#[tokio::test]
async fn the_report_and_the_listing_agree_about_what_is_there() {
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, &token().await).unwrap();

    let report = client.check(&Selector::Everything).await.unwrap();
    assert!(report.reachable && report.authenticated, "{report:?}");
    assert_eq!(report.documents_matching, report.documents_total);

    let page = client
        .documents(&Selector::Everything, 0, 100, None)
        .await
        .unwrap();
    assert_eq!(page.total, report.documents_total);
    assert_eq!(page.documents.len(), page.total.min(100));
}

#[tokio::test]
async fn each_listed_document_reads_back_as_itself() {
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, &token().await).unwrap();
    let page = client
        .documents(&Selector::Everything, 0, 5, None)
        .await
        .unwrap();

    for listed in &page.documents {
        let read = client.document(listed.id).await.unwrap();
        assert_eq!(read.id, listed.id);
        assert_eq!(read.title, listed.title);
        assert_eq!(read.tags, listed.tags);
        assert_eq!(read.correspondent, listed.correspondent);
        // Tag ids must have been resolved to names: a listing that showed
        // "1" where a person wrote "Hauptstraße 12" would select nothing.
        for tag in &read.tags {
            assert!(tag.parse::<i64>().is_err(), "unresolved tag id {tag:?}");
        }
        assert!(
            read.download_path.contains(&read.id.to_string()),
            "{}",
            read.download_path
        );
    }
}

#[tokio::test]
async fn a_window_past_the_first_page_matches_the_same_documents_fetched_whole() {
    // The pagination the fake was written to catch, against the real paging.
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, &token().await).unwrap();
    let whole = client
        .documents(&Selector::Everything, 0, 100, None)
        .await
        .unwrap();
    if whole.total < 2 {
        eprintln!("skipping: needs two documents (`scannerd --drain-only` a couple)");
        return;
    }

    let window = client
        .documents(&Selector::Everything, 1, 1, None)
        .await
        .unwrap();
    assert_eq!(window.documents.len(), 1);
    assert_eq!(window.documents[0].id, whole.documents[1].id);
}

#[tokio::test]
async fn every_tag_that_exists_selects_the_documents_that_carry_it() {
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, &token().await).unwrap();
    let report = client.check(&Selector::Everything).await.unwrap();
    let everything = client
        .documents(&Selector::Everything, 0, 100, None)
        .await
        .unwrap();

    for tag in &report.tags {
        let expected = everything
            .documents
            .iter()
            .filter(|document| document.tags.contains(tag))
            .count();
        // Upper-cased on purpose: a tag typed differently from how Paperless
        // stores it is still the same address. ASCII only — full Unicode
        // upper-casing turns "ß" into "SS", which is another spelling rather
        // than another case, and Paperless (rightly) does not match it.
        let page = client
            .documents(&Selector::Tag(tag.to_ascii_uppercase()), 0, 100, None)
            .await
            .unwrap();
        assert_eq!(page.total, expected, "tag {tag:?}");
    }
}

#[tokio::test]
async fn a_tag_that_does_not_exist_selects_nothing_and_says_why() {
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, &token().await).unwrap();
    let selector = Selector::Tag("keine Adresse & kein Tag".into());

    let report = client.check(&selector).await.unwrap();
    assert_eq!(report.documents_matching, 0);
    assert!(!report.notes.is_empty(), "{report:?}");

    let page = client.documents(&selector, 0, 10, None).await.unwrap();
    assert_eq!(page.total, 0);
}

#[tokio::test]
async fn a_wrong_token_is_an_auth_error_not_an_empty_mailbox() {
    if !available() {
        return;
    }
    let client = Paperless::new(BASE, "not-a-token").unwrap();
    let err = client
        .documents(&Selector::Everything, 0, 10, None)
        .await
        .unwrap_err();
    assert!(matches!(err, PaperError::Auth), "got {err:?}");
}
