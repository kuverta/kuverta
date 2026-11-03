//! Letters on paper, posted by LetterXpress.
//!
//! LetterXpress prints a PDF, folds it into a window envelope and hands it to
//! Deutsche Post. The address is not sent beside the file: it is read off
//! the first page, from where the window of a DIN envelope sits, so a PDF is
//! a letter only if its address is there. What it read comes back with the
//! job, which is how the person sees whether it read it right.
//!
//! Everything is one JSON request with the account's name and API key in
//! it, `"mode": "test"` or `"live"` included. A letter sent in test mode is
//! not printed: it waits in the account's Postbox on the LetterXpress site,
//! to be looked at, sent from there or deleted, and goes away by itself
//! after seven days. That is the default everywhere above this crate, and
//! the only way to try a letter without paying for it.
//!
//! A job can be changed or taken back for fifteen minutes after it was sent,
//! and not once it is done. After that it is on its way.
//!
//! See the LetterXpress API v3 documentation (July 2026).

use base64::Engine;
use md5::{Digest, Md5};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

pub const API: &str = "https://api.letterxpress.de/v3";

#[derive(Debug, thiserror::Error)]
pub enum LetterError {
    #[error("could not reach LetterXpress: {0}")]
    Network(String),

    #[error("LetterXpress did not accept the user name and API key")]
    Auth,

    #[error("LetterXpress refused it ({status}): {message}")]
    Refused { status: u16, message: String },

    #[error("LetterXpress answered something unexpected: {0}")]
    Shape(String),

    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, LetterError>;

/// Whether letters are printed. In test mode they wait in the Postbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Test,
    Live,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Test => "test",
            Mode::Live => "live",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "test" => Some(Mode::Test),
            "live" => Some(Mode::Live),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub api_key: String,
    pub mode: Mode,
}

/// Registered mail, which LetterXpress posts only within Germany.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Registered {
    /// Einschreiben Einwurf: put in the letter box, the delivery recorded.
    Drop,
    /// Einschreiben: handed over against a signature.
    Signed,
}

impl Registered {
    fn code(self) -> &'static str {
        match self {
            Registered::Drop => "r1",
            Registered::Signed => "r2",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        match code {
            "r1" => Some(Registered::Drop),
            "r2" => Some(Registered::Signed),
            _ => None,
        }
    }
}

/// How a letter is printed and posted. The default is the cheapest: black
/// and white, one side, within Germany, not registered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub color: bool,
    pub duplex: bool,
    /// Outside Germany. LetterXpress also reads this off the address, but
    /// prices are asked for one or the other.
    pub international: bool,
    pub registered: Option<Registered>,
}

impl Spec {
    pub fn check(&self) -> Result<()> {
        if self.international && self.registered.is_some() {
            return Err(LetterError::Invalid(
                "LetterXpress sends registered mail only within Germany".into(),
            ));
        }
        Ok(())
    }

    fn specification(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        out.insert("color".into(), json!(if self.color { "4" } else { "1" }));
        out.insert(
            "mode".into(),
            json!(if self.duplex { "duplex" } else { "simplex" }),
        );
        out.insert(
            "shipping".into(),
            json!(if self.international {
                "international"
            } else {
                "national"
            }),
        );
        out
    }
}

/// What the account holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    #[serde(deserialize_with = "number")]
    pub balance: f64,
    pub currency: String,
}

/// A letter sent, as LetterXpress keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    /// `queue`, `hold`, `done`, `canceled`, or `draft` for one in the Postbox.
    pub status: String,
    #[serde(default)]
    pub registered: Option<String>,
    #[serde(default)]
    pub dispatch_date: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub items: Vec<Item>,
}

impl Job {
    /// What it costs, VAT included: the items' amounts are net.
    pub fn total(&self) -> f64 {
        self.items.iter().map(|item| item.amount + item.vat).sum()
    }

    /// The address as LetterXpress read it off the page.
    pub fn address(&self) -> Option<&str> {
        self.items
            .iter()
            .find_map(|item| item.address.as_deref())
            .map(str::trim)
            .filter(|address| !address.is_empty())
    }

    pub fn tracking(&self) -> (Option<&str>, Option<&str>) {
        let code = self
            .items
            .iter()
            .find_map(|item| item.tracking_code.as_deref())
            .filter(|code| !code.trim().is_empty());
        let status = self
            .items
            .iter()
            .find_map(|item| item.tracking_status.as_deref())
            .filter(|status| !status.trim().is_empty() && status.trim() != "--");
        (code, status)
    }

    pub fn registered(&self) -> Option<Registered> {
        self.registered.as_deref().and_then(Registered::from_code)
    }
}

/// One envelope of a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub pages: Option<i64>,
    #[serde(default, deserialize_with = "number")]
    pub amount: f64,
    #[serde(default, deserialize_with = "number")]
    pub vat: f64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tracking_code: Option<String>,
    #[serde(default)]
    pub tracking_status: Option<String>,
}

/// A figure, whether it comes as a number or as text.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<f64, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::Number(n) => Ok(n.as_f64().unwrap_or_default()),
        Value::String(s) => s.trim().parse().map_err(serde::de::Error::custom),
        Value::Null => Ok(0.0),
        other => Err(serde::de::Error::custom(format!("not a figure: {other}"))),
    }
}

/// A letter to send: the PDF, and how.
pub struct Letter<'a> {
    pub pdf: &'a [u8],
    pub spec: Spec,
    /// Shown with the job on the LetterXpress site.
    pub filename: Option<&'a str>,
    /// Kept with the job; at most 255 characters.
    pub notice: Option<&'a str>,
}

pub struct LetterXpress {
    base: String,
    credentials: Credentials,
    http: reqwest::Client,
}

impl LetterXpress {
    pub fn new(credentials: Credentials) -> Self {
        Self::at(API, credentials)
    }

    /// The same, at another address: for tests.
    pub fn at(base: &str, credentials: Credentials) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            credentials,
            http: reqwest::Client::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.credentials.mode
    }

    /// The same account in the other mode: a job sent in test mode is only
    /// found by asking in test mode.
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.credentials.mode = mode;
        self
    }

    pub async fn balance(&self) -> Result<Balance> {
        let data = self
            .call(reqwest::Method::GET, "/balance", json!({}))
            .await?;
        serde_json::from_value(data).map_err(|err| LetterError::Shape(err.to_string()))
    }

    /// What a letter of `pages` pages costs, as LetterXpress quotes it.
    pub async fn price(&self, spec: &Spec, pages: usize) -> Result<f64> {
        spec.check()?;
        let mut specification = spec.specification();
        specification.insert("pages".into(), json!(pages));
        let mut letter = json!({ "specification": specification });
        if let Some(registered) = spec.registered {
            letter["registered"] = json!(registered.code());
        }
        let data = self
            .call(reqwest::Method::GET, "/price", json!({ "letter": letter }))
            .await?;
        let price = data
            .get("price")
            .ok_or_else(|| LetterError::Shape("no price in the answer".into()))?;
        number(price.clone()).map_err(|err: serde_json::Error| LetterError::Shape(err.to_string()))
    }

    /// Sends a letter. In live mode it is printed and posted; in test mode it
    /// waits in the Postbox.
    pub async fn submit(&self, letter: &Letter<'_>) -> Result<Job> {
        letter.spec.check()?;
        if !letter.pdf.starts_with(b"%PDF") {
            return Err(LetterError::Invalid(
                "only a PDF can be sent as a letter".into(),
            ));
        }
        let file = base64::engine::general_purpose::STANDARD.encode(letter.pdf);
        let checksum = format!("{:x}", Md5::digest(file.as_bytes()));
        let mut body = json!({
            "base64_file": file,
            "base64_file_checksum": checksum,
            "specification": letter.spec.specification(),
        });
        if let Some(registered) = letter.spec.registered {
            body["registered"] = json!(registered.code());
        }
        if let Some(name) = letter.filename.filter(|n| !n.trim().is_empty()) {
            body["filename_original"] = json!(name);
        }
        if let Some(notice) = letter.notice.filter(|n| !n.trim().is_empty()) {
            body["notice"] = json!(notice.chars().take(255).collect::<String>());
        }
        let data = self
            .call(
                reqwest::Method::POST,
                "/printjobs",
                json!({ "letter": body }),
            )
            .await?;
        job(data)
    }

    pub async fn job(&self, id: i64) -> Result<Job> {
        let data = self
            .call(reqwest::Method::GET, &format!("/printjobs/{id}"), json!({}))
            .await?;
        job(data)
    }

    /// Takes a job back: possible for fifteen minutes after it was sent, and
    /// not once it is done.
    pub async fn cancel(&self, id: i64) -> Result<()> {
        self.call(
            reqwest::Method::DELETE,
            &format!("/printjobs/{id}"),
            json!({}),
        )
        .await?;
        Ok(())
    }

    /// One request: the credentials go in the body of every one, GET
    /// included, and what comes back is `{status, message, data}`.
    async fn call(&self, method: reqwest::Method, path: &str, body: Value) -> Result<Value> {
        let mut body = body;
        body["auth"] = json!({
            "username": self.credentials.username,
            "apikey": self.credentials.api_key,
            "mode": self.credentials.mode.as_str(),
        });
        let response = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|err| LetterError::Network(err.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| LetterError::Network(err.to_string()))?;
        tracing::debug!(%method, path, status = status.as_u16(), "letterxpress");
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(LetterError::Auth);
        }
        let answer: Value = serde_json::from_str(&text).map_err(|_| {
            if status.is_success() {
                LetterError::Shape(short(&text))
            } else {
                LetterError::Refused {
                    status: status.as_u16(),
                    message: short(&text),
                }
            }
        })?;
        if !status.is_success() {
            return Err(LetterError::Refused {
                status: status.as_u16(),
                message: message(&answer),
            });
        }
        Ok(answer.get("data").cloned().unwrap_or(Value::Null))
    }
}

fn job(data: Value) -> Result<Job> {
    serde_json::from_value(data).map_err(|err| LetterError::Shape(err.to_string()))
}

/// What a refusal says, with the fields it names when it names them.
fn message(answer: &Value) -> String {
    let mut out = answer
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no reason given")
        .to_string();
    if let Some(errors) = answer.get("errors").and_then(Value::as_object) {
        let details: Vec<String> = errors
            .iter()
            .map(|(field, why)| match why {
                Value::Array(lines) => format!(
                    "{field}: {}",
                    lines
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                other => format!("{field}: {other}"),
            })
            .collect();
        if !details.is_empty() {
            out = format!("{out} ({})", details.join("; "));
        }
    }
    out
}

fn short(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > 200 {
        format!("{}…", text.chars().take(200).collect::<String>())
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_mail_abroad_is_refused_before_anything_is_sent() {
        let spec = Spec {
            international: true,
            registered: Some(Registered::Drop),
            ..Spec::default()
        };
        assert!(spec.check().is_err());
        assert!(Spec::default().check().is_ok());
    }

    #[test]
    fn a_job_reads_with_figures_as_text_and_no_tracking_yet() {
        let job: Job = serde_json::from_value(json!({
            "id": 6035143,
            "status": "queue",
            "registered": "r1",
            "items": [{
                "address": "Erika Mustermann, Heidestr. 17, 51147 Köln",
                "pages": 1,
                "amount": "0.68",
                "vat": 0.13,
                "status": "queue",
                "tracking_code": null,
                "tracking_status": "--"
            }]
        }))
        .unwrap();
        assert!((job.total() - 0.81).abs() < 1e-9);
        assert_eq!(
            job.address(),
            Some("Erika Mustermann, Heidestr. 17, 51147 Köln")
        );
        assert_eq!(job.tracking(), (None, None));
        assert_eq!(job.registered(), Some(Registered::Drop));
    }

    #[test]
    fn a_refusal_names_the_fields_it_refused() {
        let said = message(&json!({
            "message": "The given data was invalid.",
            "errors": { "letter.base64_file": ["The file is not a valid PDF."] }
        }));
        assert_eq!(
            said,
            "The given data was invalid. (letter.base64_file: The file is not a valid PDF.)"
        );
    }
}
