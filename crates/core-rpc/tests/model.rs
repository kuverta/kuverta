//! The model pass: after sync, beside the rules, never shown.
//!
//! Against a real store and a stand-in Ollama on loopback. The properties that
//! matter are not whether the model is right — that is `fuckmail eval`'s job —
//! but that its verdicts land where they can be compared and nowhere else.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use core_rpc::Core;
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("fuckmail-model-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn core_with(name: &str, count: usize) -> (Core, AccountId, TempDir) {
    let dir = TempDir::new(name);
    let store = Store::open(dir.0.join("fuckmail.db")).unwrap();
    let blobs = Blobs::new(dir.0.join("blobs"));
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@fuckmail.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "dev@fuckmail.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    for n in 0..count {
        store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("model-{n}@example.com")),
                    subject: Some(format!("Ihre Rechnung {n}")),
                    from_name: Some("Hosting AG".into()),
                    from_addr: Some("rechnung@hosting.example.de".into()),
                    date_utc: Some(1_700_000_000 + n as i64),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: n as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap();
    }
    (Core::new(store, blobs), account, dir)
}

/// An Ollama that always gives the same answer.
fn ollama_saying(answer: &'static str) -> core_ai::Ollama {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
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
            let mut payload = vec![0u8; length];
            reader.read_exact(&mut payload).ok();

            let body =
                format!(r#"{{"message":{{"role":"assistant","content":"{answer}"}},"done":true}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    core_ai::Ollama::new(&base).unwrap()
}

/// An Ollama that is not there: the port was open a moment ago.
fn ollama_down() -> core_ai::Ollama {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    core_ai::Ollama::new(&base).unwrap()
}

fn first_id(core: &Core, account: AccountId) -> MessageId {
    core.messages(account, 0, 10, &ListFilter::default())
        .unwrap()
        .rows[0]
        .id
}

#[tokio::test]
async fn a_model_verdict_is_recorded_beside_the_rules_and_changes_nothing_shown() {
    let (core, account, _dir) = core_with("beside", 3);
    let id = first_id(&core, account);
    core.store()
        .record_verdict(
            id,
            &Verdict {
                category: "newsletter".into(),
                confidence: Some(0.9),
                source: ClassifierSource::Rules,
                model: None,
                latency_ms: Some(0),
            },
        )
        .unwrap();

    let pass = core
        .model_pass(
            account,
            &ollama_saying("marketing"),
            &core_ai::PromptClassifier::new("stand-in"),
            10,
        )
        .await
        .unwrap();

    assert_eq!(pass.waiting, 3);
    assert_eq!(pass.classified, 3);
    assert_eq!(pass.failed, 0);

    // Never shown: a model can be wrong for a month without mail moving.
    let rows = core
        .messages(account, 0, 10, &ListFilter::default())
        .unwrap()
        .rows;
    let row = rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row.category.as_deref(), Some("newsletter"));

    // Compared: which is the whole reason to run it.
    let disagreements = core.store().disagreements(account).unwrap();
    assert_eq!(disagreements.len(), 1);
    assert_eq!(disagreements[0].rules_category, "newsletter");
    assert_eq!(disagreements[0].model_category, "marketing");
}

#[tokio::test]
async fn a_second_pass_does_not_ask_the_model_again() {
    let (core, account, _dir) = core_with("again", 2);
    let ollama = ollama_saying("transactional");
    let classifier = core_ai::PromptClassifier::new("stand-in");

    core.model_pass(account, &ollama, &classifier, 10)
        .await
        .unwrap();
    let second = core
        .model_pass(account, &ollama, &classifier, 10)
        .await
        .unwrap();

    assert_eq!(second.waiting, 0);
    assert_eq!(second.classified, 0);
}

#[tokio::test]
async fn a_different_model_gets_a_pass_of_its_own() {
    // Otherwise trying a second model would find nothing to do, and two models
    // could never be compared.
    let (core, account, _dir) = core_with("second-model", 2);
    let ollama = ollama_saying("transactional");

    core.model_pass(
        account,
        &ollama,
        &core_ai::PromptClassifier::new("first"),
        10,
    )
    .await
    .unwrap();
    let other = core
        .model_pass(
            account,
            &ollama,
            &core_ai::PromptClassifier::new("second"),
            10,
        )
        .await
        .unwrap();

    assert_eq!(other.waiting, 2);
}

#[tokio::test]
async fn an_answer_that_names_nothing_is_counted_as_not_knowing() {
    let (core, account, _dir) = core_with("unparseable", 2);
    let pass = core
        .model_pass(
            account,
            &ollama_saying("no idea, sorry"),
            &core_ai::PromptClassifier::new("stand-in"),
            10,
        )
        .await
        .unwrap();

    assert_eq!(pass.classified, 2);
    assert_eq!(pass.unparseable, 2);
}

#[tokio::test]
async fn a_model_server_that_is_down_stops_the_pass_instead_of_grinding_on() {
    let (core, account, _dir) = core_with("down", 10);
    let pass = core
        .model_pass(
            account,
            &ollama_down(),
            &core_ai::PromptClassifier::new("stand-in"),
            10,
        )
        .await
        .unwrap();

    assert_eq!(pass.classified, 0);
    assert_eq!(pass.failed, 3, "should give up after three in a row");
    assert!(pass.stopped.is_some());
}
