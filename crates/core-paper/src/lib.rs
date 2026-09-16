//! Paperless-ngx, read as a mailbox.
//!
//! A physical address is an account. Post that arrives at it is mail: the
//! correspondent wrote it, the title is what it is about, the OCR text is the
//! body, and the date it was created is when it arrived. Paperless is the
//! server — it does the scanning, OCR, tagging and archiving — and this crate
//! is the client, in exactly the sense `core-proto` is the client for IMAP.
//!
//! That framing is the whole design. Nothing downstream should need to know
//! whether an item came from an IMAP server or a scanner, which is why
//! [`Document`] carries the same shapes a message does and why the rules
//! classifier can file post without being taught anything: it reads a sender,
//! a subject and some body text, and those are facts whatever produced them.
//!
//! What this deliberately does not do is write. Paperless owns the documents;
//! this reads them. Consuming new scans is `scannerd`'s job, upstream.

use std::collections::HashMap;

pub mod scan;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum PaperError {
    #[error("network: {0}")]
    Network(String),

    #[error("paperless rejected the credentials")]
    Auth,

    #[error("paperless answered {status} for {path}")]
    Status { status: u16, path: String },

    #[error("paperless answered something that is not a document list: {0}")]
    Shape(String),

    #[error("{0} is not a usable Paperless URL")]
    Url(String),
}

pub type Result<T> = std::result::Result<T, PaperError>;

/// Where a physical address's post lives, and which of it is that address's.
///
/// One Paperless instance usually holds the post of several addresses — a home
/// and an office, or a person and a company. The selector is what separates
/// them, and it is a Paperless concept rather than one invented here, so a
/// mailbox is set up by tagging in Paperless rather than by configuring
/// something twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperMailbox {
    /// What to call this address in the sidebar. "Home", "Büro".
    pub label: String,
    /// e.g. `http://localhost:8000`.
    pub base_url: String,
    pub selector: Selector,
}

/// Which documents belong to this address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Selector {
    /// Every document in the instance. The right answer when one Paperless
    /// serves one address, which is the common case.
    Everything,
    /// Documents carrying this tag.
    Tag(String),
    /// Documents from this correspondent.
    Correspondent(String),
    /// Documents filed under this storage path.
    StoragePath(String),
}

impl Selector {
    /// The query parameters Paperless filters on.
    ///
    /// `__iexact` throughout: these names are typed by a person into two
    /// different systems, and a mailbox that silently shows nothing because of
    /// a capital letter is the worst possible failure here — it looks exactly
    /// like no post having arrived.
    fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Everything => vec![],
            Self::Tag(name) => vec![("tags__name__iexact", name.clone())],
            Self::Correspondent(name) => vec![("correspondent__name__iexact", name.clone())],
            Self::StoragePath(name) => vec![("storage_path__name__iexact", name.clone())],
        }
    }
}

/// Which date post is listed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// The date on the letter, as Paperless read it — when it was written.
    #[default]
    Created,
    /// When it reached Paperless — when it was scanned.
    Added,
}

impl Order {
    fn param(self) -> &'static str {
        match self {
            Self::Created => "-created",
            Self::Added => "-added",
        }
    }
}

/// One piece of post, in the shape a message has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: i64,
    /// The subject line. Paperless titles a document from its filename or its
    /// content; either way it is what the thing is about.
    pub title: String,
    /// Who sent it. `None` when Paperless has not worked out a correspondent,
    /// which is the paper equivalent of a missing `From`.
    pub correspondent: Option<String>,
    /// When it is dated — the date on the letter, not when it was scanned.
    pub created_utc: Option<i64>,
    /// When it reached the archive. The paper "received" date.
    pub added_utc: Option<i64>,
    pub tags: Vec<String>,
    /// The OCR text, which is the body.
    pub content: Option<String>,
    pub page_count: Option<i64>,
    /// Where the PDF is, relative to the instance.
    pub download_path: String,
}

impl Document {
    /// Who sent it: Paperless's correspondent, or failing that the letterhead.
    ///
    /// A freshly scanned letter has no correspondent — Paperless only suggests
    /// ones it already knows — so without this the first letters from anyone
    /// show a dash, and filing them teaches nothing. The letterhead is the first
    /// line of the OCR text with words in it, cut before any address that
    /// follows on the same line: on a letter, that is who it is from far more
    /// often than not. Only ever a fallback; a correspondent set in Paperless is
    /// the answer.
    pub fn sender(&self) -> Option<&str> {
        self.correspondent.as_deref().or_else(|| self.letterhead())
    }

    /// Whether [`Document::sender`] was read off the page rather than set in
    /// Paperless — worth showing, since a guess should look like one.
    pub fn sender_is_inferred(&self) -> bool {
        self.correspondent.is_none() && self.letterhead().is_some()
    }

    /// What it is about: the title, unless the title is only a scan's name.
    ///
    /// `scannerd` titles uploads `Post <timestamp>`, and Paperless titles
    /// anything else from its filename. A list of those says nothing, so a
    /// machine-made title gives way to the line a letter states its business
    /// on — after the letterhead and the address block, and not the date. A
    /// title a person set is always kept.
    pub fn subject(&self) -> &str {
        if !machine_made(&self.title) {
            return &self.title;
        }
        self.content
            .as_deref()
            .and_then(subject_line)
            .unwrap_or(&self.title)
    }

    fn letterhead(&self) -> Option<&str> {
        self.content
            .as_deref()
            .and_then(|text| lines(text).next())
            .map(letterhead_name)
    }

    /// What the classifier reads, from a document instead of a message.
    ///
    /// This is the whole point of the framing: post is filed by exactly the
    /// rules that file mail, because the rules read a sender, a subject and
    /// some body text, and a letter has all three. No paper-specific
    /// classifier, no second taxonomy, and a correction made on post teaches
    /// the same table a correction made on mail does.
    ///
    /// Two mappings are worth arguing about:
    ///
    /// The correspondent is used as the *address* as well as the name. Paper
    /// has no address to key on, and the learned overrides need something
    /// stable to remember a correction against — "everything from the
    /// Stadtwerke is transactional" is exactly the rule a person wants to
    /// teach it, and the correspondent is what carries that.
    ///
    /// A document always counts as carrying an attachment, because it is one:
    /// there is a PDF, and a scanned invoice is no less an invoice than an
    /// emailed one.
    pub fn facts(&self) -> core_rules::MessageFacts<'_> {
        core_rules::MessageFacts {
            from_addr: self.sender(),
            from_name: self.sender(),
            subject: Some(self.subject()),
            // Paper has no bulk headers. It is post: somebody sent it to an
            // address, and none of `List-Id`, `Precedence` or `Auto-Submitted`
            // has any meaning here.
            list_id: None,
            list_unsubscribe: None,
            precedence: None,
            auto_submitted: None,
            in_reply_to: None,
            has_attachments: true,
            // Addressed to one physical address: yours.
            recipient_count: 1,
            snippet: self.content.as_deref(),
        }
    }
}

/// One window of a mailbox, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentPage {
    /// Documents matching the selector, not just this window.
    pub total: usize,
    pub offset: usize,
    pub documents: Vec<Document>,
}

/// What a connection to a Paperless instance actually found.
///
/// The §3.6 lesson, applied to paper: a preflight is worth more than it costs.
/// Connect, say what the credentials are, say how much post is there and how
/// much of it this mailbox's selector matches — before anyone waits on a list
/// that turns out to be empty for a reason nobody can see.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperReport {
    pub reachable: bool,
    pub authenticated: bool,
    /// Documents in the whole instance.
    pub documents_total: usize,
    /// Documents this mailbox's selector matches.
    pub documents_matching: usize,
    /// Tags that exist, so a selector typo is visible rather than silent.
    pub tags: Vec<String>,
    pub correspondents: Vec<String>,
    pub notes: Vec<String>,
}

/// A document's file, as Paperless served it.
#[derive(Debug, Clone)]
pub struct Download {
    /// Without parameters: `application/pdf`, `image/jpeg`.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// What answers at an address, as far as can be told without a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    /// A Paperless, asking for credentials.
    Paperless,
    /// Something answers, and it is not a Paperless — or not one this can
    /// recognise.
    SomethingElse,
    /// Nothing answers.
    Nothing,
}

/// Whether a Paperless is at `base_url`, with a short timeout, for setup to
/// look in the usual places.
///
/// Its document list refuses an anonymous request with
/// `WWW-Authenticate: Token` — Django REST framework's token scheme, which
/// few other servers on port 8000 will be using. An instance that lets
/// anonymous requests through answers the list itself, and that counts too.
pub async fn probe(base_url: &str) -> Probe {
    let base = base_url.trim().trim_end_matches('/');
    let Ok(http) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    else {
        return Probe::Nothing;
    };
    let response = match http
        .get(format!("{base}/api/documents/"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Probe::Nothing,
    };
    let status = response.status();
    let token_scheme = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("token"));
    if status == reqwest::StatusCode::UNAUTHORIZED && token_scheme {
        return Probe::Paperless;
    }
    if status.is_success() {
        let listing = response.json::<serde_json::Value>().await.ok();
        if listing.is_some_and(|body| body.get("results").is_some() && body.get("count").is_some())
        {
            return Probe::Paperless;
        }
    }
    Probe::SomethingElse
}

/// Exchanges a Paperless user's name and password for their API token — the
/// token the account page in Paperless shows, created if there is none yet.
///
/// So setup can ask for what a person knows rather than send them looking
/// for a token.
pub async fn obtain_token(base_url: &str, username: &str, password: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let base = base_url.trim().trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(PaperError::Url(base_url.to_string()));
    }
    let response = reqwest::Client::new()
        .post(format!("{base}/api/token/"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|err| PaperError::Network(err.to_string()))?;
    let status = response.status();
    // Wrong credentials are a 400 here, with the reason in the body.
    if status == reqwest::StatusCode::BAD_REQUEST
        || status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        return Err(PaperError::Auth);
    }
    if !status.is_success() {
        return Err(PaperError::Status {
            status: status.as_u16(),
            path: "/api/token/".to_string(),
        });
    }
    let token: Token = response
        .json()
        .await
        .map_err(|err| PaperError::Shape(err.to_string()))?;
    Ok(token.token)
}

pub struct Paperless {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Paperless {
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        let base = base_url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(PaperError::Url(base_url.to_string()));
        }
        Ok(Self {
            base,
            token: token.to_string(),
            http: reqwest::Client::new(),
        })
    }

    /// One window of a mailbox.
    ///
    /// `query` is Paperless's own full-text search when set, which means search
    /// over OCR'd text costs nothing here — the index already exists upstream.
    pub async fn documents(
        &self,
        selector: &Selector,
        offset: usize,
        limit: usize,
        query: Option<&str>,
    ) -> Result<DocumentPage> {
        self.documents_in_order(selector, offset, limit, query, Order::Created)
            .await
    }

    /// Every document id the selector matches, and nothing else about them —
    /// what telling read from unread needs, at a few bytes a letter.
    pub async fn document_ids(&self, selector: &Selector) -> Result<Vec<i64>> {
        const PAGE: usize = 1000;
        let mut ids = Vec::new();
        for page in 1.. {
            let mut params = vec![
                ("fields".to_string(), "id".to_string()),
                ("page".to_string(), page.to_string()),
                ("page_size".to_string(), PAGE.to_string()),
            ];
            params.extend(
                selector
                    .params()
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v)),
            );
            let body = self.get("/api/documents/", &params).await?;
            let listing: Listing =
                serde_json::from_value(body).map_err(|err| PaperError::Shape(err.to_string()))?;
            let got = listing.results.len();
            ids.extend(listing.results.into_iter().map(|raw| raw.id));
            if got < PAGE || ids.len() >= listing.count {
                break;
            }
        }
        Ok(ids)
    }

    /// One window of a mailbox, newest first by the date given.
    pub async fn documents_in_order(
        &self,
        selector: &Selector,
        offset: usize,
        limit: usize,
        query: Option<&str>,
        order: Order,
    ) -> Result<DocumentPage> {
        // Paperless pages by number, not by offset, and the page number means
        // nothing without the size: page 2 of 4 and page 2 of 5 start in
        // different places. So the size stays fixed at the caller's limit and a
        // second page is fetched when the offset does not land on a boundary —
        // rather than inflating the size to cover the remainder, which quietly
        // shifts every page under it.
        let page_size = limit.max(1);
        let skip = offset % page_size;
        let mut page = offset / page_size + 1;

        let mut gathered = Vec::new();
        let total = loop {
            let listing = self.page(selector, page, page_size, query, order).await?;
            let (count, got) = (listing.count, listing.results.len());
            gathered.extend(listing.results);

            // Enough to satisfy the window, or the server has run out.
            if gathered.len() >= skip + limit || got < page_size {
                break count;
            }
            page += 1;
        };

        // Names are resolved separately because Paperless answers with ids.
        // Fetched once per call rather than per document: a page of twenty
        // documents from four correspondents should not be twenty-four
        // requests.
        let (correspondents, tags) = self.lookups().await?;

        let documents = gathered
            .into_iter()
            .skip(skip)
            .take(limit)
            .map(|raw| raw.into_document(&correspondents, &tags, &self.base))
            .collect();

        Ok(DocumentPage {
            total,
            offset,
            documents,
        })
    }

    /// One Paperless page, as it sends it.
    async fn page(
        &self,
        selector: &Selector,
        page: usize,
        page_size: usize,
        query: Option<&str>,
        order: Order,
    ) -> Result<Listing> {
        let mut params = vec![
            // Newest first.
            ("ordering".to_string(), order.param().to_string()),
            ("page".to_string(), page.to_string()),
            ("page_size".to_string(), page_size.to_string()),
        ];
        params.extend(
            selector
                .params()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        );
        if let Some(query) = query.filter(|q| !q.trim().is_empty()) {
            params.push(("query".to_string(), query.to_string()));
        }

        let body = self.get("/api/documents/", &params).await?;
        serde_json::from_value(body).map_err(|err| PaperError::Shape(err.to_string()))
    }

    /// One document, with its text.
    pub async fn document(&self, id: i64) -> Result<Document> {
        let body = self.get(&format!("/api/documents/{id}/"), &[]).await?;
        let raw: RawDocument =
            serde_json::from_value(body).map_err(|err| PaperError::Shape(err.to_string()))?;
        let (correspondents, tags) = self.lookups().await?;
        Ok(raw.into_document(&correspondents, &tags, &self.base))
    }

    /// Connects and reports what is there, before anything depends on it.
    pub async fn check(&self, selector: &Selector) -> Result<PaperReport> {
        let mut notes = Vec::new();

        let all = self
            .get(
                "/api/documents/",
                &[("page_size".to_string(), "1".to_string())],
            )
            .await?;
        let all: Listing =
            serde_json::from_value(all).map_err(|err| PaperError::Shape(err.to_string()))?;

        let mut params = vec![("page_size".to_string(), "1".to_string())];
        params.extend(
            selector
                .params()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        );
        let mine = self.get("/api/documents/", &params).await?;
        let mine: Listing =
            serde_json::from_value(mine).map_err(|err| PaperError::Shape(err.to_string()))?;

        let (correspondents, tags) = self.lookups().await?;
        let tag_names: Vec<String> = tags.values().cloned().collect();
        let correspondent_names: Vec<String> = correspondents.values().cloned().collect();

        // The failure this exists to catch: a selector that matches nothing
        // looks exactly like an address that has had no post.
        if mine.count == 0 && all.count > 0 {
            notes.push(match selector {
                Selector::Everything => "this instance holds no documents".to_string(),
                Selector::Tag(name) => format!(
                    "no document carries the tag {name}; the tags that exist are: {}",
                    list(&tag_names)
                ),
                Selector::Correspondent(name) => format!(
                    "no document is from {name}; the correspondents that exist are: {}",
                    list(&correspondent_names)
                ),
                Selector::StoragePath(name) => {
                    format!("no document is filed under the storage path {name}")
                }
            });
        }

        Ok(PaperReport {
            reachable: true,
            authenticated: true,
            documents_total: all.count,
            documents_matching: mine.count,
            tags: tag_names,
            correspondents: correspondent_names,
            notes,
        })
    }

    async fn lookups(&self) -> Result<(HashMap<i64, String>, HashMap<i64, String>)> {
        let correspondents = self.names("/api/correspondents/").await?;
        let tags = self.names("/api/tags/").await?;
        Ok((correspondents, tags))
    }

    async fn names(&self, path: &str) -> Result<HashMap<i64, String>> {
        // 500 covers any realistic instance in one request; Paperless caps
        // page_size itself, so asking for more is not a way to be wrong.
        let body = self
            .get(path, &[("page_size".to_string(), "500".to_string())])
            .await?;
        let listing: NameListing =
            serde_json::from_value(body).map_err(|err| PaperError::Shape(err.to_string()))?;
        Ok(listing
            .results
            .into_iter()
            .map(|entry| (entry.id, entry.name))
            .collect())
    }

    /// A document's file, for a reader that wants the page rather than its
    /// OCR text.
    ///
    /// What Paperless's download serves: the archived PDF it made — pages
    /// turned the right way up and straightened, with a text layer — when it
    /// made one, and the original otherwise, which may be a JPEG.
    pub async fn download(&self, id: i64) -> Result<Download> {
        self.fetch_file(id, false).await
    }

    /// The document as it was uploaded, before Paperless re-encoded it into
    /// its archive — for scannerd's letters, the photographs themselves, which
    /// is what a vision model should read.
    pub async fn download_original(&self, id: i64) -> Result<Download> {
        self.fetch_file(id, true).await
    }

    async fn fetch_file(&self, id: i64, original: bool) -> Result<Download> {
        let params = if original {
            vec![("original".to_string(), "true".to_string())]
        } else {
            Vec::new()
        };
        let response = self
            .send(
                &format!("/api/documents/{id}/download/"),
                &params,
                "application/pdf, image/*;q=0.8, */*;q=0.5",
            )
            .await?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let bytes = response
            .bytes()
            .await
            .map_err(|err| PaperError::Network(err.to_string()))?
            .to_vec();
        Ok(Download {
            content_type,
            bytes,
        })
    }

    async fn get(&self, path: &str, params: &[(String, String)]) -> Result<serde_json::Value> {
        self.send(path, params, "application/json")
            .await?
            .json()
            .await
            .map_err(|err| PaperError::Shape(err.to_string()))
    }

    async fn send(
        &self,
        path: &str,
        params: &[(String, String)],
        accept: &str,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{path}{}", self.base, query_string(params));
        let response = self
            .http
            .get(&url)
            // Paperless's own scheme. Not Bearer: a Bearer token is silently
            // treated as anonymous, which reads as an empty mailbox.
            .header("Authorization", format!("Token {}", self.token))
            .header("Accept", accept)
            .send()
            .await
            .map_err(|err| PaperError::Network(err.to_string()))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(PaperError::Auth);
        }
        if !status.is_success() {
            return Err(PaperError::Status {
                status: status.as_u16(),
                path: path.to_string(),
            });
        }
        Ok(response)
    }
}

/// `?a=b&c=d`, or nothing at all.
///
/// Written out rather than reached for through `reqwest`'s `query`, which this
/// workspace's trimmed feature set does not include — and adding a feature to a
/// dependency six crates share, to save ten lines here, is the wrong trade.
fn query_string(params: &[(String, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let pairs: Vec<String> = params
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect();
    format!("?{}", pairs.join("&"))
}

/// Percent-encodes everything that is not unreserved.
///
/// Deliberately conservative: correspondents and tags are written by people and
/// contain spaces, umlauts and ampersands, and an under-encoded ampersand does
/// not fail — it silently splits one filter into two and returns the wrong
/// documents.
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

// -- reading a letter --------------------------------------------------------
//
// Heuristics, and meant to stay small: they only ever stand in for what
// Paperless has not been told, and a letter a person has given a correspondent
// and a title never reaches them.

/// Lines of OCR text worth reading: trimmed, with at least three letters.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.chars().filter(|c| c.is_alphabetic()).count() >= 3)
}

/// A letterhead line, cut to the name. "Stadtwerke Musterstadt GmbH · Postfach
/// 1234 · 80000 München" is from the Stadtwerke, not from a Postfach.
fn letterhead_name(line: &str) -> &str {
    let name = line
        .find(['·', '|', ','])
        .map(|at| line[..at].trim())
        .filter(|name| name.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .unwrap_or(line);
    clip(name, 80)
}

/// A title that is only a filename: "Post 1789400000-1", "IMG_2044",
/// "scan-0003", or none at all.
fn machine_made(title: &str) -> bool {
    let lowered = title.trim().to_lowercase();
    if lowered.is_empty() || lowered == "(no title)" {
        return true;
    }
    let rest = ["dokument", "document", "post", "scan", "img", "doc"]
        .iter()
        .find_map(|prefix| lowered.strip_prefix(prefix))
        .unwrap_or(&lowered)
        .trim_matches(|c: char| !c.is_alphanumeric());
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | ' ' | ':'))
}

/// The line a letter states its business on.
///
/// DIN 5008 order: letterhead, the address block (often with the sender's
/// return line above it), the date, then the subject. So: skip past the last
/// line near the top that looks like an address, then past any date.
fn subject_line(text: &str) -> Option<&str> {
    let top: Vec<&str> = lines(text).take(12).collect();
    let past_address = top
        .iter()
        .rposition(|line| looks_like_address(line))
        .map_or(1, |at| at + 1);
    lines(text)
        .skip(past_address)
        .find(|line| !looks_like_date(line))
        .map(|line| clip(line, 120))
}

/// A postcode followed by a place: "80331 München", "D-80331 München",
/// "A-1010 Wien". Four digits only with a country prefix, because "2025
/// Einkommensteuer" is a year.
fn looks_like_address(line: &str) -> bool {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(2).any(|pair| {
        let (prefixed, code) = match pair[0].split_once('-') {
            Some((country, code))
                if (1..=2).contains(&country.len())
                    && country.chars().all(|c| c.is_ascii_uppercase()) =>
            {
                (true, code)
            }
            _ => (false, pair[0]),
        };
        let digits = code.chars().all(|c| c.is_ascii_digit());
        let length_fits = code.len() == 5 || (prefixed && code.len() == 4);
        digits && length_fits && pair[1].chars().next().is_some_and(char::is_uppercase)
    })
}

/// A short line carrying a date: "München, 01.09.2026", "Datum: 2026-09-01".
fn looks_like_date(line: &str) -> bool {
    let has_date = line.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_digit());
        let parts: Vec<&str> = word.split(['.', '-', '/']).collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    });
    has_date && line.split_whitespace().count() <= 4
}

/// At most `max` characters, cut on a character boundary.
fn clip(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(at, _)| text[..at].trim_end())
}

fn list(names: &[String]) -> String {
    if names.is_empty() {
        return "none".to_string();
    }
    names.join(", ")
}

// -- what Paperless actually sends ------------------------------------------

#[derive(Deserialize)]
struct Listing {
    count: usize,
    results: Vec<RawDocument>,
}

#[derive(Deserialize)]
struct NameListing {
    results: Vec<NameEntry>,
}

#[derive(Deserialize)]
struct NameEntry {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct RawDocument {
    id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    correspondent: Option<i64>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    added: Option<String>,
    #[serde(default)]
    tags: Vec<i64>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    page_count: Option<i64>,
}

impl RawDocument {
    fn into_document(
        self,
        correspondents: &HashMap<i64, String>,
        tags: &HashMap<i64, String>,
        base: &str,
    ) -> Document {
        Document {
            download_path: format!("{base}/api/documents/{}/download/", self.id),
            correspondent: self
                .correspondent
                .and_then(|id| correspondents.get(&id).cloned()),
            tags: self
                .tags
                .iter()
                .filter_map(|id| tags.get(id).cloned())
                .collect(),
            created_utc: self.created.as_deref().and_then(parse_timestamp),
            added_utc: self.added.as_deref().and_then(parse_timestamp),
            // An untitled document is legal and does happen — the paper
            // equivalent of a message with no subject, and it must not read as
            // a document that failed to load.
            title: if self.title.trim().is_empty() {
                "(no title)".to_string()
            } else {
                self.title
            },
            content: self.content.filter(|text| !text.trim().is_empty()),
            page_count: self.page_count,
            id: self.id,
        }
    }
}

/// Seconds since the epoch from what Paperless sends.
///
/// It sends two shapes depending on the field and the version: a full RFC 3339
/// timestamp, and a bare `YYYY-MM-DD` for dates that have no time. Parsed by
/// hand because the alternative is a date library in a crate that otherwise
/// needs none, and the two shapes are both fixed-width.
fn parse_timestamp(raw: &str) -> Option<i64> {
    let (date, time) = match raw.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (raw, None),
    };

    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;

    let days = days_from_civil(year, month, day);
    let seconds = match time {
        Some(time) => {
            let mut hms = time.trim_end_matches('Z').split(':');
            let hours: i64 = hms.next().unwrap_or("0").parse().unwrap_or(0);
            let minutes: i64 = hms.next().unwrap_or("0").parse().unwrap_or(0);
            // Seconds may carry a fraction or an offset; the whole part is all
            // that matters to a list sorted by day.
            let secs: i64 = hms
                .next()
                .unwrap_or("0")
                .split(['.', '+', '-'])
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            hours * 3600 + minutes * 60 + secs
        }
        None => 0,
    };

    Some(days * 86_400 + seconds)
}

/// Days since 1970-01-01. Howard Hinnant's civil-from-days, inverted.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
