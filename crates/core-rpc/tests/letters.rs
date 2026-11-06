//! A document posted on paper, against a LetterXpress that is not there.
//!
//! The client's own requests are core-letter's to test. What is tested here
//! is the ledger around them: what is checked before anything is sent, that
//! the letter is written down before LetterXpress is asked, and what is kept
//! of what it answered — and, for a letter that was printed, the record of
//! it filed in the account's Sent folder, on a mail server that is not there
//! either.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use core_letter::{Credentials, LetterXpress, Mode, Spec};
use core_rpc::pdf::Window;
use core_rpc::table::{Cell, Format, Table};
use core_rpc::{letters, Core, LetterView, Session};
use core_store::model::*;
use core_store::{Blobs, Store};
use mail_parser::MimeHeaders;

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

// -- the record in Sent --------------------------------------------------------

/// A mail server that takes any login, has one folder and calls it Sent,
/// keeps what is appended to it, and refuses everything else — fetching the
/// folder back included, which the filing shrugs off.
struct MailServer {
    port: u16,
    appended: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MailServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let appended = Arc::new(Mutex::new(Vec::new()));
        let kept = appended.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                Self::talk(stream, &kept);
            }
        });
        Self { port, appended }
    }

    fn talk(mut stream: TcpStream, appended: &Mutex<Vec<Vec<u8>>>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        stream.write_all(b"* OK ready\r\n").ok();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let line = line.trim_end();
            let mut words = line.splitn(3, ' ');
            let tag = words.next().unwrap_or_default();
            let answer = match words.next().unwrap_or_default() {
                "LOGIN" => format!("{tag} OK in\r\n"),
                "LIST" => {
                    format!("* LIST (\\HasNoChildren \\Sent) \"/\" Sent\r\n{tag} OK listed\r\n")
                }
                "APPEND" => {
                    let size = line
                        .rsplit_once('{')
                        .and_then(|(_, size)| size.trim_end_matches('}').parse().ok())
                        .unwrap_or(0);
                    stream.write_all(b"+ go on\r\n").ok();
                    let mut message = vec![0u8; size];
                    reader.read_exact(&mut message).ok();
                    reader.read_line(&mut String::new()).ok();
                    appended.lock().unwrap().push(message);
                    format!("{tag} OK appended\r\n")
                }
                "LOGOUT" => {
                    stream
                        .write_all(format!("* BYE\r\n{tag} OK out\r\n").as_bytes())
                        .ok();
                    break;
                }
                _ => format!("{tag} NO not here\r\n"),
            };
            stream.write_all(answer.as_bytes()).ok();
        }
    }

    /// What is in Sent, as text.
    fn sent(&self) -> Vec<String> {
        self.appended
            .lock()
            .unwrap()
            .iter()
            .map(|message| String::from_utf8_lossy(message).into_owned())
            .collect()
    }
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PASSWORD_VAR: &str = "KUVERTA_LETTERS_PASSWORD";

/// A store on disk, as a session needs one, with an account whose mail
/// server is at `imap_port` on this computer.
fn on_disk(name: &str, imap_port: u16) -> (TempDir, Core, AccountId, Session) {
    let path = std::env::temp_dir().join(format!("kuverta-letters-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Erika Mustermann".into(),
            email: "erika@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port,
            imap_security: ImapSecurity::Plaintext,
            username: "erika@kuverta.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    // SAFETY: the same value every time, and nothing reads it but the
    // sessions made here.
    unsafe { std::env::set_var(PASSWORD_VAR, "devpass") };
    let session = Session::new(&dir.0).with_password_env(Some(PASSWORD_VAR.into()));
    let core = Core::new(store, Blobs::new(dir.0.join("blobs")));
    (dir, core, account, session)
}

/// A port on this computer that nothing answers on.
fn dead_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A cancellation written, sent to the LetterXpress that is not there in
/// `mode`, and recorded: the letter, and the PDF that went.
async fn posted(
    core: &Core,
    account: AccountId,
    base: &str,
    mode: Mode,
    spec: Spec,
) -> (LetterView, Vec<u8>) {
    let document = core
        .write_document(
            account,
            "Kündigung",
            "Köln, 3. November 2026\n\nKündigung meines Vertrags",
            None,
            false,
            Some(&Window {
                recipient: "Fitnessstudio Muster GmbH\nHauptstraße 1\n12345 Musterstadt",
                sender: Some("Erika Mustermann, Heidestraße 17, 51147 Köln"),
                registered: spec.registered.is_some(),
            }),
        )
        .unwrap();
    let order = core
        .letter_order_with(
            account,
            document.id,
            spec,
            None,
            client(base).with_mode(mode),
        )
        .unwrap();
    let id = core.begin_letter(&order, Some(0.81)).unwrap();
    let job = letters::submit(&order).await.unwrap();
    (core.record_letter(id, &job).unwrap(), order.pdf)
}

#[tokio::test]
async fn the_record_for_sent_names_the_letter_says_how_it_went_and_carries_the_pdf() {
    let base = serve();
    let (core, account) = core();
    let spec = Spec {
        duplex: true,
        registered: Some(letters::Registered::Drop),
        ..Spec::default()
    };
    let (letter, pdf) = posted(&core, account, &base, Mode::Live, spec).await;
    let me = core
        .store()
        .account_by_email("dev@kuverta.test")
        .unwrap()
        .unwrap();

    let raw = letters::filed_message(&me, &letter, &pdf, None).unwrap();
    let message = mail_parser::MessageParser::default().parse(&raw).unwrap();
    assert_eq!(message.subject(), Some("Letter by post: Kündigung"));
    assert_eq!(
        message.date().map(|date| date.to_timestamp()),
        Some(letter.created_at)
    );
    // Who it went to is a name with no address behind it: nothing to reply
    // to, and nobody the store would count as written to.
    match message.to() {
        Some(mail_parser::Address::Group(groups)) => {
            assert_eq!(groups[0].name.as_deref(), Some("Fitnessstudio Muster GmbH"));
            assert!(groups[0].addresses.is_empty());
        }
        other => panic!("not a group: {other:?}"),
    }
    let read = core_proto::parse::parse_message(&raw, None)
        .unwrap()
        .message;
    assert_eq!(read.from_name.as_deref(), Some("Dev"));
    assert_eq!(read.from_addr.as_deref(), Some("dev@kuverta.test"));
    assert_eq!(read.recipients, None);

    let body = message.body_text(0).unwrap();
    for line in [
        "Address, as LetterXpress read it off the page: Fitnessstudio Muster GmbH, Hauptstraße 1, 12345 Musterstadt",
        "Sent to LetterXpress: ",
        "Printed: in black and white, on both sides of the paper",
        "Registered: Einschreiben Einwurf, delivery recorded",
        "Destination: within Germany",
        "Pages: 1",
        "Cost: 0.81 EUR, VAT included",
        "LetterXpress job: 77",
        "Attached is the PDF as it was posted.",
    ] {
        assert!(body.contains(line), "{line:?} is not in:\n{body}");
    }
    assert_eq!(message.attachment_count(), 1);
    let file = message.attachment(0).unwrap();
    assert_eq!(file.attachment_name(), Some("Kündigung.pdf"));
    assert_eq!(
        file.content_type().and_then(|kind| kind.subtype()),
        Some("pdf")
    );
    assert_eq!(file.contents(), pdf);

    // Filed some time after: dated when the letter went all the same, and
    // saying that the PDF is the document, not what was in the envelope.
    let late =
        letters::filed_message(&me, &letter, &pdf, Some(letter.created_at + 86_400)).unwrap();
    let message = mail_parser::MessageParser::default().parse(&late).unwrap();
    assert_eq!(
        message.date().map(|date| date.to_timestamp()),
        Some(letter.created_at)
    );
    let body = message.body_text(0).unwrap();
    assert!(body.contains("Filed here later, on "), "{body}");
    assert!(
        body.contains("the document as kuverta keeps it now"),
        "{body}"
    );
    assert!(!body.contains("as it was posted"), "{body}");
}

#[tokio::test]
async fn a_printed_letter_is_filed_in_sent_once_and_one_in_the_postbox_never() {
    let base = serve();
    let server = MailServer::start();
    let (_dir, core, account, session) = on_disk("filed", server.port);

    // Test mode: it was never printed, so there is nothing to file.
    let (parked, pdf) = posted(&core, account, &base, Mode::Test, Spec::default()).await;
    assert_eq!(session.file_letter(parked.id, &pdf).await.unwrap(), None);
    assert!(server.sent().is_empty());

    let (letter, pdf) = posted(&core, account, &base, Mode::Live, Spec::default()).await;
    assert_eq!(
        session
            .file_letter(letter.id, &pdf)
            .await
            .unwrap()
            .as_deref(),
        Some("Sent")
    );
    let sent = server.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("LetterXpress job: 77"), "{}", sent[0]);
    assert!(sent[0].contains("Erika Mustermann"), "{}", sent[0]);

    // Asked again, and by the sweep after a sync: it is in Sent already.
    assert_eq!(session.file_letter(letter.id, &pdf).await.unwrap(), None);
    assert_eq!(session.file_letters("erika@kuverta.test").await.unwrap(), 0);
    assert_eq!(server.sent().len(), 1);
}

#[tokio::test]
async fn a_record_that_cannot_be_filed_leaves_the_letter_sent_and_is_filed_later() {
    let base = serve();
    // No mail server: the letter goes, its record does not.
    let (_dir, core, account, session) = on_disk("unfiled", dead_port());
    let (letter, pdf) = posted(&core, account, &base, Mode::Live, Spec::default()).await;

    let err = session.file_letter(letter.id, &pdf).await.unwrap_err();
    assert!(!err.to_string().is_empty());
    let kept = core.letter_view(letter.id).unwrap();
    assert_eq!(kept.state, "sent");
    assert_eq!(kept.job_id, Some(77));
    assert!(session.file_letters("erika@kuverta.test").await.is_err());
    assert_eq!(core.letter_view(letter.id).unwrap().state, "sent");

    // A letter taken back in the meantime is not filed after all.
    let (back, _) = posted(&core, account, &base, Mode::Live, Spec::default()).await;
    core.letter_cancelled(back.id).unwrap();

    // The server is there again — as after a sync that reached it.
    let server = MailServer::start();
    core.store()
        .connection()
        .execute("UPDATE account SET imap_port = ?1", [server.port])
        .unwrap();
    assert_eq!(session.file_letters("erika@kuverta.test").await.unwrap(), 1);
    let sent = server.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Filed here later, on "), "{}", sent[0]);

    // And only once, however often it is asked.
    assert_eq!(session.file_letters("erika@kuverta.test").await.unwrap(), 0);
    assert_eq!(session.file_letter(letter.id, &pdf).await.unwrap(), None);
    assert_eq!(server.sent().len(), 1);
}
