//! Choosing where models run, and trying one.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use core_rpc::ai::{
    is_loopback, read_page, try_model, LOCAL_PROVIDER, READ_AGAIN_NOTE, STOPPED_NOTE,
};
use core_rpc::{AiProviderInput, Core, RpcError, Task};

fn core(name: &str) -> (Core, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fuckmail-rpc-ai-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (Core::open(dir.as_path()).unwrap(), dir)
}

fn provider(kind: &str, label: &str, base_url: &str) -> AiProviderInput {
    AiProviderInput {
        id: None,
        kind: kind.into(),
        label: label.into(),
        base_url: base_url.into(),
    }
}

/// An Ollama whose every chat is answered with `reply`.
fn ollama_saying(reply: &'static str) -> String {
    ollama_reading((reply, "stop"), (reply, "stop")).0
}

/// An Ollama that answers a plain reading with `plain` and one penalising
/// repetition with `guarded`, each as (text, why it stopped), and says which
/// kind each request was: `true` for a penalised one.
fn ollama_reading(
    plain: (&'static str, &'static str),
    guarded: (&'static str, &'static str),
) -> (String, mpsc::Receiver<bool>) {
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
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let penalised = request["options"].get("repeat_penalty").is_some();
            let _ = tx.send(penalised);
            let (reply, why) = if penalised { guarded } else { plain };
            let answer = serde_json::json!({
                "message": { "role": "assistant", "content": reply },
                "done": true,
                "done_reason": why,
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    (base, rx)
}

/// What came back for a tax office statement: the letterhead, then two
/// column headings over and over.
fn looping() -> &'static str {
    let mut text = String::from("Finanzamt Beispielstadt\n");
    for _ in 0..100 {
        text.push_str("EUR\nCt\n");
    }
    Box::leak(text.into_boxed_str())
}

#[tokio::test]
async fn a_page_read_cleanly_is_read_once() {
    let (base, readings) = ollama_reading(("Rechnung 4711", "stop"), ("Rechnung 1234", "stop"));
    let provider = core_ai::Provider::connect("ollama", &base, None).unwrap();

    let text = read_page(&provider, "qwen2.5vl:3b", b"\xff\xd8page")
        .await
        .unwrap();
    assert_eq!(text, "Rechnung 4711");
    assert_eq!(readings.try_iter().collect::<Vec<_>>(), [false]);
}

#[tokio::test]
async fn a_page_the_model_looped_on_is_read_again_and_says_how() {
    let second = "Finanzamt Beispielstadt\nSchuldbetrag | EUR | Ct\nSumme | 1.944 | 93";
    let (base, readings) = ollama_reading((looping(), "length"), (second, "stop"));
    let provider = core_ai::Provider::connect("ollama", &base, None).unwrap();

    let text = read_page(&provider, "qwen2.5vl:3b", b"\xff\xd8page")
        .await
        .unwrap();
    assert_eq!(text, format!("{second}\n{READ_AGAIN_NOTE}"));
    assert_eq!(readings.try_iter().collect::<Vec<_>>(), [false, true]);
}

#[tokio::test]
async fn when_the_second_reading_loops_too_the_first_is_kept_with_its_loop_cut() {
    let (base, readings) = ollama_reading((looping(), "length"), (looping(), "length"));
    let provider = core_ai::Provider::connect("ollama", &base, None).unwrap();

    let text = read_page(&provider, "qwen2.5vl:3b", b"\xff\xd8page")
        .await
        .unwrap();
    assert_eq!(
        text,
        format!(
            "Finanzamt Beispielstadt\nEUR\nCt\nEUR\nCt\n{}",
            core_ai::LOOP_MARK
        )
    );
    assert_eq!(readings.try_iter().collect::<Vec<_>>(), [false, true]);
}

#[tokio::test]
async fn a_page_cut_short_without_a_loop_says_it_stopped() {
    let (base, _readings) = ollama_reading(
        (
            "Sehr geehrte Damen und Herren,\nanbei erhalten Sie",
            "length",
        ),
        ("Sehr geehrte", "length"),
    );
    let provider = core_ai::Provider::connect("ollama", &base, None).unwrap();

    let text = read_page(&provider, "qwen2.5vl:3b", b"\xff\xd8page")
        .await
        .unwrap();
    assert_eq!(
        text,
        format!("Sehr geehrte Damen und Herren,\nanbei erhalten Sie\n{STOPPED_NOTE}")
    );
}

#[test]
fn every_job_runs_on_this_computer_until_someone_chooses_otherwise() {
    let (core, dir) = core("defaults");

    let providers = core.ai_providers().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, LOCAL_PROVIDER);
    assert_eq!(providers[0].kind, "ollama");
    assert!(providers[0].local);
    assert!(!providers[0].has_key);

    let tasks = core.ai_tasks().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks
        .iter()
        .all(|task| task.provider_id == LOCAL_PROVIDER && !task.chosen));
    let chat = tasks.iter().find(|task| task.task == Task::Chat).unwrap();
    assert_eq!(chat.model, "llama3.2:3b");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn what_is_not_a_provider_is_refused_before_it_is_stored() {
    let (core, dir) = core("refused");

    for input in [
        provider("carrier-pigeon", "Pigeons", "https://example.com"),
        provider("openai", "DeepSeek", "api.deepseek.com"),
        provider("openai", "   ", "https://api.deepseek.com"),
        AiProviderInput {
            id: Some(999),
            ..provider("openai", "DeepSeek", "https://api.deepseek.com")
        },
    ] {
        let err = core.save_ai_provider(&input).err().unwrap();
        assert!(matches!(err, RpcError::Rejected(_)), "{err}");
    }
    assert_eq!(core.ai_providers().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_job_goes_to_a_hosted_service_only_when_chosen_and_back_when_it_is_removed() {
    let (core, dir) = core("jobs");
    let deepseek = core
        .save_ai_provider(&provider("openai", "DeepSeek", "https://api.deepseek.com/"))
        .unwrap();
    let shown = core.ai_providers().unwrap();
    let hosted = shown.iter().find(|p| p.id == deepseek).unwrap();
    assert!(!hosted.local, "a hosted service is not this computer");
    assert_eq!(hosted.base_url, "https://api.deepseek.com");

    assert!(core.set_ai_task(Task::Chat, 999, "deepseek-chat").is_err());
    assert!(core.set_ai_task(Task::Chat, deepseek, "  ").is_err());
    core.set_ai_task(Task::Chat, deepseek, "deepseek-chat")
        .unwrap();
    let chat = core
        .ai_tasks()
        .unwrap()
        .into_iter()
        .find(|task| task.task == Task::Chat)
        .unwrap();
    assert_eq!(
        (chat.provider_id, chat.model.as_str(), chat.chosen),
        (deepseek, "deepseek-chat", true)
    );

    assert!(
        core.delete_ai_provider(LOCAL_PROVIDER).is_err(),
        "the local Ollama stays"
    );
    core.delete_ai_provider(deepseek).unwrap();
    let chat = core
        .ai_tasks()
        .unwrap()
        .into_iter()
        .find(|task| task.task == Task::Chat)
        .unwrap();
    assert_eq!((chat.provider_id, chat.chosen), (LOCAL_PROVIDER, false));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_job_is_given_the_chosen_model_on_its_provider() {
    let (core, dir) = core("choice");
    let upstairs = core
        .save_ai_provider(&provider(
            "ollama",
            "Ollama upstairs",
            "http://192.168.8.10:11434",
        ))
        .unwrap();
    core.set_ai_task(Task::Vision, upstairs, "qwen2.5vl:7b")
        .unwrap();

    let choice = core.ai_for(Task::Vision).unwrap();
    assert_eq!(choice.model, "qwen2.5vl:7b");
    assert_eq!(choice.provider_label, "Ollama upstairs");
    assert_eq!(choice.provider.base_url(), "http://192.168.8.10:11434");
    assert!(
        !choice.local,
        "another machine on the network is not this computer"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_an_address_that_is_this_computer_counts_as_local() {
    for local in [
        "http://127.0.0.1:11434",
        "http://localhost:11434/",
        "http://[::1]:11434",
        "http://127.0.0.2",
    ] {
        assert!(is_loopback(local), "{local}");
    }
    for elsewhere in [
        "https://api.deepseek.com",
        "http://192.168.8.10:11434",
        "http://127.example.com",
        "http://localhost.example.com",
    ] {
        assert!(!is_loopback(elsewhere), "{elsewhere}");
    }
}

#[tokio::test]
async fn trying_a_vision_model_checks_it_read_the_sample_page() {
    let reads =
        core_ai::Provider::connect("ollama", &ollama_saying("Rechnung 4711"), None).unwrap();
    let trial = try_model(&reads, Task::Vision, "qwen2.5vl:3b")
        .await
        .unwrap();
    assert!(trial.passed, "{}", trial.verdict);
    assert_eq!(trial.reply, "Rechnung 4711");

    let guesses =
        core_ai::Provider::connect("ollama", &ollama_saying("Rechnung 1234"), None).unwrap();
    let trial = try_model(&guesses, Task::Vision, "llama3.2:3b")
        .await
        .unwrap();
    assert!(!trial.passed);
    assert!(trial.verdict.contains("4711"), "{}", trial.verdict);
}

#[tokio::test]
async fn a_server_that_is_not_there_is_reported_as_such() {
    // Bound and dropped: nothing listens there any more.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let gone =
        core_ai::Provider::connect("ollama", &format!("http://127.0.0.1:{port}"), None).unwrap();
    let err = try_model(&gone, Task::Chat, "llama3.2:3b")
        .await
        .err()
        .unwrap();
    assert!(matches!(err, RpcError::Network(_)), "{err}");
    assert!(err.to_string().contains("is Ollama running?"), "{err}");
}
