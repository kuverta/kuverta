//! Scaffolding shared by the OAuth test binaries.
//!
//! Each file in `tests/` is its own crate, so a mock used by more than one of
//! them has to live here rather than be copied into each.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A minimal HTTP/1.1 server that answers from a closure.
///
/// Hand-rolled rather than pulling in a test web framework: it only has to
/// read a form POST and write a JSON body.
pub struct MockServer {
    base: String,
    requests: Arc<AtomicUsize>,
}

impl MockServer {
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let handler = Arc::new(handler);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let counter = Arc::clone(&counter);
                let handler = Arc::clone(&handler);

                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 2048];

                    // Read until the headers are complete, then until the body
                    // named by Content-Length has arrived.
                    let (path, body) = loop {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);

                        let text = String::from_utf8_lossy(&buffer).to_string();
                        let Some(header_end) = text.find("\r\n\r\n") else {
                            continue;
                        };

                        let head = &text[..header_end];
                        let body = &text[header_end + 4..];
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().ok())?
                            })
                            .unwrap_or(0);

                        if body.len() >= length {
                            let path = head
                                .lines()
                                .next()
                                .and_then(|line| line.split_whitespace().nth(1))
                                .unwrap_or("/")
                                .to_string();
                            break (path, body.to_string());
                        }
                    };

                    counter.fetch_add(1, Ordering::SeqCst);
                    let (status, json) = handler(&path, &body);

                    let response = format!(
                        "HTTP/1.1 {status} X\r\n\
                         Content-Type: application/json\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n{json}",
                        json.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });

        Self {
            base: format!("http://{addr}"),
            requests,
        }
    }

    /// Base URL, e.g. `http://127.0.0.1:54321`.
    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}
