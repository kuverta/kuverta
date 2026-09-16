//! Listing models and talking to them, over both kinds of provider, against
//! servers that answer the way the real ones do.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use core_ai::{AiError, ModelInfo, OpenAiCompatible, Provider, TRANSCRIBE};
use serde_json::{json, Value};

/// One request, as the server saw it.
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Value,
}

/// Answers every request with `answer(method, path, body)` as status and JSON.
fn serve(answer: fn(&str, &str, &Value) -> (u16, Value)) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).ok();
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();

            let mut length = 0usize;
            let mut authorization = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    authorization = Some(line["authorization:".len()..].trim().to_string());
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).ok();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

            let (status, reply) = answer(&method, &path, &body);
            let _ = tx.send(Seen {
                method,
                path,
                authorization,
                body,
            });
            let text = reply.to_string();
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, rx)
}

fn completion(content: &str) -> Value {
    json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": content } }] })
}

#[tokio::test]
async fn a_hosted_model_is_asked_with_the_key_and_answers_from_its_choices() {
    let (base, seen) = serve(|_, _, _| (200, completion("transactional")));
    let service = OpenAiCompatible::new(&format!("{base}/v1/"), Some("sk-test".into())).unwrap();

    let reply = service
        .chat("deepseek-chat", "system words", "user words")
        .await
        .unwrap();
    assert_eq!(reply.content, "transactional");

    let request = seen.recv().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(request.body["model"], "deepseek-chat");
    assert_eq!(
        request.body["messages"][0],
        json!({ "role": "system", "content": "system words" })
    );
    assert_eq!(
        request.body["messages"][1],
        json!({ "role": "user", "content": "user words" })
    );
    assert_eq!(request.body["temperature"], 0);
}

#[tokio::test]
async fn a_page_goes_to_a_hosted_model_as_a_data_url() {
    let (base, seen) = serve(|_, _, _| (200, completion("Stadtwerke Musterstadt")));
    let service = OpenAiCompatible::new(&base, Some("sk-test".into())).unwrap();

    let reply = service
        .transcribe("gpt-4o-mini", b"\xff\xd8a photographed page")
        .await
        .unwrap();
    assert_eq!(reply.content, "Stadtwerke Musterstadt");

    let body = seen.recv().unwrap().body;
    assert_eq!(body["messages"][0]["content"], TRANSCRIBE);
    let parts = &body["messages"][1]["content"];
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(
        parts[1]["image_url"]["url"],
        "data:image/jpeg;base64,/9hhIHBob3RvZ3JhcGhlZCBwYWdl"
    );
    assert_eq!(body["temperature"], 0);
    assert!(body["max_tokens"].as_u64().unwrap() >= 2048);
}

#[tokio::test]
async fn a_hosted_model_that_ran_out_of_room_says_so() {
    let (base, _seen) = serve(|_, _, _| {
        (
            200,
            json!({ "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "EUR Ct EUR" },
                "finish_reason": "length",
            }] }),
        )
    });
    let reply = OpenAiCompatible::new(&base, Some("sk-test".into()))
        .unwrap()
        .transcribe("some-vl-model", b"\xff\xd8a photographed page")
        .await
        .unwrap();
    assert!(reply.truncated);
}

#[tokio::test]
async fn a_blank_key_is_no_key() {
    let (base, seen) = serve(|_, _, _| (200, completion("ok")));
    OpenAiCompatible::new(&base, Some("   ".into()))
        .unwrap()
        .chat("local-model", "s", "u")
        .await
        .unwrap();
    assert_eq!(seen.recv().unwrap().authorization, None);
}

#[tokio::test]
async fn hosted_models_are_listed_with_what_the_service_says_they_can_see() {
    let (base, seen) = serve(|_, _, _| {
        (
            200,
            json!({ "object": "list", "data": [
                { "id": "text-embedding-3-small", "architecture": { "input_modalities": ["text"] } },
                { "id": "deepseek-chat", "object": "model", "owned_by": "deepseek" },
                { "id": "openai/gpt-4o", "architecture": { "input_modalities": ["text", "image"] } },
            ]}),
        )
    });
    let models = OpenAiCompatible::new(&base, Some("sk-test".into()))
        .unwrap()
        .models()
        .await
        .unwrap();

    assert_eq!(
        models,
        vec![
            ModelInfo {
                name: "deepseek-chat".into(),
                size_bytes: None,
                vision: None,
                embedding: false,
            },
            ModelInfo {
                name: "openai/gpt-4o".into(),
                size_bytes: None,
                vision: Some(true),
                embedding: false,
            },
            ModelInfo {
                name: "text-embedding-3-small".into(),
                size_bytes: None,
                vision: Some(false),
                embedding: true,
            },
        ]
    );
    let request = seen.recv().unwrap();
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("GET", "/models")
    );
    assert_eq!(request.authorization.as_deref(), Some("Bearer sk-test"));
}

#[tokio::test]
async fn a_refused_key_is_reported_with_what_the_service_said() {
    let (base, _seen) = serve(|_, _, _| {
        (
            401,
            json!({ "error": { "message": "Authentication Fails, Your api key is invalid" } }),
        )
    });
    let err = OpenAiCompatible::new(&base, Some("wrong".into()))
        .unwrap()
        .models()
        .await
        .err()
        .unwrap();
    match err {
        AiError::Status { status, body } => {
            assert_eq!(status, 401);
            assert!(body.contains("api key is invalid"), "{body}");
        }
        other => panic!("expected a status error, got {other}"),
    }
}

#[tokio::test]
async fn ollama_lists_its_models_with_what_each_can_do() {
    let (base, _seen) = serve(|method, path, body| match (method, path) {
        ("GET", "/api/tags") => (
            200,
            json!({ "models": [
                { "name": "qwen2.5vl:3b", "size": 3200000000u64 },
                { "name": "nomic-embed-text:latest", "size": 274000000 },
                { "name": "llama3.2:3b", "size": 2000000000u64 },
            ]}),
        ),
        ("POST", "/api/show") => match body["model"].as_str() {
            Some("qwen2.5vl:3b") => (200, json!({ "capabilities": ["completion", "vision"] })),
            Some("nomic-embed-text:latest") => (200, json!({ "capabilities": ["embedding"] })),
            _ => (500, json!({ "error": "something went wrong" })),
        },
        _ => (404, json!({})),
    });

    let models = Provider::connect("ollama", &base, None)
        .unwrap()
        .models()
        .await
        .unwrap();

    let names: Vec<&str> = models.iter().map(|model| model.name.as_str()).collect();
    assert_eq!(
        names,
        ["llama3.2:3b", "nomic-embed-text:latest", "qwen2.5vl:3b"]
    );
    assert_eq!(models[0].vision, None, "show failed: unknown, not no");
    assert!(!models[0].embedding);
    assert!(models[1].embedding);
    assert_eq!(models[1].vision, Some(false));
    assert_eq!(models[2].vision, Some(true));
    assert_eq!(models[2].size_bytes, Some(3_200_000_000));
}

#[test]
fn a_provider_is_ollama_or_openai_compatible_and_nothing_else() {
    assert!(matches!(
        Provider::connect("ollama", "http://127.0.0.1:11434", None),
        Ok(Provider::Ollama(_))
    ));
    assert!(matches!(
        Provider::connect("openai", "https://api.deepseek.com", Some("key".into())),
        Ok(Provider::OpenAi(_))
    ));
    assert!(matches!(
        Provider::connect("carrier-pigeon", "https://example.com", None),
        Err(AiError::Kind(_))
    ));
    assert!(matches!(
        Provider::connect("openai", "api.deepseek.com", None),
        Err(AiError::Url(_))
    ));
}
