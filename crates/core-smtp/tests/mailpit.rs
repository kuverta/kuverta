//! Submission against the Mailpit sink from `docker/`.
//!
//! Start it with `make dev-up`. As with the Dovecot tests, a missing server
//! skips rather than fails, and `KUVERTA_REQUIRE_DEV_SERVER=1` turns that
//! skip back into a failure so CI cannot go green on a broken compose file.
//!
//! Mailpit is the assertion point precisely because it records the SMTP
//! envelope separately from the message headers. That is the only place the
//! Bcc guarantee can be checked end to end: the unit tests prove the header is
//! absent from the bytes we build, and this proves the address still reached
//! the envelope after `mail-send` had its turn.

use core_accounts::EnvPassword;
use core_smtp::{submit, Draft, Mailbox};
use core_store::model::{SmtpConfig, SmtpSecurity};

const HOST: &str = "127.0.0.1";
const SMTP_PORT: u16 = 1025;
const API: &str = "http://127.0.0.1:8025/api/v1";

fn sink_available() -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{SMTP_PORT}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();

    if !reachable {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev SMTP sink on {HOST}:{SMTP_PORT} is required but not reachable");
        }
        eprintln!("skipping: dev SMTP sink not running (`make dev-up`)");
    }
    reachable
}

fn config() -> SmtpConfig {
    SmtpConfig {
        host: HOST.into(),
        port: SMTP_PORT,
        security: SmtpSecurity::Plaintext,
    }
}

fn auth() -> EnvPassword {
    // SAFETY: set before any thread reads it; Mailpit accepts anything, so
    // this is not a credential.
    unsafe { std::env::set_var("KUVERTA_SINK_PASSWORD", "devpass") };
    EnvPassword::new("KUVERTA_SINK_PASSWORD")
}

/// Finds the submitted message by its Message-ID.
///
/// Every test in this file shares one container, and `cargo test` runs them
/// concurrently. Looking a message up by the id we generated for it — rather
/// than emptying the sink and taking whatever is there — is what makes that
/// safe, and it costs nothing: the id is already on `BuiltMessage`.
async fn message_with_id(http: &reqwest::Client, message_id: &str) -> serde_json::Value {
    let listing: serde_json::Value = http
        .get(format!("{API}/messages?limit=200"))
        .send()
        .await
        .expect("list messages")
        .json()
        .await
        .expect("decode listing");

    listing["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|message| message["MessageID"] == message_id)
        .unwrap_or_else(|| panic!("no message with id {message_id} reached the sink"))
        .clone()
}

async fn raw_source(http: &reqwest::Client, id: &str) -> String {
    http.get(format!("{API}/message/{id}/raw"))
        .send()
        .await
        .expect("fetch raw source")
        .text()
        .await
        .expect("decode raw source")
}

#[tokio::test]
async fn a_submitted_message_arrives_with_its_headers_and_body() {
    if !sink_available() {
        return;
    }
    let http = reqwest::Client::new();

    let built = Draft::new(Mailbox::named("Erika", "dev@kuverta.test"))
        .to(Mailbox::named("Jane Doe", "jane@example.com"))
        .subject("Rechnung für September — Übersicht")
        .body("Hallo Jane,\n\nanbei die Übersicht.\n\nGrüße\nErika")
        .build()
        .expect("build");

    submit(&config(), "dev@kuverta.test", &auth(), &built)
        .await
        .expect("submit");

    let message = message_with_id(&http, &built.message_id).await;
    assert_eq!(message["From"]["Address"], "dev@kuverta.test");
    assert_eq!(message["To"][0]["Address"], "jane@example.com");
    // Non-ASCII must survive the encoding round trip rather than arriving as
    // mojibake or as a raw 8-bit header.
    assert_eq!(message["Subject"], "Rechnung für September — Übersicht");

    let raw = raw_source(&http, message["ID"].as_str().unwrap()).await;
    assert!(raw.contains(&format!("Message-ID: <{}>", built.message_id)));
    assert!(raw.contains("Grüße") || raw.contains("Gr=C3=BC=C3=9Fe"));
}

#[tokio::test]
async fn a_blind_recipient_reaches_the_envelope_but_not_the_message() {
    if !sink_available() {
        return;
    }
    let http = reqwest::Client::new();

    let built = Draft::new(Mailbox::new("dev@kuverta.test"))
        .to(Mailbox::new("jane@example.com"))
        .cc(Mailbox::new("bob@example.com"))
        .bcc(Mailbox::new("secret@example.com"))
        .subject("Quarterly")
        .body("See attached.")
        .build()
        .expect("build");

    assert_eq!(built.recipients.len(), 3);

    submit(&config(), "dev@kuverta.test", &auth(), &built)
        .await
        .expect("submit");

    let message = message_with_id(&http, &built.message_id).await;

    // Mailpit reports as Bcc any envelope recipient that appears in no address
    // header — so this asserts the address really was delivered to.
    let bcc: Vec<&str> = message["Bcc"]
        .as_array()
        .expect("bcc array")
        .iter()
        .map(|entry| entry["Address"].as_str().unwrap())
        .collect();
    assert_eq!(bcc, vec!["secret@example.com"]);

    // And that no recipient can see it. This is the assertion the whole
    // hand-built-envelope path in Draft::build exists for: mail-send's own
    // MessageBuilder conversion would have left the header in place here.
    let raw = raw_source(&http, message["ID"].as_str().unwrap()).await;
    let transmitted = as_transmitted(&raw, &built.message_id);
    assert!(
        !transmitted.contains("secret@example.com"),
        "the blind recipient was disclosed in the transmitted message:\n{transmitted}"
    );
    assert!(!transmitted.to_lowercase().contains("bcc:"));
    assert!(transmitted.contains("jane@example.com"));
    assert!(transmitted.contains("bob@example.com"));
}

/// The part of Mailpit's raw view that we actually sent.
///
/// Mailpit prepends its own `Received`, `Return-Path` and — the reason this
/// helper exists — a reconstructed `Bcc:` listing the envelope recipients that
/// appear in no header. That annotation is Mailpit telling us the blind
/// delivery worked, but it sits in the same response as the message, so a naive
/// substring check reads Mailpit's own note as a disclosure by us.
///
/// Everything from our `Message-ID` onwards is ours: it is the first header
/// `Draft::build` emits.
fn as_transmitted<'a>(raw: &'a str, message_id: &str) -> &'a str {
    let marker = format!("Message-ID: <{message_id}>");
    let start = raw
        .find(&marker)
        .unwrap_or_else(|| panic!("raw source does not contain {marker}:\n{raw}"));
    &raw[start..]
}

#[tokio::test]
async fn a_reply_threads_onto_the_message_it_answers() {
    if !sink_available() {
        return;
    }
    let http = reqwest::Client::new();

    let source = core_smtp::ReplySource {
        message_id: Some("parent-1@example.com".into()),
        references: vec!["root-0@example.com".into()],
        subject: Some("AW: Rechnung".into()),
        from: Some(Mailbox::named("Jane Doe", "jane@example.com")),
        to: vec![Mailbox::new("dev@kuverta.test")],
        date_utc: Some(1_757_000_000),
        body: Some("Kannst du das prüfen?".into()),
        ..Default::default()
    };

    let built = core_smtp::Draft::reply(
        Mailbox::named("Erika", "dev@kuverta.test"),
        &source,
        core_smtp::ReplyMode::Sender,
    )
    .build()
    .expect("build");

    submit(&config(), "dev@kuverta.test", &auth(), &built)
        .await
        .expect("submit");

    let message = message_with_id(&http, &built.message_id).await;
    // The German reply prefix is collapsed rather than stacked.
    assert_eq!(message["Subject"], "Re: Rechnung");

    let raw = raw_source(&http, message["ID"].as_str().unwrap()).await;
    assert!(raw.contains("In-Reply-To: <parent-1@example.com>"));
    assert!(raw.contains("References: <root-0@example.com> <parent-1@example.com>"));
}
