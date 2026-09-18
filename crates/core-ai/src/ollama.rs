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

/// How far a model download has got.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PullProgress {
    /// Ollama's own words: `pulling manifest`, `pulling <digest>`,
    /// `verifying sha256 digest`, `success`.
    pub status: String,
    /// Bytes of the part being downloaded, when it says.
    pub completed: Option<u64>,
    pub total: Option<u64>,
}

/// One line of a download's stream: progress, nothing for a blank line, or
/// the error Ollama reported mid-stream.
fn pull_line(line: &[u8]) -> Result<Option<PullProgress>, AiError> {
    #[derive(Deserialize)]
    struct Line {
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        completed: Option<u64>,
        #[serde(default)]
        total: Option<u64>,
        #[serde(default)]
        error: Option<String>,
    }
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let line: Line = serde_json::from_str(text).map_err(|err| AiError::Shape(err.to_string()))?;
    if let Some(error) = line.error {
        return Err(AiError::Status {
            status: 200,
            body: error,
        });
    }
    Ok(Some(PullProgress {
        status: line.status.unwrap_or_default(),
        completed: line.completed,
        total: line.total,
    }))
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
        self.chat_up_to(model, system, user, 12).await
    }

    /// As [`Self::chat`], with room for an answer of up to `max_tokens` — a
    /// judgement with its reason rather than one word.
    pub async fn chat_up_to(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ChatReply, AiError> {
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
            "options": { "temperature": 0, "num_predict": max_tokens },
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

    /// The server's version, asked with a short timeout: this is how setup
    /// tells an Ollama that is running from one that is not, and a server that
    /// is not there should not keep a window waiting three minutes to say so.
    pub async fn version(&self) -> Result<String, AiError> {
        #[derive(Deserialize)]
        struct Version {
            version: String,
        }
        let response = self
            .http
            .get(format!("{}/api/version", self.base))
            .timeout(Duration::from_secs(3))
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
        let version: Version =
            serde_json::from_str(&text).map_err(|err| AiError::Shape(err.to_string()))?;
        Ok(version.version)
    }

    /// Downloads a model, telling `progress` how far it has got as Ollama
    /// reports it.
    ///
    /// Ollama streams one JSON object a line. A model is gigabytes, so this
    /// has no practical timeout: six hours, against the client's three
    /// minutes, which a download would outrun on any ordinary connection.
    pub async fn pull(
        &self,
        model: &str,
        mut progress: impl FnMut(PullProgress),
    ) -> Result<(), AiError> {
        let mut response = self
            .http
            .post(format!("{}/api/pull", self.base))
            .json(&json!({ "model": model, "stream": true }))
            .timeout(Duration::from_secs(6 * 60 * 60))
            .send()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AiError::Status {
                status: status.as_u16(),
                body: body.trim().to_string(),
            });
        }

        let mut pending = Vec::new();
        let mut succeeded = false;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| AiError::Network(err.to_string()))?
        {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let Some(update) = pull_line(&line)? else {
                    continue;
                };
                succeeded |= update.status == "success";
                progress(update);
            }
        }
        if let Some(update) = pull_line(&pending)? {
            succeeded |= update.status == "success";
            progress(update);
        }
        if !succeeded {
            return Err(AiError::Shape(format!(
                "the download of {model} ended before Ollama said it had finished"
            )));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_line_is_progress_nothing_or_an_error() {
        let update = pull_line(br#"{"status":"pulling 6a0746a1ec1a","digest":"sha256:6a07","total":2019377376,"completed":241970}"#)
            .unwrap()
            .unwrap();
        assert_eq!(update.status, "pulling 6a0746a1ec1a");
        assert_eq!(update.completed, Some(241970));
        assert_eq!(update.total, Some(2019377376));

        assert_eq!(pull_line(b"  \n").unwrap(), None);

        // Ollama reports a model it does not know inside a 200 stream.
        assert!(matches!(
            pull_line(br#"{"error":"pull model manifest: file does not exist"}"#),
            Err(AiError::Status { body, .. }) if body.contains("does not exist")
        ));
    }
}
