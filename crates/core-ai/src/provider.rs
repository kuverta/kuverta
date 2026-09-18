//! Where a model runs: Ollama, or a hosted service that speaks OpenAI's API.
//!
//! Most hosted services — DeepSeek, OpenAI itself, OpenRouter, Mistral, Groq —
//! take the same `/chat/completions` request, so one client covers them given
//! the service's address and a key. What differs is what a model can do, and
//! only some services say: Ollama lists each model's capabilities, OpenRouter
//! its input modalities, and the rest a bare list of names. Where nobody says,
//! [`ModelInfo::vision`] is `None` rather than a guess.

use std::future::Future;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::ollama::{AiError, ChatReply, Ollama, TRANSCRIBE};

/// A model a provider offers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub size_bytes: Option<u64>,
    /// Whether it can be shown a page. `None` when the provider does not say.
    pub vision: Option<bool>,
    /// An embedding model, which cannot be asked anything.
    pub embedding: bool,
}

/// Anything a prompt can be sent to.
pub trait Chat {
    fn chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> impl Future<Output = Result<ChatReply, AiError>> + Send;
}

impl Chat for Ollama {
    fn chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> impl Future<Output = Result<ChatReply, AiError>> + Send {
        Ollama::chat(self, model, system, user)
    }
}

/// A hosted service with an OpenAI-compatible API.
pub struct OpenAiCompatible {
    base: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatible {
    /// `base_url` is where `/models` and `/chat/completions` are, e.g.
    /// `https://api.deepseek.com` or `https://api.openai.com/v1`.
    pub fn new(base_url: &str, key: Option<String>) -> Result<Self, AiError> {
        let base = base_url.trim().trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AiError::Url(base_url.to_string()));
        }
        let http = reqwest::Client::builder()
            // A page read by a large hosted model can take a minute.
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|err| AiError::Network(err.to_string()))?;
        Ok(Self {
            base,
            key: key.filter(|key| !key.trim().is_empty()),
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// The models the key can use.
    pub async fn models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let reply = self
            .send(self.http.get(format!("{}/models", self.base)))
            .await?;
        let data = reply
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| AiError::Shape("no data list in the reply".into()))?;
        let mut models: Vec<ModelInfo> = data
            .iter()
            .filter_map(|model| {
                let name = model.get("id")?.as_str()?.to_string();
                let vision = model
                    .pointer("/architecture/input_modalities")
                    .and_then(Value::as_array)
                    .map(|modalities| modalities.iter().any(|modality| modality == "image"));
                Some(ModelInfo {
                    embedding: name.contains("embed"),
                    name,
                    size_bytes: None,
                    vision,
                })
            })
            .collect();
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }

    /// One exchange, deterministic and short, as [`Ollama::chat`].
    pub async fn chat(&self, model: &str, system: &str, user: &str) -> Result<ChatReply, AiError> {
        self.chat_up_to(model, system, user, 12).await
    }

    /// As [`Ollama::chat_up_to`].
    pub async fn chat_up_to(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ChatReply, AiError> {
        self.exchange(&json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": 0,
            "max_tokens": max_tokens,
        }))
        .await
    }

    /// A page's text, as [`Ollama::transcribe`], with the page as a data URL.
    pub async fn transcribe(&self, model: &str, jpeg: &[u8]) -> Result<ChatReply, AiError> {
        use base64::Engine as _;
        let image = format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        );
        self.exchange(&json!({
            "model": model,
            "messages": [
                { "role": "system", "content": TRANSCRIBE },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Transcribe this page." },
                        { "type": "image_url", "image_url": { "url": image } },
                    ],
                },
            ],
            "temperature": 0,
            "max_tokens": 4096,
        }))
        .await
    }

    async fn exchange(&self, body: &Value) -> Result<ChatReply, AiError> {
        let started = Instant::now();
        let reply = self
            .send(
                self.http
                    .post(format!("{}/chat/completions", self.base))
                    .json(body),
            )
            .await?;
        let content = reply
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::Shape("no choices[0].message.content in the reply".into()))?;
        Ok(ChatReply {
            content: content.to_string(),
            latency_ms: started.elapsed().as_millis() as i64,
            truncated: reply
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                == Some("length"),
        })
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, AiError> {
        let request = match &self.key {
            Some(key) => request.bearer_auth(key),
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
            // The service says why — a refused key, a model the key cannot
            // use, no credit left — and that is what the person needs to see.
            return Err(AiError::Status {
                status: status.as_u16(),
                body: text.trim().chars().take(500).collect(),
            });
        }
        serde_json::from_str(&text).map_err(|err| AiError::Shape(err.to_string()))
    }
}

impl Chat for OpenAiCompatible {
    fn chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> impl Future<Output = Result<ChatReply, AiError>> + Send {
        OpenAiCompatible::chat(self, model, system, user)
    }
}

/// A provider as configured: either kind, used the same way.
pub enum Provider {
    Ollama(Ollama),
    OpenAi(OpenAiCompatible),
}

impl Provider {
    /// `kind` as the store keeps it: `ollama` or `openai`.
    pub fn connect(kind: &str, base_url: &str, key: Option<String>) -> Result<Self, AiError> {
        match kind {
            "ollama" => Ok(Self::Ollama(Ollama::new(base_url.trim())?)),
            "openai" => Ok(Self::OpenAi(OpenAiCompatible::new(base_url, key)?)),
            other => Err(AiError::Kind(other.to_string())),
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Self::Ollama(ollama) => ollama.base_url(),
            Self::OpenAi(service) => service.base_url(),
        }
    }

    pub async fn models(&self) -> Result<Vec<ModelInfo>, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.models().await,
            Self::OpenAi(service) => service.models().await,
        }
    }

    pub async fn chat(&self, model: &str, system: &str, user: &str) -> Result<ChatReply, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.chat(model, system, user).await,
            Self::OpenAi(service) => service.chat(model, system, user).await,
        }
    }

    /// An answer longer than one word; see [`Ollama::chat_up_to`].
    pub async fn chat_up_to(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ChatReply, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.chat_up_to(model, system, user, max_tokens).await,
            Self::OpenAi(service) => service.chat_up_to(model, system, user, max_tokens).await,
        }
    }

    pub async fn transcribe(&self, model: &str, jpeg: &[u8]) -> Result<ChatReply, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.transcribe(model, jpeg).await,
            Self::OpenAi(service) => service.transcribe(model, jpeg).await,
        }
    }

    /// A second reading of a page the first looped on, where the provider has
    /// a measured way to discourage repetition: Ollama's repeat penalty. `None`
    /// for a hosted service, whose penalties are on another scale and untried.
    pub async fn transcribe_guarded(
        &self,
        model: &str,
        jpeg: &[u8],
    ) -> Result<Option<ChatReply>, AiError> {
        match self {
            Self::Ollama(ollama) => ollama.transcribe_guarded(model, jpeg).await.map(Some),
            Self::OpenAi(_) => Ok(None),
        }
    }
}

impl Chat for Provider {
    fn chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> impl Future<Output = Result<ChatReply, AiError>> + Send {
        Provider::chat(self, model, system, user)
    }
}
