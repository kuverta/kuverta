//! The uploader, against a Paperless on loopback.
//!
//! The multipart body is written out by hand, which is the kind of code that
//! works against one server and silently not against another — so the test
//! asserts on the bytes rather than only on the outcome.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use scannerd::upload::Uploader;

/// A server that answers once and hands back everything it was sent.
fn serve(status: &'static str, body: &'static str) -> (String, mpsc::Receiver<String>) {
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
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                headers.push_str(&line);
            }

            let mut payload = vec![0u8; length];
            reader.read_exact(&mut payload).ok();

            let _ = tx.send(format!(
                "{request}{headers}\r\n{}",
                String::from_utf8_lossy(&payload)
            ));

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });

    (base, rx)
}

fn uploader(base: &str) -> Uploader {
    Uploader::new(
        base,
        "secret-token",
        vec!["post".into(), "home".into()],
        "Post ".into(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_capture_is_posted_as_multipart_with_its_tags() {
    let (base, sent) = serve("200 OK", "\"3f2c-task-id\"");

    let task = uploader(&base)
        .send("1757600000-7.jpg", b"\xff\xd8jpeg-bytes".to_vec())
        .await
        .unwrap();

    // The task id is the only handle on a document whose OCR has not finished,
    // and the only way to tell a real success from a successful-looking one.
    assert_eq!(task, "3f2c-task-id");

    let request = sent.recv().unwrap();
    assert!(
        request.contains("POST /api/documents/post_document/"),
        "{request}"
    );
    // Header names arrive lowercased on the wire, so the check is too.
    assert!(
        request
            .to_lowercase()
            .contains("authorization: token secret-token"),
        "{request}"
    );
    assert!(
        request.contains("multipart/form-data; boundary="),
        "{request}"
    );

    // The file part, named as Paperless expects.
    assert!(
        request.contains("name=\"document\"; filename=\"1757600000-7.jpg\""),
        "{request}"
    );
    assert!(request.contains("Content-Type: image/jpeg"), "{request}");
    assert!(request.contains("jpeg-bytes"), "{request}");

    // One `tags` field per tag, which is how Paperless takes repeats.
    assert_eq!(request.matches("name=\"tags\"").count(), 2, "{request}");
    assert!(request.contains("\r\n\r\npost\r\n"), "{request}");
    assert!(request.contains("\r\n\r\nhome\r\n"), "{request}");

    // The title comes from the filename without its extension.
    assert!(
        request.contains("\r\n\r\nPost 1757600000-7\r\n"),
        "{request}"
    );
}

#[tokio::test]
async fn the_body_ends_with_a_closing_boundary() {
    // Without the trailing `--`, Django reads the body as truncated and
    // rejects it — the classic hand-rolled-multipart bug.
    let (base, sent) = serve("200 OK", "\"ok\"");
    uploader(&base).send("a.jpg", b"x".to_vec()).await.unwrap();

    let request = sent.recv().unwrap();
    let boundary = request
        .split("boundary=")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap()
        .to_string();
    assert!(
        request.ends_with(&format!("--{boundary}--\r\n")),
        "{request}"
    );
}

#[tokio::test]
async fn a_rejected_token_says_so_rather_than_being_a_status_code() {
    let (base, _sent) = serve("403 Forbidden", "{}");
    let err = uploader(&base)
        .send("a.jpg", b"x".to_vec())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("rejected the token"), "{err}");
}

#[tokio::test]
async fn a_server_error_keeps_what_the_server_said() {
    // A capture is about to be kept for a retry on the strength of this, so
    // the reason has to survive into the log.
    let (base, _sent) = serve("500 Internal Server Error", "consumer is down");
    let err = uploader(&base)
        .send("a.jpg", b"x".to_vec())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("500"), "{err}");
    assert!(err.to_string().contains("consumer is down"), "{err}");
}

#[tokio::test]
async fn a_url_without_a_scheme_is_refused_before_anything_is_captured() {
    assert!(Uploader::new("localhost:8000", "t", vec![], String::new()).is_err());
    assert!(Uploader::new("http://localhost:8000/", "t", vec![], String::new()).is_ok());
}
