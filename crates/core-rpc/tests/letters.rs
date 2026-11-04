//! A document posted on paper, against a LetterXpress that is not there.
//!
//! The client's own requests are core-letter's to test. What is tested here
//! is the ledger around them: what is checked before anything is sent, that
//! the letter is written down before LetterXpress is asked, and what is kept
//! of what it answered.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use core_letter::{Credentials, LetterXpress, Mode, Spec};
use core_rpc::pdf::Window;
use core_rpc::table::{Cell, Format, Table};
use core_rpc::{letters, Core};
use core_store::model::*;
use core_store::{Blobs, Store};

/// Answers every request with the same job, and prices and balances.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v3", listener.local_addr().unwrap());
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
            reader.read_exact(&mut sent).ok();
            let body = if request.contains("/balance") {
                r#"{"status":200,"message":"OK","data":{"balance":12.5,"currency":"EUR"}}"#
            } else if request.contains("/price") {
                r#"{"status":200,"message":"OK","data":{"price":0.81}}"#
            } else {
                r#"{"status":200,"message":"OK","data":{"id":77,"status":"draft","registered":null,"items":[{"address":"Fitnessstudio Muster GmbH, Hauptstraße 1, 12345 Musterstadt","pages":1,"amount":0.68,"vat":0.13,"status":"draft"}]}}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    base
}

fn client(base: &str) -> LetterXpress {
    LetterXpress::at(
        base,
        Credentials {
            username: "erika".into(),
            api_key: "secret".into(),
            mode: Mode::Test,
        },
    )
}

fn core() -> (Core, AccountId) {
    let store = Store::open_in_memory().unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "dev@kuverta.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let blobs =
        Blobs::new(std::env::temp_dir().join(format!("kuverta-letters-{}", std::process::id())));
    (Core::new(store, blobs), account)
}

#[tokio::test]
async fn a_letter_is_quoted_written_down_sent_and_remembered_on_its_document() {
    let base = serve();
    let (core, account) = core();
    let document = core
        .write_document(
            account,
            "Kündigung",
            "Köln, 3. November 2026\n\nKündigung meines Vertrags\n\nSehr geehrte Damen und Herren,",
            None,
            false,
            Some(&Window {
                recipient: "Fitnessstudio Muster GmbH\nHauptstraße 1\n12345 Musterstadt",
                sender: Some("Erika Mustermann, Heidestraße 17, 51147 Köln"),
                registered: true,
            }),
        )
        .unwrap();

    let order = core
        .letter_order_with(account, document.id, Spec::default(), None, client(&base))
        .unwrap();
    assert_eq!(order.pages, 1);
    assert!(order.sent_before.is_empty());
    let quote = letters::quote(&order).await.unwrap();
    assert_eq!(quote.price, 0.81);
    assert_eq!(quote.mode, "test");
    assert_eq!(quote.balance.unwrap().balance, 12.5);

    let id = core.begin_letter(&order, Some(quote.price)).unwrap();
    assert_eq!(core.letter_view(id).unwrap().state, "sending");
    let job = letters::submit(&order).await.unwrap();
    let letter = core.record_letter(id, &job).unwrap();
    assert_eq!(letter.state, "sent");
    assert_eq!(letter.job_id, Some(77));
    assert_eq!(letter.mode, "test");
    assert_eq!(
        letter.address.as_deref(),
        Some("Fitnessstudio Muster GmbH, Hauptstraße 1, 12345 Musterstadt")
    );
    assert!((letter.cost.unwrap() - 0.81).abs() < 1e-9);
    assert!(letter.cancellable_until.is_some());

    // Posting it again is a choice the window can show as one.
    let again = core
        .letter_order_with(account, document.id, Spec::default(), None, client(&base))
        .unwrap();
    assert_eq!(again.sent_before.len(), 1);
    assert_eq!(core.letters(account).unwrap().len(), 1);
}

#[test]
fn what_cannot_be_printed_is_refused_before_anything_is_sent() {
    // No server: nothing here may reach one.
    let (core, account) = core();
    let sheet = core
        .write_table(
            account,
            "Schulden",
            &Table {
                columns: vec!["Gläubiger".into()],
                rows: vec![vec![Cell::Text("Stadtwerke".into())]],
            },
            Format::Xlsx,
            None,
        )
        .unwrap();
    let err = core
        .letter_order_with(
            account,
            sheet.id,
            Spec::default(),
            None,
            client("http://127.0.0.1:9"),
        )
        .err()
        .unwrap();
    assert!(err.to_string().contains("not a PDF"), "{err}");

    let letter = core
        .write_document(account, "Brief", "Text", None, false, None)
        .unwrap();
    let abroad_registered = Spec {
        international: true,
        registered: Some(letters::Registered::Signed),
        ..Spec::default()
    };
    let err = core
        .letter_order_with(
            account,
            letter.id,
            abroad_registered,
            None,
            client("http://127.0.0.1:9"),
        )
        .err()
        .unwrap();
    assert!(err.to_string().contains("only within Germany"), "{err}");

    let other = core
        .store()
        .add_account(&NewAccount {
            label: "Other".into(),
            email: "other@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "other@kuverta.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(core
        .letter_order_with(
            other,
            letter.id,
            Spec::default(),
            None,
            client("http://127.0.0.1:9")
        )
        .is_err());
    assert!(core.letters(account).unwrap().is_empty());
}

#[tokio::test]
async fn an_address_given_by_hand_goes_on_a_cover_sheet_and_the_sender_is_offered_again() {
    let base = serve();
    let (core, account) = core();
    let form = core
        .write_document(
            account,
            "Antrag",
            "Antrag auf Verlängerung",
            None,
            false,
            None,
        )
        .unwrap();
    let address = core_rpc::LetterAddress {
        recipient: "Stadt Musterstadt\nRathausplatz 1\n12345 Musterstadt".into(),
        sender: Some("Erika Mustermann, Heidestraße 17, 51147 Köln".into()),
        cover: true,
    };
    let duplex = Spec {
        duplex: true,
        ..Spec::default()
    };
    let order = core
        .letter_order_with(account, form.id, duplex, Some(&address), client(&base))
        .unwrap();
    // The cover and its empty back, then the form.
    assert_eq!(order.pages, 3);
    // The document itself is left as it was.
    assert_eq!(core.document_view(form.id).unwrap().pages, Some(1));

    assert_eq!(core.letter_settings().unwrap().sender, None);
    let id = core.begin_letter(&order, None).unwrap();
    assert_eq!(core.letter_view(id).unwrap().pages, 3);
    assert_eq!(
        core.letter_settings().unwrap().sender.as_deref(),
        Some("Erika Mustermann, Heidestraße 17, 51147 Köln")
    );

    let over = core_rpc::LetterAddress {
        cover: false,
        ..address
    };
    let (_, _, pages) = core
        .letter_pdf(account, form.id, &Spec::default(), Some(&over))
        .unwrap();
    assert_eq!(pages, 1);
}
