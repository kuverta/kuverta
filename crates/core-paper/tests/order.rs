//! Post listed by the date on the letter or by when it was scanned, and a
//! mailbox's ids for telling read from unread.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use core_paper::{Order, Paperless, Selector};

/// Answers every request with `body` and hands back each request line.
fn serve(body: &'static str) -> (String, mpsc::Receiver<String>) {
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
            let _ = tx.send(request.trim().to_string());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, rx)
}

const EMPTY: &str = r#"{"count":0,"next":null,"results":[]}"#;

fn documents_request(requests: &mpsc::Receiver<String>) -> String {
    requests
        .try_iter()
        .find(|line| line.contains("/api/documents/"))
        .expect("a documents request")
}

#[tokio::test]
async fn post_is_listed_by_the_letters_date_unless_scan_time_is_asked_for() {
    let (base, requests) = serve(EMPTY);
    let client = Paperless::new(&base, "token").unwrap();

    client.documents(&Selector::Everything, 0, 10, None).await.unwrap();
    assert!(documents_request(&requests).contains("ordering=-created"));

    client
        .documents_in_order(&Selector::Everything, 0, 10, None, Order::Added)
        .await
        .unwrap();
    assert!(documents_request(&requests).contains("ordering=-added"));
}

#[tokio::test]
async fn a_mailboxs_ids_are_asked_for_alone_and_within_its_selector() {
    let (base, requests) = serve(r#"{"count":3,"next":null,"results":[{"id":6},{"id":5},{"id":4}]}"#);
    let client = Paperless::new(&base, "token").unwrap();

    let ids = client
        .document_ids(&Selector::Tag("Vogelhüttendeich 28".into()))
        .await
        .unwrap();

    assert_eq!(ids, vec![6, 5, 4]);
    let request = documents_request(&requests);
    assert!(request.contains("fields=id"), "{request}");
    assert!(request.contains("tags__name__iexact=Vogelh%C3%BCttendeich%2028"), "{request}");
}

#[test]
fn the_order_travels_as_a_word() {
    assert_eq!(serde_json::to_string(&Order::Added).unwrap(), "\"added\"");
    assert_eq!(serde_json::from_str::<Order>("\"created\"").unwrap(), Order::Created);
    assert_eq!(Order::default(), Order::Created);
}
