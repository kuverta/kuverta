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

    #[error("{0} is not a kind of model provider kuverta knows")]
    Kind(String),
}

/// What a vision model is told before it is shown a page.
///
/// Exactly, and in the page's own language: a transcript is text to search and
/// sort a letter by, and a translation or a summary would be neither. And no
/// guessing: an unreadable word marked as one is honest, a plausible one made up
/// is not.
pub const TRANSCRIBE: &str = "You transcribe photographed letters. Write out all of the text on \
the page exactly as it is printed, in its original language, top to bottom, one printed line per \
line. Do not translate, summarise, correct, explain or add anything. Where a word cannot be read, \
write [?] instead of guessing it. Reply with the transcription only.";

/// The context a page is read in, in tokens.
///
/// A photographed page is most of Ollama's default 4096 on its own — 3,341
/// tokens for a 1392×1820 scan — which leaves under a thousand for its text,
/// and a dense letter needs more: the context filled and was cut mid-answer.
/// Sixteen thousand holds a page and a full answer, for a few hundred megabytes
/// more memory while a page is being read.
pub const TRANSCRIBE_CONTEXT: u32 = 16384;

fn transcribe_body(model: &str, jpeg: &[u8], guarded: bool) -> Value {
    use base64::Engine as _;
    // Deterministic, so reading a page twice gives the same text. Room for a
    // dense page — those measured took 200 to 864 tokens — and not so much
    // that a loop runs for minutes before it ends.
    let mut options = json!({
        "temperature": 0,
        "num_predict": 2048,
        "num_ctx": TRANSCRIBE_CONTEXT,
    });
    if guarded {
        options["repeat_penalty"] = json!(1.2);
        options["repeat_last_n"] = json!(128);
    }
    json!({
        "model": model,
        "stream": false,
        "messages": [
            { "role": "system", "content": TRANSCRIBE },
            {
                "role": "user",
                "content": "Transcribe this page.",
                "images": [base64::engine::general_purpose::STANDARD.encode(jpeg)],
            },
        ],
        "options": options,
    })
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
    /// The model ran out of room before it finished: the answer is cut short,
    /// and if the context itself filled, whatever came after is not to be
    /// trusted either.
    pub truncated: bool,
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

        self.exchange(&body).await
    }

    /// The text on a photographed page, as a vision model reads it.
    ///
    /// For scans a camera took, where Tesseract's OCR is no use: a vision model
    /// reads through blur and uneven light far better. It can also put a
    /// plausible word where it could not read one, which is why whoever shows
    /// a transcript should keep the scan itself a click away.
    pub async fn transcribe(&self, model: &str, jpeg: &[u8]) -> Result<ChatReply, AiError> {
        self.exchange(&transcribe_body(model, jpeg, false)).await
    }

    /// Reads a page again, penalising the model for repeating itself.
    ///
    /// For a page the plain reading looped on, not for every page. On a tax
    /// office statement the penalty turned a loop of 116 repeats into 30 clean
    /// lines; on pages that read cleanly without it, it cost 5 to 22 points of
    /// similarity and up to a quarter of the numbers — an IBAN is digits
    /// repeating, and so is an amount. See docs/decisions.md §24.
    pub async fn transcribe_guarded(&self, model: &str, jpeg: &[u8]) -> Result<ChatReply, AiError> {
        self.exchange(&transcribe_body(model, jpeg, true)).await
    }

    /// The models pulled into this Ollama, with what each can do.
    ///
    /// The list gives names and sizes; what a model can do takes a `show` per
    /// model, which is local and quick. A model `show` fails for is listed
    /// anyway, with what it can do unknown.
    pub async fn models(&self) -> Result<Vec<crate::ModelInfo>, AiError> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Tag>,
        }
        #[derive(Deserialize)]
        struct Tag {
            name: String,
            #[serde(default)]
            size: Option<u64>,
        }

        let response = self
            .http
            .get(format!("{}/api/tags", self.base))
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
                body: text.trim().to_string(),
            });
        }
        let tags: Tags =
            serde_json::from_str(&text).map_err(|err| AiError::Shape(err.to_string()))?;

        let mut models = Vec::with_capacity(tags.models.len());
        for tag in tags.models {
            let capabilities: Option<Vec<String>> = self
                .post("/api/show", &json!({ "model": tag.name }))
                .await
                .ok()
                .and_then(|show| show.get("capabilities").cloned())
                .and_then(|capabilities| serde_json::from_value(capabilities).ok());
            let has = |what: &str| {
                capabilities
                    .as_ref()
                    .map(|list| list.iter().any(|capability| capability == what))
            };
            models.push(crate::ModelInfo {
                vision: has("vision"),
                embedding: has("embedding").unwrap_or_else(|| tag.name.contains("embed")),
                size_bytes: tag.size,
                name: tag.name,
            });
        }
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }

    async fn exchange(&self, body: &Value) -> Result<ChatReply, AiError> {
        let started = Instant::now();
        let reply = self.post("/api/chat", body).await?;
        let content = reply
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::Shape("no message.content in the reply".into()))?;

        Ok(ChatReply {
            content: content.to_string(),
            latency_ms: started.elapsed().as_millis() as i64,
            truncated: reply.get("done_reason").and_then(Value::as_str) == Some("length"),
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
