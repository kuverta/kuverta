//! Handing a capture to Paperless.
//!
//! `POST /api/documents/post_document/`, which is the same endpoint the
//! Paperless web UI uses. Paperless answers with a task id and does the rest —
//! OCR, deskew, rotation, tagging, archiving — which is the whole reason this
//! daemon is six hundred lines and not six thousand.
//!
//! The multipart body is written out by hand. `reqwest`'s multipart support is
//! a feature this workspace does not enable, and turning it on for six crates
//! to save thirty lines in one is the wrong trade — the same call made in
//! `core-paper` for its query strings.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::folders::Folder;

/// Paperless's matching algorithms, as its API numbers them.
const MATCH_ANY_WORD: u64 = 1;
const MATCH_AUTO: u64 = 6;

/// A question about a letter already sent is quick or not worth waiting for:
/// the loop asks again in a few seconds.
const QUESTION: Duration = Duration::from_secs(10);

/// Where Paperless is with a letter it was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// Queued or being read.
    Working,
    /// Filed, as this document.
    Done(u64),
    /// Refused: it has the same document already.
    Duplicate,
    Failed(String),
}

pub struct Uploader {
    base: String,
    token: String,
    http: reqwest::Client,
    /// Tags applied to everything this daemon uploads, by name.
    tags: Vec<String>,
    /// The same tags as Paperless's ids, once looked up.
    ///
    /// The upload endpoint takes only ids — a name is refused with a 400 — but
    /// a name is what a person writes in a unit file. Looked up lazily rather
    /// than at start, because a Pi on post duty boots before its network does,
    /// and cached only once every name resolved.
    tag_ids: Mutex<Option<Vec<u64>>>,
    title_prefix: String,
}

impl Uploader {
    pub fn new(
        base_url: &str,
        token: &str,
        tags: Vec<String>,
        title_prefix: String,
    ) -> Result<Self> {
        let base = base_url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            bail!("{base_url} is not a usable Paperless URL");
        }
        // reqwest is built without a crypto provider (see Cargo.toml), so one
        // has to be the process default before a client is made. An error only
        // means one already is.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Bounded, because the capture loop waits on uploads: a Wi-Fi link that
        // drops mid-request must fail the upload, which keeps the capture, not
        // hang the loop so that the next letter is never seen.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .context("could not set up an HTTP client")?;
        Ok(Self {
            base,
            token: token.to_string(),
            http,
            tags,
            tag_ids: Mutex::new(None),
            title_prefix,
        })
    }

    /// Sends one capture. Returns the task id Paperless answers with.
    pub async fn send(&self, filename: &str, jpeg: Vec<u8>) -> Result<String> {
        if self.token.trim().is_empty() {
            bail!("no Paperless token is set yet (the setup page or PAPERLESS_TOKEN); the capture is kept");
        }
        // Before building anything: a tag that does not exist fails every
        // upload the same way, and the capture has to stay in the spool.
        let tag_ids = self.tag_ids().await?;

        let boundary = format!("----scannerd{}", std::process::id());
        let title = format!("{}{}", self.title_prefix, stem(filename));

        let mut fields: Vec<(String, String)> = vec![("title".into(), title)];
        // Paperless takes repeated `tags` fields, one per tag.
        for id in tag_ids {
            fields.push(("tags".into(), id.to_string()));
        }

        let body = multipart(&boundary, &fields, filename, &jpeg);

        let response = self
            .http
            .post(format!("{}/api/documents/post_document/", self.base))
            .header("Authorization", format!("Token {}", self.token))
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .context("could not reach Paperless")?;

        let text = successful(response).await?;

        // The body is a quoted task id. Worth keeping: it is the only handle
        // on a document whose OCR has not finished, and the only way to tell a
        // successful upload from a successful-looking one.
        Ok(text.trim().trim_matches('"').to_string())
    }

    /// Connects, checks the token, and looks the tags up afresh — for the
    /// setup page, which wants what is wrong in words.
    pub async fn check(&self) -> Result<String> {
        if self.token.trim().is_empty() {
            bail!("no Paperless token is set");
        }
        let response = self
            .http
            .get(format!("{}/api/tags/?page_size=1", self.base))
            .header("Authorization", format!("Token {}", self.token))
            .send()
            .await
            .with_context(|| format!("could not reach Paperless at {}", self.base))?;
        successful(response).await?;

        *self.tag_ids.lock().unwrap() = None;
        let ids = self.tag_ids().await?;
        Ok(match ids.len() {
            0 => "connected; no tags are set".to_string(),
            1 => "connected; the tag exists".to_string(),
            n => format!("connected; all {n} tags exist"),
        })
    }

    async fn tag_ids(&self) -> Result<Vec<u64>> {
        let cached = self.tag_ids.lock().unwrap().clone();
        if let Some(ids) = cached {
            return Ok(ids);
        }

        let mut ids = Vec::with_capacity(self.tags.len());
        for name in &self.tags {
            ids.push(self.tag_id(name).await?);
        }
        *self.tag_ids.lock().unwrap() = Some(ids.clone());
        Ok(ids)
    }

    async fn tag_id(&self, name: &str) -> Result<u64> {
        self.find_tag(name).await?.with_context(|| {
            format!(
                "paperless has no tag named {name:?}: create it there or correct --tag \
                 (captures are kept until then)"
            )
        })
    }

    /// Makes each folder's tag exist in Paperless and match as the folder
    /// says: its words, or Paperless's learning when it has none. Returns
    /// each folder with its tag's id.
    ///
    /// Done before letters are sent, because Paperless decides a document's
    /// tags as it reads it: a tag made afterwards is not given to a letter
    /// already filed.
    pub async fn sync_folders(&self, folders: &[Folder]) -> Result<Vec<(u64, Folder)>> {
        let mut out = Vec::with_capacity(folders.len());
        for folder in folders {
            let words = folder.paperless_match();
            let matching = serde_json::json!({
                "matching_algorithm": if words.is_empty() { MATCH_AUTO } else { MATCH_ANY_WORD },
                "match": words,
                "is_insensitive": true,
            });
            let id = match self.find_tag(&folder.name).await? {
                Some(id) => {
                    let response = self
                        .http
                        .patch(format!("{}/api/tags/{id}/", self.base))
                        .header("Authorization", format!("Token {}", self.token))
                        .timeout(QUESTION)
                        .header("Content-Type", "application/json")
                        .body(matching.to_string())
                        .send()
                        .await
                        .context("could not reach Paperless")?;
                    successful(response)
                        .await
                        .with_context(|| format!("could not set up the tag {:?}", folder.name))?;
                    id
                }
                None => {
                    let mut tag = matching.clone();
                    tag["name"] = folder.name.clone().into();
                    let response = self
                        .http
                        .post(format!("{}/api/tags/", self.base))
                        .header("Authorization", format!("Token {}", self.token))
                        .timeout(QUESTION)
                        .header("Content-Type", "application/json")
                        .body(tag.to_string())
                        .send()
                        .await
                        .context("could not reach Paperless")?;
                    let text = successful(response)
                        .await
                        .with_context(|| format!("could not create the tag {:?}", folder.name))?;
                    serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|tag| tag["id"].as_u64())
                        .with_context(|| {
                            format!("paperless created the tag {:?} but gave no id", folder.name)
                        })?
                }
            };
            out.push((id, folder.clone()));
        }
        Ok(out)
    }

    /// Where Paperless is with the letter it answered `task` for.
    pub async fn task(&self, task: &str) -> Result<Task> {
        let text = self
            .get(&format!("/api/tasks/?task_id={}", encode(task)))
            .await?;
        let answer: serde_json::Value = serde_json::from_str(&text)
            .context("paperless answered the task question with something other than JSON")?;
        // A list, or a page of one in some versions.
        let tasks = answer
            .as_array()
            .or_else(|| answer["results"].as_array())
            .cloned()
            .unwrap_or_default();
        let Some(found) = tasks
            .into_iter()
            .find(|t| t["task_id"].as_str() == Some(task))
        else {
            // Not listed yet: Paperless records a task once a worker has it.
            return Ok(Task::Working);
        };
        // Paperless 2 says `result` and `related_document`, in capitals; 3
        // says `result_data` and `related_document_ids`, in lower case. Both
        // are read, so the Paperless can be either.
        let result = match (&found["result"], &found["result_data"]) {
            (serde_json::Value::String(text), _) => text.clone(),
            (_, serde_json::Value::Null) => String::new(),
            (_, data) => data
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| data.to_string()),
        };
        let document = {
            let id = |value: &serde_json::Value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|id| id.parse().ok()))
            };
            id(&found["related_document"])
                .or_else(|| id(&found["result_data"]["document_id"]))
                .or_else(|| found["related_document_ids"].get(0).and_then(id))
        };
        let status = found["status"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_uppercase();
        Ok(match status.as_str() {
            "SUCCESS" => match document {
                Some(id) => Task::Done(id),
                None => Task::Failed(format!(
                    "filed, but Paperless did not say as what: {result}"
                )),
            },
            "FAILURE" if result.to_lowercase().contains("duplicate") => Task::Duplicate,
            "FAILURE" | "REVOKED" => Task::Failed(result),
            _ => Task::Working,
        })
    }

    /// Deletes a document — into Paperless's trash, from which it can still
    /// be restored there.
    pub async fn delete_document(&self, document: u64) -> Result<()> {
        let response = self
            .http
            .delete(format!("{}/api/documents/{document}/", self.base))
            .header("Authorization", format!("Token {}", self.token))
            .timeout(QUESTION)
            .send()
            .await
            .context("could not reach Paperless")?;
        successful(response).await.map(|_| ())
    }

    /// The tag ids of a document.
    pub async fn document_tags(&self, document: u64) -> Result<Vec<u64>> {
        let text = self.get(&format!("/api/documents/{document}/")).await?;
        let answer: serde_json::Value = serde_json::from_str(&text)
            .context("paperless answered the document question with something other than JSON")?;
        Ok(answer["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.as_u64())
            .collect())
    }

    async fn get(&self, path: &str) -> Result<String> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Token {}", self.token))
            .timeout(QUESTION)
            .send()
            .await
            .context("could not reach Paperless")?;
        successful(response).await
    }

    /// The id of the tag called `name`, if Paperless has one.
    async fn find_tag(&self, name: &str) -> Result<Option<u64>> {
        let response = self
            .http
            .get(format!(
                "{}/api/tags/?name__iexact={}",
                self.base,
                encode(name)
            ))
            .header("Authorization", format!("Token {}", self.token))
            .send()
            .await
            .context("could not reach Paperless")?;

        let text = successful(response).await?;
        let page: serde_json::Value = serde_json::from_str(&text)
            .context("paperless answered the tag lookup with something other than JSON")?;

        // `iexact` is a filter, not a guarantee of one result; take the tag
        // whose name actually matches rather than whichever came first.
        let wanted = name.to_lowercase();
        Ok(page["results"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|tag| tag["name"].as_str().map(str::to_lowercase) == Some(wanted.clone()))
            .and_then(|tag| tag["id"].as_u64()))
    }
}

/// The body of a successful response, or what went wrong in words.
async fn successful(response: reqwest::Response) -> Result<String> {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        bail!("paperless rejected the token");
    }
    if !status.is_success() {
        bail!("paperless answered {status}: {}", text.trim());
    }
    Ok(text)
}

/// The bytes of a `multipart/form-data` body.
fn multipart(boundary: &str, fields: &[(String, String)], filename: &str, file: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(file.len() + 512);

    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"document\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", content_type(filename)).as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    body
}

/// What a file is, by its extension: a letter is a PDF, a single page a
/// photograph.
fn content_type(filename: &str) -> &'static str {
    if filename.to_ascii_lowercase().ends_with(".pdf") {
        "application/pdf"
    } else {
        "image/jpeg"
    }
}

/// A filename without its extension, for the document title.
fn stem(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename)
        .to_string()
}

/// Percent-encodes everything that is not unreserved — the same rule as
/// `core-paper`, for the same reason: tag names are written by people, and an
/// unencoded `&` or space silently looks up a different tag.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
