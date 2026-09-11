//! Postal addresses, as a mailbox the shell can read.
//!
//! A physical address is an account and Paperless-ngx is its server, so this
//! sits exactly where the IMAP side's account handling sits: the store holds
//! where the instance is and which of its documents belong to this address,
//! the keychain holds the token, and the rows that come back have the shape a
//! message row has.
//!
//! Nothing here writes to Paperless. It owns the documents; this reads them.

use core_paper::{Document, PaperReport, Paperless, Selector};
use core_store::{NewPaperMailbox, StoredPaperMailbox};
use serde::{Deserialize, Serialize};

use crate::{Core, Result, RpcError};

/// The keychain account name for an address's API token.
///
/// Keyed on the mailbox id rather than the URL so that moving an instance does
/// not orphan its token, and so two addresses on one instance keep separate
/// credentials — which they may well need, since Paperless tokens carry a
/// user's permissions.
fn token_entry(id: i64) -> core_accounts::KeychainPassword {
    core_accounts::KeychainPassword::new(format!("paper:{id}"))
}

/// A configured physical address, as the shell sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperMailboxView {
    pub id: i64,
    pub label: String,
    pub base_url: String,
    pub selector_kind: String,
    pub selector_value: Option<String>,
    /// Whether a token is stored. The token itself never crosses this boundary.
    pub has_token: bool,
}

/// A physical address, as configured.
#[derive(Debug, Clone, Deserialize)]
pub struct PaperMailboxInput {
    pub label: String,
    pub base_url: String,
    /// `everything` | `tag` | `correspondent` | `storage_path`.
    pub selector_kind: String,
    pub selector_value: Option<String>,
}

/// One piece of post, in the shape a message row has.
///
/// The field names match `MessageRow` wherever they mean the same thing, so
/// the list that draws mail draws this without being taught a second shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperRow {
    pub id: i64,
    pub date_utc: Option<i64>,
    pub from: String,
    pub subject: String,
    /// Always false. Paper has no read state — Paperless does not track one,
    /// and inventing one here would be a promise this cannot keep.
    pub unread: bool,
    /// Always true. The document *is* the attachment.
    pub has_attachments: bool,
    pub category: Option<String>,
    pub snippet: Option<String>,
    pub tags: Vec<String>,
    pub page_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPage {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<PaperRow>,
}

/// One document, opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperDetail {
    pub row: PaperRow,
    /// The OCR text, which is the body.
    pub body_text: Option<String>,
    /// Where the PDF is, for a viewer that wants it.
    pub download_url: String,
}

impl Core {
    pub fn paper_mailboxes(&self) -> Result<Vec<PaperMailboxView>> {
        Ok(self
            .store
            .paper_mailboxes()?
            .into_iter()
            .map(|stored| PaperMailboxView {
                has_token: token_entry(stored.id).peek().ok().flatten().is_some(),
                id: stored.id,
                label: stored.label,
                base_url: stored.base_url,
                selector_kind: stored.selector_kind,
                selector_value: stored.selector_value,
            })
            .collect())
    }

    pub fn save_paper_mailbox(&self, input: &PaperMailboxInput) -> Result<i64> {
        // Validated here rather than at the edge of the network: a selector
        // kind the store does not know would be saved happily and then match
        // nothing, which looks exactly like an address with no post.
        selector_from(&input.selector_kind, input.selector_value.as_deref())?;

        if !input.base_url.starts_with("http://") && !input.base_url.starts_with("https://") {
            return Err(RpcError::Rejected(format!(
                "{} is not a usable Paperless URL",
                input.base_url
            )));
        }

        Ok(self.store.upsert_paper_mailbox(&NewPaperMailbox {
            label: input.label.trim().to_string(),
            base_url: input.base_url.trim().to_string(),
            selector_kind: input.selector_kind.clone(),
            selector_value: input.selector_value.clone(),
        })?)
    }

    pub fn delete_paper_mailbox(&self, id: i64) -> Result<()> {
        // The token goes with it. Leaving a credential behind for a mailbox
        // nobody can see is how a keychain fills with things nobody can
        // account for.
        let _ = token_entry(id).delete();
        Ok(self.store.delete_paper_mailbox(id)?)
    }

    pub fn set_paper_token(&self, id: i64, token: &str) -> Result<()> {
        self.mailbox(id)?;
        token_entry(id)
            .store(token)
            .map_err(|err| RpcError::Auth(err.to_string()))
    }

    /// Everything needed to read one address, with no lock held.
    ///
    /// Built synchronously and then used asynchronously, which is the whole
    /// point: the desktop app keeps `Core` behind a `Mutex`, and a guard held
    /// across an `await` is both a deadlock waiting to happen and not `Send`.
    /// So the store is consulted here and released, and the network happens
    /// against what came back.
    pub fn paper_session(&self, id: i64) -> Result<PaperSession> {
        let (client, selector) = self.client(id)?;
        Ok(PaperSession {
            client,
            selector,
            classifier: self.paper_classifier()?,
        })
    }

    /// One window of an address's post, classified.
    pub async fn paper_documents(
        &self,
        id: i64,
        offset: usize,
        limit: usize,
        query: Option<&str>,
    ) -> Result<PaperPage> {
        self.paper_session(id)?
            .documents(offset, limit, query)
            .await
    }

    pub async fn paper_document(&self, id: i64, document_id: i64) -> Result<PaperDetail> {
        self.paper_session(id)?.document(document_id).await
    }

    /// Connects and reports what is there, before anything depends on it.
    pub async fn paper_check(&self, id: i64) -> Result<PaperReport> {
        self.paper_session(id)?.check().await
    }

    // -- internals ---------------------------------------------------------

    fn mailbox(&self, id: i64) -> Result<StoredPaperMailbox> {
        self.store
            .paper_mailbox(id)?
            .ok_or_else(|| RpcError::Rejected(format!("no postal address {id}")))
    }

    fn client(&self, id: i64) -> Result<(Paperless, Selector)> {
        let mailbox = self.mailbox(id)?;
        let token = token_entry(id)
            .peek()
            .map_err(|err| RpcError::Auth(err.to_string()))?
            .ok_or_else(|| RpcError::Auth(format!("no API token stored for {}", mailbox.label)))?;

        let client = Paperless::new(&mailbox.base_url, &token)
            .map_err(|err| RpcError::Rejected(err.to_string()))?;
        let selector = selector_from(&mailbox.selector_kind, mailbox.selector_value.as_deref())?;
        Ok((client, selector))
    }

    /// The classifier post is filed by.
    ///
    /// Carries every account's corrections rather than none: post has no
    /// account of its own, and a correspondent a user has already filed once
    /// on the mail side should not have to be filed again because the letter
    /// came on paper. Corrections made *on* post are not recorded yet — that
    /// needs a table of its own, since `correction` hangs off a message.
    fn paper_classifier(&self) -> Result<core_rules::Classifier> {
        let mut learned = core_rules::Learned::new();
        for account in self.store.accounts()? {
            for row in self.store.learned_categories(account.id)? {
                let Some(category) = core_rules::Category::parse(&row.category) else {
                    continue;
                };
                if let Some(sender) = row.sender {
                    learned.insert_sender(&sender, category);
                }
                if let Some(list_id) = row.list_id {
                    learned.insert_list(&list_id, category);
                }
            }
        }
        Ok(core_rules::Classifier::new(learned))
    }
}

/// One address, ready to read.
pub struct PaperSession {
    client: Paperless,
    selector: Selector,
    classifier: core_rules::Classifier,
}

impl PaperSession {
    pub async fn documents(
        &self,
        offset: usize,
        limit: usize,
        query: Option<&str>,
    ) -> Result<PaperPage> {
        let page = self
            .client
            .documents(&self.selector, offset, limit, query)
            .await
            .map_err(paper_error)?;

        Ok(PaperPage {
            total: page.total,
            offset: page.offset,
            rows: page
                .documents
                .iter()
                .map(|document| to_row(document, &self.classifier))
                .collect(),
        })
    }

    pub async fn document(&self, document_id: i64) -> Result<PaperDetail> {
        let document = self
            .client
            .document(document_id)
            .await
            .map_err(paper_error)?;

        Ok(PaperDetail {
            row: to_row(&document, &self.classifier),
            body_text: document.content.clone(),
            download_url: document.download_path.clone(),
        })
    }

    pub async fn check(&self) -> Result<PaperReport> {
        self.client.check(&self.selector).await.map_err(paper_error)
    }
}

fn to_row(document: &Document, classifier: &core_rules::Classifier) -> PaperRow {
    let verdict = classifier.classify(&document.facts());
    PaperRow {
        id: document.id,
        date_utc: document.created_utc,
        from: document
            .correspondent
            .clone()
            .unwrap_or_else(|| "—".to_string()),
        subject: document.title.clone(),
        unread: false,
        has_attachments: true,
        category: Some(verdict.category.as_str().to_string()),
        snippet: document.content.as_ref().map(|text| snippet(text)),
        tags: document.tags.clone(),
        page_count: document.page_count,
    }
}

/// The first line or so of the OCR text, for the list.
fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 220 {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(220).collect();
    format!("{cut}…")
}

fn selector_from(kind: &str, value: Option<&str>) -> Result<Selector> {
    let named = |value: Option<&str>| -> Result<String> {
        value
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| RpcError::Rejected(format!("a {kind} selector needs a value")))
    };

    Ok(match kind {
        "everything" => Selector::Everything,
        "tag" => Selector::Tag(named(value)?),
        "correspondent" => Selector::Correspondent(named(value)?),
        "storage_path" => Selector::StoragePath(named(value)?),
        other => {
            return Err(RpcError::Rejected(format!(
                "{other} is not a way to select documents"
            )))
        }
    })
}

fn paper_error(err: core_paper::PaperError) -> RpcError {
    match err {
        core_paper::PaperError::Auth => RpcError::Auth("paperless rejected the token".into()),
        core_paper::PaperError::Network(detail) => RpcError::Network(detail),
        other => RpcError::Rejected(other.to_string()),
    }
}
