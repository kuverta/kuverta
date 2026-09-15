//! Asking a vision model to read a page, without a model.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use core_ai::{Ollama, TRANSCRIBE};

/// Answers one chat request with `reply` as the model's text and hands back
/// the request body.
fn serve(reply: &'static str) -> (String, mpsc::Receiver<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).ok();
            if let Ok(json) = serde_json::from_slice(&body) {
                let _ = tx.send(json);
            }
            let answer = serde_json::json!({ "message": { "role": "assistant", "content": reply } }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, rx)
}

#[tokio::test]
async fn a_page_goes_to_the_model_as_an_image_and_its_text_comes_back() {
    let (base, requests) = serve("Stadtwerke Musterstadt GmbH\nRechnung Nr. 2026-48211");
    let jpeg = b"\xff\xd8a photographed page";

    let reply = Ollama::new(&base)
        .unwrap()
        .transcribe("qwen2.5vl:3b", jpeg)
        .await
        .unwrap();
    assert_eq!(reply.content, "Stadtwerke Musterstadt GmbH\nRechnung Nr. 2026-48211");

    let body = requests.recv().unwrap();
    assert_eq!(body["model"], "qwen2.5vl:3b");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["content"], TRANSCRIBE);
    // Standard base64 of the bytes, as Ollama takes images.
    assert_eq!(body["messages"][1]["images"][0], "/9hhIHBob3RvZ3JhcGhlZCBwYWdl");
    // Deterministic, and room for a whole page rather than the one word a
    // classification gets.
    assert_eq!(body["options"]["temperature"], 0);
    assert!(body["options"]["num_predict"].as_u64().unwrap() >= 2048);
}

#[test]
fn the_model_is_told_not_to_translate_or_guess() {
    let lowered = TRANSCRIBE.to_lowercase();
    assert!(lowered.contains("original language"));
    assert!(lowered.contains("do not translate"));
    assert!(lowered.contains("[?]"));
}
