//! Claude, over Anthropic's own Messages API.
//!
//! Anthropic does not speak OpenAI's API natively, and the differences matter
//! enough to talk to it directly: the key goes in `x-api-key` with an
//! `anthropic-version`, the system prompt is a field of its own, tool calls
//! and their results are content blocks matched by id, and a page is an
//! `image` block rather than a data URL. Its model list says what each model
//! can do, so whether one sees images is known rather than guessed.
//!
//! Current Claude models think before they answer, adaptively, and some
//! cannot be told not to; the thinking counts against `max_tokens`. So a
//! short judgement is given room to think and asked at low effort, and a
//! budget sized for one word of output is never the whole budget.
//!
//! Thinking blocks are never sent back. Claude checks that a thinking block's
//! conversation is unchanged since it was written, and the assistant does
//! change its history — mail withheld after a refusal, a cut-off answer
//! joined to its continuation — so each turn goes back as its text and its
//! tool calls only, which every Claude model accepts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::agent::{AssistantReply, ToolCall, ToolSpec, Turn};
use crate::ollama::{AiError, ChatReply, TRANSCRIBE};
use crate::provider::ModelInfo;
use crate::usage::{self, Meter};

/// The API version this client is written against.
const VERSION: &str = "2023-06-01";

/// The least a request is allowed, thinking included. Billed by what is
/// used, not by what is allowed, so room to think costs nothing unused.
const MIN_TOKENS: u32 = 2_048;

/// How long one assistant answer may be, as for the other providers, plus
/// room to think.
const CONVERSE_TOKENS: u32 = 16_000;

/// Claude, at Anthropic or at an address that speaks its API.
pub struct Anthropic {
    base: String,
    key: Option<String>,
    http: reqwest::Client,
    /// Models that refused to be told an effort level — older ones do — and
    /// are not told from then on.
    effortless: Mutex<HashSet<String>>,
    pub(crate) meter: Option<Arc<dyn Meter>>,
}

impl Anthropic {
    /// `base_url` is where `/models` and `/messages` are, e.g.
    /// `https://api.anthropic.com/v1`.
    pub fn new(base_url: &str, key: Option<String>) -> Result<Self, AiError> {
        let base = base_url.trim().trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AiError::Url(base_url.to_string()));
        }
        let http = reqwest::Client::builder()
            // A page, or a long answer with thinking first, can take minutes.
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|err| AiError::Network(err.to_string()))?;
        Ok(Self {
            base,
            key: key.filter(|key| !key.trim().is_empty()),
            http,
            effortless: Mutex::new(HashSet::new()),
            meter: None,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// The models the key can use, every page of them.
    pub async fn models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let mut models = Vec::new();
        let mut after: Option<String> = None;
        loop {
            // Model ids are letters, digits, dots and dashes: nothing to escape.
            let url = match &after {
                Some(id) => format!("{}/models?limit=1000&after_id={id}", self.base),
                None => format!("{}/models?limit=1000", self.base),
            };
            let reply = self.send(self.http.get(url)).await?;
            let data = reply
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::Shape("no data list in the reply".into()))?;
            models.extend(data.iter().filter_map(|model| {
                Some(ModelInfo {
                    name: model.get("id")?.as_str()?.to_string(),
                    size_bytes: None,
                    vision: model
                        .pointer("/capabilities/image_input/supported")
                        .and_then(Value::as_bool),
                    embedding: false,
                })
            }));
            let more = reply.get("has_more").and_then(Value::as_bool) == Some(true);
            let last = reply
                .get("last_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            match last {
                Some(last) if more && after.as_deref() != Some(last.as_str()) => after = Some(last),
                _ => break,
            }
        }
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }

    /// One exchange, short, as [`crate::Ollama::chat`].
    pub async fn chat(&self, model: &str, system: &str, user: &str) -> Result<ChatReply, AiError> {
        self.chat_up_to(model, system, user, 12).await
    }

    /// As [`crate::Ollama::chat_up_to`]: `max_tokens` is what the answer
    /// needs, and thinking is given room on top. A judgement this short is
    /// asked at low effort.
    pub async fn chat_up_to(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ChatReply, AiError> {
        let body = json!({
            "model": model,
            "max_tokens": max_tokens.max(MIN_TOKENS),
            "system": system,
            "messages": [{ "role": "user", "content": user }],
            "output_config": { "effort": "low" },
        });
        let reply = self.messages(model, body).await?;
        Ok(reply.into())
    }

    /// A page's text, as [`crate::Ollama::transcribe`].
    pub async fn transcribe(&self, model: &str, jpeg: &[u8]) -> Result<ChatReply, AiError> {
        use base64::Engine as _;
        let body = json!({
            "model": model,
            "max_tokens": 8_192,
            "system": TRANSCRIBE,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/jpeg",
                            "data": base64::engine::general_purpose::STANDARD.encode(jpeg),
                        },
                    },
                    { "type": "text", "text": "Transcribe this page." },
                ],
            }],
        });
        Ok(self.messages(model, body).await?.into())
    }

    /// One exchange of a conversation with tools, as the other providers'.
    pub async fn converse(
        &self,
        model: &str,
        turns: &[Turn],
        tools: &[ToolSpec],
    ) -> Result<AssistantReply, AiError> {
        let (system, messages) = conversation(turns, !tools.is_empty());
        let mut body = json!({
            "model": model,
            "max_tokens": CONVERSE_TOKENS,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.parameters,
                        })
                    })
                    .collect(),
            );
        }
        let reply = self.messages(model, body).await?;
        Ok(AssistantReply {
            content: reply.text,
            calls: reply.calls,
            latency_ms: reply.latency_ms,
            truncated: reply.truncated,
        })
    }

    /// Sends one `/messages` request, without the effort level if this model
    /// turns it away, and reads the reply.
    async fn messages(&self, model: &str, mut body: Value) -> Result<Answer, AiError> {
        if self.effortless.lock().unwrap().contains(model) {
            strip_effort(&mut body);
        }
        let started = Instant::now();
        let reply = match self.post_messages(&body).await {
            Err(AiError::Status {
                status: 400,
                body: why,
            }) if why.contains("effort") && body.get("output_config").is_some() => {
                self.effortless.lock().unwrap().insert(model.to_string());
                strip_effort(&mut body);
                self.post_messages(&body).await?
            }
            other => other?,
        };
        usage::note(&self.meter, model, usage::from_anthropic(&reply));
        answer(&reply, started.elapsed().as_millis() as i64)
    }

    async fn post_messages(&self, body: &Value) -> Result<Value, AiError> {
        self.send(self.http.post(format!("{}/messages", self.base)).json(body))
            .await
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, AiError> {
        let request = request.header("anthropic-version", VERSION);
        let request = match &self.key {
            Some(key) => request.header("x-api-key", key),
            None => request,
        };
        let response = request
            .send()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?;
        if !status.is_success() {
            return Err(AiError::Status {
                status: status.as_u16(),
                body: text.trim().chars().take(500).collect(),
            });
        }
        serde_json::from_str(&text).map_err(|err| AiError::Shape(err.to_string()))
    }
}

fn strip_effort(body: &mut Value) {
    if let Some(config) = body.as_object_mut() {
        config.remove("output_config");
    }
}

/// A reply, read.
struct Answer {
    text: String,
    calls: Vec<ToolCall>,
    latency_ms: i64,
    truncated: bool,
}

impl From<Answer> for ChatReply {
    fn from(answer: Answer) -> Self {
        ChatReply {
            content: answer.text,
            latency_ms: answer.latency_ms,
            truncated: answer.truncated,
        }
    }
}

/// The text and tool calls of a reply. Thinking blocks are left out: nothing
/// here shows them, and they are not sent back (see the module note).
fn answer(reply: &Value, latency_ms: i64) -> Result<Answer, AiError> {
    let stop = reply.get("stop_reason").and_then(Value::as_str);
    if stop == Some("refusal") {
        let category = reply
            .pointer("/stop_details/category")
            .and_then(Value::as_str)
            .unwrap_or("unspecified");
        return Err(AiError::Refused(category.to_string()));
    }
    let blocks = reply
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::Shape("no content in the reply".into()))?;
    let mut text = String::new();
    let mut calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            Some("tool_use") => {
                let (Some(id), Some(name)) = (
                    block.get("id").and_then(Value::as_str),
                    block.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                calls.push(ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                });
            }
            _ => {}
        }
    }
    Ok(Answer {
        text,
        calls,
        latency_ms,
        truncated: stop == Some("max_tokens"),
    })
}

/// The turns as Claude takes them: system turns joined into the one system
/// prompt, the rest as alternating messages of content blocks.
///
/// With no tools on offer — an answer's last round, a continuation — tool
/// calls and their results go as plain text, so a request never carries tool
/// blocks without the tools they name.
fn conversation(turns: &[Turn], with_tools: bool) -> (String, Vec<Value>) {
    let mut system = Vec::new();
    let mut messages: Vec<(&str, Vec<Value>)> = Vec::new();
    let mut push = |role: &'static str, block: Value| match messages.last_mut() {
        Some((last, blocks)) if *last == role => blocks.push(block),
        _ => messages.push((role, vec![block])),
    };
    for turn in turns {
        match turn {
            Turn::System { content } => system.push(content.as_str()),
            Turn::User { content } => push("user", json!({ "type": "text", "text": content })),
            Turn::Assistant { content, calls } => {
                if !content.trim().is_empty() {
                    push("assistant", json!({ "type": "text", "text": content }));
                }
                for call in calls {
                    push(
                        "assistant",
                        if with_tools {
                            json!({
                                "type": "tool_use",
                                "id": tool_id(&call.id),
                                "name": call.name,
                                "input": call.arguments,
                            })
                        } else {
                            json!({
                                "type": "text",
                                "text": format!("[used {} with {}]", call.name, call.arguments),
                            })
                        },
                    );
                }
            }
            Turn::Tool {
                call_id,
                name,
                content,
            } => push(
                "user",
                if with_tools {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": tool_id(call_id),
                        "content": content,
                    })
                } else {
                    json!({ "type": "text", "text": format!("[{name} returned]\n{content}") })
                },
            ),
        }
    }
    let messages = messages
        .into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect();
    (system.join("\n\n"), messages)
}

/// A call id Claude accepts: its own pass through, one another provider made
/// up — `call-0` — is kept to the characters Claude allows.
fn tool_id(id: &str) -> String {
    let id: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if id.is_empty() {
        "call".into()
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "search_mail".into(),
            arguments: json!({ "query": "eSIM" }),
        }
    }

    #[test]
    fn a_conversation_alternates_and_keeps_calls_with_their_results() {
        let turns = vec![
            Turn::System {
                content: "Be brief.".into(),
            },
            Turn::User {
                content: "Find the eSIM".into(),
            },
            Turn::Assistant {
                content: String::new(),
                calls: vec![call("toolu_1"), call("toolu_2")],
            },
            Turn::Tool {
                call_id: "toolu_1".into(),
                name: "search_mail".into(),
                content: "one".into(),
            },
            Turn::Tool {
                call_id: "toolu_2".into(),
                name: "search_mail".into(),
                content: "two".into(),
            },
            Turn::User {
                content: "and?".into(),
            },
        ];
        let (system, messages) = conversation(&turns, true);
        assert_eq!(system, "Be brief.");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(
            messages[1]["content"][1]["input"],
            json!({ "query": "eSIM" })
        );
        // Both results, and the next question, in one user message.
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(messages[2]["content"][1]["tool_use_id"], "toolu_2");
        assert_eq!(messages[2]["content"][2]["text"], "and?");
    }

    #[test]
    fn without_tools_calls_and_results_go_as_text() {
        let turns = vec![
            Turn::User {
                content: "Find it".into(),
            },
            Turn::Assistant {
                content: String::new(),
                calls: vec![call("call-0")],
            },
            Turn::Tool {
                call_id: "call-0".into(),
                name: "search_mail".into(),
                content: "found".into(),
            },
        ];
        let (_, messages) = conversation(&turns, false);
        let text = serde_json::to_string(&messages).unwrap();
        assert!(!text.contains("tool_use"), "{text}");
        assert!(!text.contains("tool_result"), "{text}");
        assert!(text.contains("[search_mail returned]"), "{text}");
    }

    #[test]
    fn a_made_up_call_id_is_one_claude_accepts() {
        assert_eq!(tool_id("toolu_01A-b"), "toolu_01A-b");
        assert_eq!(tool_id("call 0.1"), "call_0_1");
        assert_eq!(tool_id(""), "call");
    }

    #[test]
    fn a_reply_is_its_text_and_calls_and_never_its_thinking() {
        let reply = json!({
            "content": [
                { "type": "thinking", "thinking": "", "signature": "abc" },
                { "type": "text", "text": "Looking." },
                { "type": "tool_use", "id": "toolu_1", "name": "search_mail", "input": { "query": "x" } },
            ],
            "stop_reason": "tool_use",
        });
        let read = answer(&reply, 5).unwrap();
        assert_eq!(read.text, "Looking.");
        assert_eq!(
            read.calls,
            vec![ToolCall {
                id: "toolu_1".into(),
                name: "search_mail".into(),
                arguments: json!({ "query": "x" }),
            }]
        );
        assert!(!read.truncated);
    }

    #[test]
    fn a_refusal_is_an_error_not_an_empty_answer() {
        let reply = json!({
            "content": [],
            "stop_reason": "refusal",
            "stop_details": { "type": "refusal", "category": "cyber" },
        });
        assert!(
            matches!(answer(&reply, 0), Err(AiError::Refused(category)) if category == "cyber")
        );
    }
}
