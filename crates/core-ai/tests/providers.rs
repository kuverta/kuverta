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
async fn local_assistant_overrides_small_server_context_and_preserves_tool_history() {
    use core_ai::{ToolSpec, Turn};
    let (base, seen) = serve(|_, _, body| {
        if body["options"]["num_ctx"].as_u64().unwrap_or(4096) < 6705 + 2048 {
            return (
                400,
                json!({"error":"request (6705 tokens) exceeds the available context size (4096 tokens)"}),
            );
        }
        (
            200,
            json!({"message":{"content":"Synthetic summary"},"done_reason":"stop"}),
        )
    });
    let provider = Provider::connect("ollama", &base, None).unwrap();
    let turns = vec![
        Turn::System {
            content: "Keep user instructions".into(),
        },
        Turn::User {
            content: "Review synthetic headers".into(),
        },
        Turn::Tool {
            call_id: "test".into(),
            name: "headers".into(),
            content: "Synthetic headers".repeat(1200),
        },
    ];
    let tools = vec![ToolSpec {
        name: "headers".into(),
        description: "Read synthetic headers".into(),
        parameters: json!({"type":"object"}),
    }];
    assert_eq!(
        provider
            .converse("test-local", &turns, &tools)
            .await
            .unwrap()
            .content,
        "Synthetic summary"
    );
    let request = seen.recv().unwrap();
    assert_eq!(
        request.body["messages"][0]["content"],
        "Keep user instructions"
    );
    assert_eq!(
        request.body["messages"][2]["content"],
        "Synthetic headers".repeat(1200)
    );
    assert_eq!(request.body["tools"][0]["function"]["name"], "headers");
    assert!(request.body["options"]["num_ctx"].as_u64().unwrap() <= 16384);
    assert!(request.body["options"]["num_predict"].as_u64().unwrap() <= 2048);
}

#[tokio::test]
async fn local_context_overflow_is_actionable_and_is_not_retried() {
    let (base, seen) = serve(|_, _, _| {
        (
            400,
            json!({"error":"{\"error\":{\"type\":\"exceed_context_size_error\"}}"}),
        )
    });
    let provider = Provider::connect("ollama", &base, None).unwrap();
    let error = provider.converse("test-local", &[], &[]).await.unwrap_err();
    assert!(matches!(error, AiError::ContextLimit(16384)));
    seen.recv().unwrap();
    assert!(seen.try_recv().is_err());
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
fn a_provider_is_ollama_openai_compatible_or_claude_and_nothing_else() {
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

#[tokio::test]
async fn an_assistant_answer_cut_off_by_the_length_limit_says_so() {
    let (base, seen) = serve(|_, _, _| {
        (
            200,
            json!({ "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "4. Nalina Hussein, Kreis Pinneberg — asks you to" },
                "finish_reason": "length",
            }] }),
        )
    });
    let reply = OpenAiCompatible::new(&base, Some("sk-test".into()))
        .unwrap()
        .converse(
            "deepseek-chat",
            &[core_ai::Turn::User {
                content: "Summarise my unread mail from people.".into(),
            }],
            &[],
        )
        .await
        .unwrap();
    assert!(reply.truncated);
    // Room for a long summary before it comes to that.
    assert!(seen.recv().unwrap().body["max_tokens"].as_u64().unwrap() >= 8_000);
}

/// What a thinking model sends back when its budget ran out mid-thought, and
/// the answer it gives once told not to think.
fn thinks_unless_told(_: &str, _: &str, body: &Value) -> (u16, Value) {
    if body["reasoning_effort"] == "none" {
        return (200, completion(r#"{"urgency": 2}"#));
    }
    (
        200,
        json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "", "reasoning_content": "We need to judge" },
                "finish_reason": "length",
            }],
            "usage": { "completion_tokens": 160, "completion_tokens_details": { "reasoning_tokens": 160 } },
        }),
    )
}

#[tokio::test]
async fn a_model_that_thought_its_budget_away_is_asked_again_without_thinking() {
    let (base, seen) = serve(thinks_unless_told);
    let service = OpenAiCompatible::new(&base, Some("sk-test".into())).unwrap();

    let reply = service
        .chat_up_to("deepseek-flash", "system", "user", 160)
        .await
        .unwrap();
    assert_eq!(reply.content, r#"{"urgency": 2}"#);
    assert!(seen.recv().unwrap().body.get("reasoning_effort").is_none());
    assert_eq!(seen.recv().unwrap().body["reasoning_effort"], "none");

    // And is told straight away the next time.
    service
        .chat_up_to("deepseek-flash", "system", "user", 160)
        .await
        .unwrap();
    assert_eq!(seen.recv().unwrap().body["reasoning_effort"], "none");
    assert!(seen.try_recv().is_err());
}

#[tokio::test]
async fn a_service_that_refuses_to_stop_thinking_keeps_its_first_reply() {
    let (base, seen) = serve(|_, _, body| {
        if body.get("reasoning_effort").is_some() {
            return (
                400,
                json!({ "error": { "message": "Unrecognized request argument" } }),
            );
        }
        thinks_unless_told("", "", body)
    });
    let service = OpenAiCompatible::new(&base, Some("sk-test".into())).unwrap();

    let reply = service
        .chat_up_to("o-mini", "system", "user", 160)
        .await
        .unwrap();
    assert_eq!(reply.content, "");
    assert!(reply.truncated);
    assert_eq!(seen.iter().take(2).count(), 2);
}

#[tokio::test]
async fn a_model_that_answers_is_not_told_anything() {
    let (base, seen) = serve(|_, _, _| (200, completion(r#"{"urgency": 1}"#)));
    let service = OpenAiCompatible::new(&base, Some("sk-test".into())).unwrap();

    service
        .chat_up_to("deepseek-chat", "system", "user", 160)
        .await
        .unwrap();
    assert!(seen.recv().unwrap().body.get("reasoning_effort").is_none());
    assert!(seen.try_recv().is_err());
}

#[tokio::test]
async fn claude_is_asked_with_its_own_headers_and_answers_from_its_text_blocks() {
    let (base, seen) = serve(|_, _, _| {
        (
            200,
            json!({
                "content": [
                    { "type": "thinking", "thinking": "", "signature": "sig" },
                    { "type": "text", "text": "transactional" },
                ],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 120, "output_tokens": 30 },
            }),
        )
    });
    let claude = Provider::connect(
        "anthropic",
        &format!("{base}/v1/"),
        Some("sk-ant-test".into()),
    )
    .unwrap();
    assert!(matches!(claude, Provider::Anthropic(_)));

    let reply = claude
        .chat("claude-haiku-5-5", "system words", "user words")
        .await
        .unwrap();
    assert_eq!(reply.content, "transactional");
    assert!(!reply.truncated);

    let request = seen.recv().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.authorization, None, "the key goes in x-api-key");
    assert_eq!(request.body["system"], "system words");
    assert_eq!(
        request.body["messages"][0],
        json!({ "role": "user", "content": "user words" })
    );
    // Room to think before one word, and no sampling settings Claude refuses.
    assert!(request.body["max_tokens"].as_u64().unwrap() >= 1024);
    assert!(request.body.get("temperature").is_none());
    assert_eq!(request.body["output_config"]["effort"], "low");
}

#[tokio::test]
async fn a_claude_model_that_takes_no_effort_level_is_asked_again_without_one() {
    let (base, seen) = serve(|_, _, body| {
        if body.get("output_config").is_some() {
            (
                400,
                json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "effort is not supported on this model" } }),
            )
        } else {
            (
                200,
                json!({ "content": [{ "type": "text", "text": "ok" }], "stop_reason": "end_turn" }),
            )
        }
    });
    let claude = Provider::connect("anthropic", &base, Some("k".into())).unwrap();
    for _ in 0..2 {
        let reply = claude.chat("claude-haiku-4-5", "s", "u").await.unwrap();
        assert_eq!(reply.content, "ok");
    }
    let tries: Vec<bool> = seen
        .try_iter()
        .map(|request| request.body.get("output_config").is_some())
        .collect();
    assert_eq!(
        tries,
        vec![true, false, false],
        "told once, then remembered"
    );
}

#[tokio::test]
async fn claude_lists_its_models_with_what_they_can_see() {
    let (base, seen) = serve(|_, path, _| {
        let page = if path.contains("after_id") {
            json!({ "data": [
                { "id": "claude-opus-5-5", "capabilities": { "image_input": { "supported": true } } },
            ], "has_more": false, "last_id": "claude-opus-5-5" })
        } else {
            json!({ "data": [
                { "id": "claude-haiku-5-5", "capabilities": { "image_input": { "supported": true } } },
                { "id": "claude-text-only", "capabilities": { "image_input": { "supported": false } } },
            ], "has_more": true, "last_id": "claude-text-only" })
        };
        (200, page)
    });
    let models = Provider::connect("anthropic", &base, Some("k".into()))
        .unwrap()
        .models()
        .await
        .unwrap();
    let names: Vec<_> = models.iter().map(|m| (m.name.as_str(), m.vision)).collect();
    assert_eq!(
        names,
        vec![
            ("claude-haiku-5-5", Some(true)),
            ("claude-opus-5-5", Some(true)),
            ("claude-text-only", Some(false)),
        ]
    );
    let first = seen.recv().unwrap();
    assert_eq!(first.method, "GET");
    assert!(first.path.starts_with("/models"), "{}", first.path);
}

#[tokio::test]
async fn a_page_goes_to_claude_as_an_image_block() {
    let (base, seen) = serve(|_, _, _| {
        (
            200,
            json!({ "content": [{ "type": "text", "text": "Stadtwerke Musterstadt" }], "stop_reason": "max_tokens" }),
        )
    });
    let reply = Provider::connect("anthropic", &base, Some("k".into()))
        .unwrap()
        .transcribe("claude-opus-5-5", b"\xff\xd8a photographed page")
        .await
        .unwrap();
    assert_eq!(reply.content, "Stadtwerke Musterstadt");
    assert!(reply.truncated);
    let body = seen.recv().unwrap().body;
    assert_eq!(body["system"], TRANSCRIBE);
    let image = &body["messages"][0]["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["source"]["media_type"], "image/jpeg");
    assert_eq!(image["source"]["data"], "/9hhIHBob3RvZ3JhcGhlZCBwYWdl");
}
