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

use anyhow::{bail, Context, Result};

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
        page["results"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|tag| tag["name"].as_str().map(str::to_lowercase) == Some(wanted.clone()))
            .and_then(|tag| tag["id"].as_u64())
            .with_context(|| {
                format!(
                    "paperless has no tag named {name:?}: create it there or correct --tag \
                     (captures are kept until then)"
                )
            })
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
    body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    body
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
