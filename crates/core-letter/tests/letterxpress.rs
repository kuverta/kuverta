//! The LetterXpress client against a LetterXpress that is not there.
//!
//! A canned server on a loopback socket that keeps every request, so the
//! assertions are on what was sent — the credentials in every body, the
//! checksum of the base64, the method — as much as on what came back. What
//! it cannot check is whether LetterXpress answers in these shapes; a letter
//! in test mode, which lands in the Postbox and costs nothing, is for that.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use base64::Engine;
use core_letter::{Credentials, Letter, LetterError, LetterXpress, Mode, Registered, Spec};
use md5::{Digest, Md5};
use serde_json::Value;

/// A request as the server saw it: method, path and JSON body.
struct Seen {
    method: String,
    path: String,
    body: Value,
}

fn serve(answer: fn(&str, &str) -> (u16, String)) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v3", listener.local_addr().unwrap());
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
            reader.read_exact(&mut sent).ok();
            let mut parts = request.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let (status, body) = answer(&method, &path);
            let _ = tx.send(Seen {
                method,
                path,
                body: serde_json::from_slice(&sent).unwrap_or(Value::Null),
            });
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, rx)
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

const JOB: &str = r#"{"status":200,"message":"OK","data":{"id":6035143,"shipping":"national","mode":"simplex","color":"1","c4":0,"registered":"r1","bank_form":0,"notice":null,"status":"draft","group":null,"dispatch_date":null,"filename_original":"Brief.pdf","created_at":"2026-11-03 13:38:07","updated_at":"2026-11-03 13:38:07","items":[{"address":"Erika Mustermann, Heidestr. 17, 51147 Köln","pages":1,"amount":0.68,"vat":0.13,"status":"draft"}]}}"#;

#[tokio::test]
async fn every_request_carries_the_credentials_and_the_mode() {
    let (base, seen) = serve(|_, _| {
        (
            200,
            r#"{"status":200,"message":"OK","data":{"balance":54.89,"currency":"EUR"}}"#.into(),
        )
    });
    let balance = client(&base).balance().await.unwrap();
    assert_eq!(balance.balance, 54.89);
    assert_eq!(balance.currency, "EUR");

    let request = seen.recv().unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/v3/balance");
    assert_eq!(request.body["auth"]["username"], "erika");
    assert_eq!(request.body["auth"]["apikey"], "secret");
    assert_eq!(request.body["auth"]["mode"], "test");
}

#[tokio::test]
async fn a_price_is_asked_for_the_pages_and_the_registration() {
    let (base, seen) = serve(|_, _| {
        (
            200,
            r#"{"status":200,"message":"OK","data":{"price":4.99,"letter":{}}}"#.into(),
        )
    });
    let spec = Spec {
        duplex: true,
        registered: Some(Registered::Drop),
        ..Spec::default()
    };
    assert_eq!(client(&base).price(&spec, 3).await.unwrap(), 4.99);

    let request = seen.recv().unwrap();
    assert_eq!(request.path, "/v3/price");
    let letter = &request.body["letter"];
    assert_eq!(letter["specification"]["pages"], 3);
    assert_eq!(letter["specification"]["mode"], "duplex");
    assert_eq!(letter["specification"]["color"], "1");
    assert_eq!(letter["specification"]["shipping"], "national");
    assert_eq!(letter["registered"], "r1");
}

#[tokio::test]
async fn a_letter_goes_as_base64_with_the_md5_of_that_base64() {
    let (base, seen) = serve(|_, _| (200, JOB.into()));
    let pdf = b"%PDF-1.5\n% a letter\n%%EOF\n";
    let job = client(&base)
        .submit(&Letter {
            pdf,
            spec: Spec {
                registered: Some(Registered::Drop),
                ..Spec::default()
            },
            filename: Some("Brief.pdf"),
            notice: Some("Kündigung"),
        })
        .await
        .unwrap();
    assert_eq!(job.id, 6035143);
    assert_eq!(job.status, "draft");
    assert_eq!(
        job.address(),
        Some("Erika Mustermann, Heidestr. 17, 51147 Köln")
    );
    assert!((job.total() - 0.81).abs() < 1e-9);

    let request = seen.recv().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v3/printjobs");
    let letter = &request.body["letter"];
    let file = letter["base64_file"].as_str().unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(file)
            .unwrap(),
        pdf
    );
    assert_eq!(
        letter["base64_file_checksum"].as_str().unwrap(),
        format!("{:x}", Md5::digest(file.as_bytes()))
    );
    assert_eq!(letter["registered"], "r1");
    assert_eq!(letter["filename_original"], "Brief.pdf");
    assert_eq!(letter["notice"], "Kündigung");
    assert_eq!(request.body["auth"]["mode"], "test");
}

#[tokio::test]
async fn what_is_not_a_pdf_is_not_sent() {
    let (base, seen) = serve(|_, _| (200, JOB.into()));
    let err = client(&base)
        .submit(&Letter {
            pdf: b"PK\x03\x04 a spreadsheet",
            spec: Spec::default(),
            filename: None,
            notice: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, LetterError::Invalid(_)), "{err}");
    assert!(seen.try_recv().is_err(), "nothing should have been sent");
}

#[tokio::test]
async fn a_job_is_fetched_and_taken_back_by_its_id() {
    let (base, seen) = serve(|method, _| {
        if method == "DELETE" {
            (
                200,
                r#"{"status":200,"message":"Print job deleted successfully"}"#.into(),
            )
        } else {
            (200, JOB.into())
        }
    });
    let lx = client(&base);
    assert_eq!(lx.job(6035143).await.unwrap().id, 6035143);
    lx.cancel(6035143).await.unwrap();
    let first = seen.recv().unwrap();
    assert_eq!(
        (first.method.as_str(), first.path.as_str()),
        ("GET", "/v3/printjobs/6035143")
    );
    let second = seen.recv().unwrap();
    assert_eq!(
        (second.method.as_str(), second.path.as_str()),
        ("DELETE", "/v3/printjobs/6035143")
    );
}

#[tokio::test]
async fn wrong_credentials_and_refusals_say_which_they_are() {
    let (base, _) = serve(|_, _| (401, r#"{"message":"Unauthorized."}"#.into()));
    assert!(matches!(
        client(&base).balance().await.unwrap_err(),
        LetterError::Auth
    ));

    let (base, _) = serve(|_, _| {
        (
            403,
            r#"{"message":"A job can only be deleted within 15 minutes."}"#.into(),
        )
    });
    let err = client(&base).cancel(1).await.unwrap_err();
    assert!(
        matches!(&err, LetterError::Refused { status: 403, message } if message.contains("15 minutes")),
        "{err}"
    );
}
