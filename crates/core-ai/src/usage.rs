//! How many tokens a model was sent and wrote back.
//!
//! Each provider says so in its reply — Ollama as `prompt_eval_count` and
//! `eval_count`, OpenAI's API as `usage` — and the count is handed to a
//! [`Meter`] the caller attached, if any. Counting is the caller's business:
//! this crate does not know where a tally is kept, or which job asked.

use std::sync::Arc;

use serde_json::Value;

/// Tokens in, tokens out, for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// What the model was given: the prompt, the conversation, a page.
    pub input_tokens: u64,
    /// What it wrote, thinking included where the service counts it.
    pub output_tokens: u64,
}

/// Something that keeps count. Called once per request that reported usage,
/// after the reply arrived; it must not block for long.
pub trait Meter: Send + Sync {
    fn record(&self, model: &str, usage: Usage);
}

/// Usage in an Ollama reply, from `/api/chat` or `/api/embed`. A prompt
/// Ollama had cached leaves `prompt_eval_count` out, which counts as none.
pub(crate) fn from_ollama(reply: &Value) -> Option<Usage> {
    let input = reply.get("prompt_eval_count").and_then(Value::as_u64);
    let output = reply.get("eval_count").and_then(Value::as_u64);
    (input.is_some() || output.is_some()).then(|| Usage {
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
    })
}

/// Usage in an OpenAI-style reply.
pub(crate) fn from_openai(reply: &Value) -> Option<Usage> {
    let usage = reply.get("usage")?;
    let input = usage.get("prompt_tokens").and_then(Value::as_u64);
    let output = usage.get("completion_tokens").and_then(Value::as_u64);
    (input.is_some() || output.is_some()).then(|| Usage {
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
    })
}

/// Hands a reply's usage to the meter, when there are both.
pub(crate) fn note(meter: &Option<Arc<dyn Meter>>, model: &str, usage: Option<Usage>) {
    if let (Some(meter), Some(usage)) = (meter, usage) {
        meter.record(model, usage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_is_read_the_way_each_provider_reports_it() {
        assert_eq!(
            from_ollama(&json!({"prompt_eval_count": 120, "eval_count": 30})),
            Some(Usage {
                input_tokens: 120,
                output_tokens: 30
            })
        );
        // A cached prompt: only what was written.
        assert_eq!(
            from_ollama(&json!({"eval_count": 7})),
            Some(Usage {
                input_tokens: 0,
                output_tokens: 7
            })
        );
        assert_eq!(from_ollama(&json!({"message": {}})), None);
        assert_eq!(
            from_openai(
                &json!({"usage": {"prompt_tokens": 900, "completion_tokens": 12, "total_tokens": 912}})
            ),
            Some(Usage {
                input_tokens: 900,
                output_tokens: 12
            })
        );
        assert_eq!(from_openai(&json!({"choices": []})), None);
    }
}
