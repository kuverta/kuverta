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

type Route = fn(&str) -> (&'static str, String);

/// A server that answers each request by `route` and hands back everything
/// it was sent.
fn serve_with(route: Route) -> (String, mpsc::Receiver<String>) {
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

            let (status, body) = route(&request);
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

/// The tags this fake Paperless knows, as `(encoded lookup, id, name)`.
const TAGS: &[(&str, u64, &str)] = &[
    ("post", 7, "post"),
    ("home", 9, "Home"),
    ("Hauptstra%C3%9Fe%2012", 1, "Hauptstraße 12"),
];

/// Answers a tag lookup as Paperless does: a page, empty when nothing matches.
fn tag_lookup(request_line: &str) -> String {
    let query = request_line
        .split("name__iexact=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    let results: Vec<String> = TAGS
        .iter()
        .filter(|(lookup, _, _)| lookup.eq_ignore_ascii_case(query))
        .map(|(_, id, name)| format!("{{\"id\":{id},\"name\":\"{name}\"}}"))
        .collect();
    format!(
        "{{\"count\":{},\"results\":[{}]}}",
        results.len(),
        results.join(",")
    )
}

/// A Paperless that knows [`TAGS`] and answers uploads with `status` and `body`.
macro_rules! paperless {
    ($status:expr, $body:expr) => {
        serve_with(|request| {
            if request.starts_with("GET /api/tags/") {
                ("200 OK", tag_lookup(request))
            } else {
                ($status, $body.to_string())
            }
        })
    };
}

fn uploader(base: &str, tags: &[&str]) -> Uploader {
    Uploader::new(
        base,
        "secret-token",
        tags.iter().map(|tag| tag.to_string()).collect(),
        "Post ".into(),
    )
    .unwrap()
}

/// Everything the server received so far. Each request is recorded before it
/// is answered, so once a send has returned, all of its requests are here.
fn received(sent: &mpsc::Receiver<String>) -> Vec<String> {
    sent.try_iter().collect()
}

fn the_upload(requests: &[String]) -> &String {
    requests
        .iter()
        .find(|request| request.starts_with("POST /api/documents/post_document/"))
        .unwrap_or_else(|| panic!("nothing was uploaded: {requests:?}"))
}

#[tokio::test]
async fn a_capture_is_posted_as_multipart_with_its_tags() {
    let (base, sent) = paperless!("200 OK", "\"3f2c-task-id\"");

    let task = uploader(&base, &["post", "home"])
        .send("1757600000-7.jpg", b"\xff\xd8jpeg-bytes".to_vec(), &[])
        .await
        .unwrap();

    // The task id is the only handle on a document whose OCR has not finished,
    // and the only way to tell a real success from a successful-looking one.
    assert_eq!(task, "3f2c-task-id");

    let requests = received(&sent);
    let request = the_upload(&requests);
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

    // The title comes from the filename without its extension.
    assert!(
        request.contains("\r\n\r\nPost 1757600000-7\r\n"),
        "{request}"
    );
}

#[tokio::test]
async fn tags_are_sent_as_the_ids_paperless_knows_them_by() {
    // Found against a real Paperless: the upload endpoint refuses a tag name
    // with `Expected pk value, received str`, so every tagged capture failed
    // and sat in the spool. The fake used to accept names and hid it.
    let (base, sent) = paperless!("200 OK", "\"task\"");
    uploader(&base, &["post", "home"])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap();

    let requests = received(&sent);
    let request = the_upload(&requests);
    // One `tags` field per tag, which is how Paperless takes repeats.
    assert_eq!(request.matches("name=\"tags\"").count(), 2, "{request}");
    assert!(request.contains("name=\"tags\"\r\n\r\n7\r\n"), "{request}");
    assert!(request.contains("name=\"tags\"\r\n\r\n9\r\n"), "{request}");
    assert!(!request.contains("\r\n\r\npost\r\n"), "{request}");
}

#[tokio::test]
async fn a_tag_name_with_an_umlaut_and_a_space_is_looked_up_encoded() {
    let (base, sent) = paperless!("200 OK", "\"task\"");
    uploader(&base, &["Hauptstraße 12"])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap();

    let requests = received(&sent);
    assert!(
        requests[0].starts_with("GET /api/tags/?name__iexact=Hauptstra%C3%9Fe%2012 "),
        "{requests:?}"
    );
    assert!(
        the_upload(&requests).contains("name=\"tags\"\r\n\r\n1\r\n"),
        "{requests:?}"
    );
}

#[tokio::test]
async fn a_tag_paperless_does_not_have_fails_by_name_and_uploads_nothing() {
    // Nothing half-tagged: an upload without the address tag is post that
    // belongs to no address, which is worse than post that waits.
    let (base, sent) = paperless!("200 OK", "\"task\"");
    let err = uploader(&base, &["post", "Nebenstraße 3"])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap_err();

    assert!(err.to_string().contains("Nebenstraße 3"), "{err}");
    let requests = received(&sent);
    assert!(
        requests.iter().all(|request| request.starts_with("GET ")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn tags_are_looked_up_once_not_per_capture() {
    let (base, sent) = paperless!("200 OK", "\"task\"");
    let uploader = uploader(&base, &["post", "home"]);
    uploader.send("a.jpg", b"x".to_vec(), &[]).await.unwrap();
    uploader.send("b.jpg", b"y".to_vec(), &[]).await.unwrap();

    let requests = received(&sent);
    let lookups = requests
        .iter()
        .filter(|request| request.starts_with("GET /api/tags/"))
        .count();
    assert_eq!(lookups, 2, "{requests:?}");
    assert_eq!(requests.len(), 4, "{requests:?}");
}

#[tokio::test]
async fn without_tags_nothing_is_looked_up() {
    let (base, sent) = paperless!("200 OK", "\"task\"");
    uploader(&base, &[])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap();

    let requests = received(&sent);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(!the_upload(&requests).contains("name=\"tags\""));
}

#[tokio::test]
async fn the_body_ends_with_a_closing_boundary() {
    // Without the trailing `--`, Django reads the body as truncated and
    // rejects it — the classic hand-rolled-multipart bug.
    let (base, sent) = paperless!("200 OK", "\"ok\"");
    uploader(&base, &["post"])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap();

    let requests = received(&sent);
    let request = the_upload(&requests);
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
    let (base, _sent) = paperless!("403 Forbidden", "{}");
    let err = uploader(&base, &[])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("rejected the token"), "{err}");
}

#[tokio::test]
async fn a_token_rejected_at_the_tag_lookup_says_so_too() {
    // The lookup is now the first request a bad token meets.
    let (base, sent) = serve_with(|_| ("401 Unauthorized", "{}".into()));
    let err = uploader(&base, &["post"])
        .send("a.jpg", b"x".to_vec(), &[])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("rejected the token"), "{err}");
    assert_eq!(received(&sent).len(), 1);
}

#[tokio::test]
async fn a_server_error_keeps_what_the_server_said() {
    // A capture is about to be kept for a retry on the strength of this, so
    // the reason has to survive into the log.
    let (base, _sent) = paperless!("500 Internal Server Error", "consumer is down");
    let err = uploader(&base, &["post"])
        .send("a.jpg", b"x".to_vec(), &[])
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
