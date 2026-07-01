//! A document's file, for reading the scan when its OCR text is no use.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use core_paper::{PaperError, Paperless};

/// A server that answers every request with `status`, `content_type` and
/// `body`, once each.
fn serve(status: &'static str, content_type: &'static str, body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(body).ok();
        }
    });
    base
}

#[tokio::test]
async fn the_file_comes_back_as_bytes_with_its_type() {
    // Not text: a PDF holds bytes no UTF-8 decoding may touch.
    let body: &'static [u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\nbinary\x00bytes";
    let base = serve("200 OK", "application/pdf", body);

    let download = Paperless::new(&base, "token")
        .unwrap()
        .download(4)
        .await
        .unwrap();

    assert_eq!(download.content_type, "application/pdf");
    assert_eq!(download.bytes, body);
}

#[tokio::test]
async fn a_content_type_is_read_without_its_parameters() {
    // An original that Paperless never archived is served as it was uploaded.
    let base = serve("200 OK", "image/jpeg; charset=binary", b"\xff\xd8photo");
    let download = Paperless::new(&base, "token")
        .unwrap()
        .download(4)
        .await
        .unwrap();
    assert_eq!(download.content_type, "image/jpeg");
}

#[tokio::test]
async fn a_refused_token_is_an_auth_error_not_a_broken_file() {
    let base = serve("403 Forbidden", "application/json", b"{\"detail\":\"no\"}");
    let err = Paperless::new(&base, "wrong")
        .unwrap()
        .download(4)
        .await
        .unwrap_err();
    assert!(matches!(err, PaperError::Auth), "got {err:?}");
}

/// Against the Paperless in `docker/`, when it is up: a scanned letter really
/// downloads as a PDF. Skips otherwise; `KUVERTA_REQUIRE_DEV_SERVER=1` makes
/// that a failure, as in `dev_server.rs`.
#[tokio::test]
async fn a_document_downloads_from_the_dev_paperless_as_a_pdf() {
    let up = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:8000".parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();
    if !up {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev Paperless on localhost:8000 is required but not reachable");
        }
        eprintln!("skipping: dev Paperless not running (`make dev-up`)");
        return;
    }

    let body = reqwest::Client::new()
        .post("http://localhost:8000/api/token/")
        .header("Content-Type", "application/json")
        .body(r#"{"username":"admin","password":"admin"}"#)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let token: serde_json::Value = serde_json::from_str(&body).unwrap();
    let client = Paperless::new("http://localhost:8000", token["token"].as_str().unwrap()).unwrap();

    let page = client
        .documents(&core_paper::Selector::Everything, 0, 1, None)
        .await
        .unwrap();
    let Some(document) = page.documents.first() else {
        eprintln!("skipping: the dev Paperless holds no documents");
        return;
    };

    let download = client.download(document.id).await.unwrap();
    assert_eq!(download.content_type, "application/pdf");
    assert!(
        download.bytes.starts_with(b"%PDF"),
        "{:?}",
        &download.bytes[..8.min(download.bytes.len())]
    );
}
