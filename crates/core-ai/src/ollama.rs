//! Two endpoints of a local Ollama.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("could not reach the model server: {0}")]
    Network(String),

    #[error("the model server answered {status}: {body}")]
    Status { status: u16, body: String },

    #[error("the model server answered something unexpected: {0}")]
    Shape(String),

    #[error("{0} is not a usable model server URL")]
    Url(String),
}

pub struct Ollama {
    base: String,
    http: reqwest::Client,
}

pub struct ChatReply {
    pub content: String,
    /// Wall time for the request, which is what a user waits on — not the
    /// server's own `eval_duration`, which leaves out loading and queueing.
    pub latency_ms: i64,
}

pub struct Embedded {
    pub vectors: Vec<Vec<f32>>,
    pub latency_ms: i64,
}

impl Ollama {
    pub fn new(base_url: &str) -> Result<Self, AiError> {
        let base = base_url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AiError::Url(base_url.to_string()));
        }
        let http = reqwest::Client::builder()
            // Long, because the first request loads the model — twenty-five
            // seconds for an 8B model on the machine this was written on — and
            // a timeout that fires during a load reads as the server being down.
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|err| AiError::Network(err.to_string()))?;
        Ok(Self { base, http })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// One exchange with a chat model.
    pub async fn chat(&self, model: &str, system: &str, user: &str) -> Result<ChatReply, AiError> {
        let body = json!({
            "model": model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            // Deterministic, so the same mail gets the same verdict twice and a
            // comparison with the rules means something. And short: the answer
            // is one word, and a model given room to explain itself will.
            "options": { "temperature": 0, "num_predict": 12 },
        });

        let started = Instant::now();
        let reply = self.post("/api/chat", &body).await?;
        let content = reply
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::Shape("no message.content in the reply".into()))?;

        Ok(ChatReply {
            content: content.to_string(),
            latency_ms: started.elapsed().as_millis() as i64,
        })
    }

    /// Embeddings for several texts in one request.
    pub async fn embed(&self, model: &str, inputs: &[String]) -> Result<Embedded, AiError> {
        #[derive(Deserialize)]
        struct Reply {
            embeddings: Vec<Vec<f32>>,
        }

        let started = Instant::now();
        let reply = self
            .post("/api/embed", &json!({ "model": model, "input": inputs }))
            .await?;
        let reply: Reply =
            serde_json::from_value(reply).map_err(|err| AiError::Shape(err.to_string()))?;

        // A short answer would silently pair the wrong vector with the wrong
        // message, and every neighbour after it would be someone else's.
        if reply.embeddings.len() != inputs.len() {
            return Err(AiError::Shape(format!(
                "asked for {} embeddings and got {}",
                inputs.len(),
                reply.embeddings.len()
            )));
        }

        Ok(Embedded {
            vectors: reply.embeddings,
            latency_ms: started.elapsed().as_millis() as i64,
        })
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, AiError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?;

        if !status.is_success() {
            // Ollama explains itself in the body — "model not found" is far more
            // use in a log than a bare 404.
            return Err(AiError::Status {
                status: status.as_u16(),
                body: text.trim().to_string(),
            });
        }
        serde_json::from_str(&text).map_err(|err| AiError::Shape(err.to_string()))
    }
}
