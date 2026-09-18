//! A conversation with a model that can use tools.
//!
//! The assistant in the window is a model given a handful of tools — search
//! mail, read a message, move it, draft a reply — and a loop: it answers, or
//! asks for a tool; the tool runs; its result goes back; again, until it has
//! an answer. This is the model half of that: the turns of a conversation,
//! the tools described to the model, and one exchange with either kind of
//! provider. The loop and the tools themselves are the caller's.
//!
//! Ollama and OpenAI's API both take tools as JSON Schema functions and give
//! calls back on the assistant's message; they differ in detail — Ollama hands
//! arguments over as an object and matches results by position, OpenAI as a
//! JSON string and by call id — and that difference stays in here.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ollama::{AiError, Ollama};
use crate::provider::{OpenAiCompatible, Provider};

/// A tool as the model is told about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments, an object.
    pub parameters: Value,
}

/// One call the model asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's id for the call, or one made up where it gives none.
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One turn of a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Turn {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
        calls: Vec<ToolCall>,
    },
    Tool {
        call_id: String,
        name: String,
        content: String,
    },
}

/// What the model said back.
#[derive(Debug, Clone)]
pub struct AssistantReply {
    pub content: String,
    pub calls: Vec<ToolCall>,
    pub latency_ms: i64,
    /// The answer hit the length limit and stops mid-sentence.
    pub truncated: bool,
}

/// How long one answer may be: a summary of a busy inbox runs to pages. A
/// longer one is continued rather than cut (see `truncated`).
const MAX_TOKENS: u32 = 8_000;

impl Provider {
    /// One exchange: the conversation so far and the tools, in; an answer or
    /// calls, out.
    pub async fn converse(
        &self,
        model: &str,
        turns: &[Turn],
        tools: &[ToolSpec],
    ) -> Result<AssistantReply, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.converse(model, turns, tools).await,
            Self::OpenAi(service) => service.converse(model, turns, tools).await,
        }
    }
}

fn function(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

impl Ollama {
    pub async fn converse(
        &self,
        model: &str,
        turns: &[Turn],
        tools: &[ToolSpec],
    ) -> Result<AssistantReply, AiError> {
        let messages: Vec<Value> = turns
            .iter()
            .map(|turn| match turn {
                Turn::System { content } => json!({ "role": "system", "content": content }),
                Turn::User { content } => json!({ "role": "user", "content": content }),
                Turn::Assistant { content, calls } => json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": calls.iter().map(|call| json!({
                        "function": { "name": call.name, "arguments": call.arguments }
                    })).collect::<Vec<_>>(),
                }),
                Turn::Tool { name, content, .. } => {
                    json!({ "role": "tool", "content": content, "tool_name": name })
                }
            })
            .collect();
        let started = Instant::now();
        let reply = self
            .post(
                "/api/chat",
                &json!({
                    "model": model,
                    "stream": false,
                    "messages": messages,
                    "tools": tools.iter().map(function).collect::<Vec<_>>(),
                    "options": { "temperature": 0.2, "num_predict": MAX_TOKENS },
                }),
            )
            .await?;
        let message = reply
            .get("message")
            .ok_or_else(|| AiError::Shape("no message in the reply".into()))?;
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .filter_map(|(at, call)| {
                        let function = call.get("function")?;
                        Some(ToolCall {
                            id: format!("call-{at}"),
                            name: function.get("name")?.as_str()?.to_string(),
                            arguments: arguments(function.get("arguments")),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(AssistantReply {
            content: message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            calls,
            latency_ms: started.elapsed().as_millis() as i64,
            truncated: reply.get("done_reason").and_then(Value::as_str) == Some("length"),
        })
    }
}

impl OpenAiCompatible {
    pub async fn converse(
        &self,
        model: &str,
        turns: &[Turn],
        tools: &[ToolSpec],
    ) -> Result<AssistantReply, AiError> {
        let messages: Vec<Value> = turns
            .iter()
            .map(|turn| match turn {
                Turn::System { content } => json!({ "role": "system", "content": content }),
                Turn::User { content } => json!({ "role": "user", "content": content }),
                Turn::Assistant { content, calls } if calls.is_empty() => {
                    json!({ "role": "assistant", "content": content })
                }
                Turn::Assistant { content, calls } => json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": calls.iter().map(|call| json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }
                    })).collect::<Vec<_>>(),
                }),
                Turn::Tool {
                    call_id, content, ..
                } => json!({ "role": "tool", "tool_call_id": call_id, "content": content }),
            })
            .collect();
        let mut body = json!({
            "model": model,
            "messages": messages,
            "temperature": 0.2,
            "max_tokens": MAX_TOKENS,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.iter().map(function).collect());
        }
        let started = Instant::now();
        let reply = self
            .send(
                self.http
                    .post(format!("{}/chat/completions", self.base))
                    .json(&body),
            )
            .await?;
        let message = reply
            .pointer("/choices/0/message")
            .ok_or_else(|| AiError::Shape("no choices[0].message in the reply".into()))?;
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .filter_map(|(at, call)| {
                        let function = call.get("function")?;
                        Some(ToolCall {
                            id: call
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("call-{at}")),
                            name: function.get("name")?.as_str()?.to_string(),
                            arguments: arguments(function.get("arguments")),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(AssistantReply {
            content: message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            calls,
            latency_ms: started.elapsed().as_millis() as i64,
            truncated: reply
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                == Some("length"),
        })
    }
}

/// Arguments as an object, whichever way they came: an object (Ollama), a
/// JSON string (OpenAI), or nothing.
fn arguments(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({ "_unparsed": text }))
        }
        Some(Value::Null) | None => json!({}),
        Some(other) => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_come_as_an_object_whichever_way_they_are_sent() {
        assert_eq!(arguments(Some(&json!({"a": 1}))), json!({"a": 1}));
        assert_eq!(arguments(Some(&json!("{\"a\": 1}"))), json!({"a": 1}));
        assert_eq!(arguments(None), json!({}));
        assert_eq!(arguments(Some(&json!("not json")))["_unparsed"], "not json");
    }
}
