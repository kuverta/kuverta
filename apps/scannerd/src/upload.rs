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

use anyhow::{bail, Context, Result};

pub struct Uploader {
    base: String,
    token: String,
    http: reqwest::Client,
    /// Tags applied to everything this daemon uploads, by name.
    tags: Vec<String>,
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
        Ok(Self {
            base,
            token: token.to_string(),
            http: reqwest::Client::new(),
            tags,
            title_prefix,
        })
    }

    /// Sends one capture. Returns the task id Paperless answers with.
    pub async fn send(&self, filename: &str, jpeg: Vec<u8>) -> Result<String> {
        let boundary = format!("----scannerd{}", std::process::id());
        let title = format!("{}{}", self.title_prefix, stem(filename));

        let mut fields: Vec<(String, String)> = vec![("title".into(), title)];
        // Paperless takes repeated `tags` fields, one per tag.
        for tag in &self.tags {
            fields.push(("tags".into(), tag.clone()));
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

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("paperless rejected the token");
        }
        if !status.is_success() {
            bail!("paperless answered {status}: {}", text.trim());
        }

        // The body is a quoted task id. Worth keeping: it is the only handle
        // on a document whose OCR has not finished, and the only way to tell a
        // successful upload from a successful-looking one.
        Ok(text.trim().trim_matches('"').to_string())
    }
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
