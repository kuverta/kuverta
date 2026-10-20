//! One explicitly invoked local inference, synthetic text only. No account,
//! database, credentials or mail access; returned tool calls are NOT executed.
//! cargo run -p core-rpc --release --example local_assistant_smoke
use core_ai::{Provider, Turn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Provider::connect("ollama", "http://127.0.0.1:11434", None)?;
    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "qwen3:4b-instruct-2507-q4_K_M".into());
    let text = "Synthetic sample: Example GmbH received invoice TEST-123 from Testvendor GmbH. The invoice date is 2026-09-15 and the total is 10 EUR. This is test data, not a real invoice.\n".repeat(65);
    let reply = provider.converse(&model, &[
        Turn::System { content: "This is a synthetic context capacity test. Do not call tools. Do not inspect mail. Answer the user's question in one short sentence.".into() },
        Turn::User { content: format!("{text}\nAll examples above are synthetic. Confirm that fact in German, in one short sentence. Do not call any tools.") },
    ], &core_rpc::assistant::tools()).await?;
    if !reply.calls.is_empty() || reply.truncated || reply.content.trim().is_empty() {
        return Err("Expected one complete answer without tools".into());
    }
    println!(
        "Synthetic local assistant succeeded in {} ms: {}",
        reply.latency_ms, reply.content
    );
    Ok(())
}
