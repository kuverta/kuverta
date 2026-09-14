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
/// Keyed on the row rather than the URL so that moving an instance does not
/// orphan its token, and so two addresses on one instance keep separate
/// credentials — which they may well need, since Paperless tokens carry a
/// user's permissions. On the row's random key rather than its id, because ids
/// start at 1 in every store and two stores would share one entry.
fn token_entry(mailbox: &StoredPaperMailbox) -> core_accounts::KeychainPassword {
    core_accounts::KeychainPassword::new(format!("paper:{}", mailbox.token_key))
}

/// Where tokens were filed before store schema v9.
fn legacy_token_entry(mailbox: &StoredPaperMailbox) -> core_accounts::KeychainPassword {
    core_accounts::KeychainPassword::new(format!("paper:{}", mailbox.id))
}

/// An address's token, moving it from where it used to be filed if need be.
///
/// The old entry cannot say which store it belonged to — that is the bug. So
/// the first store to ask claims it. If that was the wrong store, the token is
/// for another instance and the check says it was rejected: a token to enter
/// again, rather than one store quietly reading with the other's credentials
/// forever, which is what sharing the entry meant.
fn stored_token(mailbox: &StoredPaperMailbox) -> Result<Option<String>> {
    let keychain = |err: core_accounts::AuthError| RpcError::Auth(err.to_string());
    if let Some(token) = token_entry(mailbox).peek().map_err(keychain)? {
        return Ok(Some(token));
    }
    let legacy = legacy_token_entry(mailbox);
    let Some(token) = legacy.peek().map_err(keychain)? else {
        return Ok(None);
    };
    token_entry(mailbox).store(&token).map_err(keychain)?;
    let _ = legacy.delete();
    Ok(Some(token))
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
    /// The address being edited, or `None` for a new one.
    ///
    /// Without it an edit is indistinguishable from an addition: changing the
    /// URL would save a second address and leave the first behind, with the
    /// token filed under its id.
    #[serde(default)]
    pub id: Option<i64>,
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
    /// Who sent it, as Paperless knows it — `None` when it does not.
    pub correspondent: Option<String>,
    /// Who sent it as far as can be told: the correspondent, or failing that
    /// the letterhead. What filing the letter teaches. Carried separately from
    /// `row.from`, which shows a dash for "unknown" and would otherwise be
    /// learned from as if a dash were a sender.
    pub sender: Option<String>,
    /// What the rules read, so a letter's verdict is explained the way mail's
    /// is — from the same facts the classifier filed it by.
    pub facts: crate::FactsView,
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
                has_token: stored_token(&stored).ok().flatten().is_some(),
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

        let mailbox = NewPaperMailbox {
            label: input.label.trim().to_string(),
            base_url: input.base_url.trim().to_string(),
            selector_kind: input.selector_kind.clone(),
            selector_value: input.selector_value.clone(),
        };

        match input.id {
            Some(id) => {
                if !self.store.update_paper_mailbox(id, &mailbox)? {
                    return Err(RpcError::Rejected(format!("no postal address {id}")));
                }
                Ok(id)
            }
            None => Ok(self.store.upsert_paper_mailbox(&mailbox)?),
        }
    }

    pub fn delete_paper_mailbox(&self, id: i64) -> Result<()> {
        // The token goes with it. Leaving a credential behind for a mailbox
        // nobody can see is how a keychain fills with things nobody can
        // account for.
        if let Some(mailbox) = self.store.paper_mailbox(id)? {
            let _ = token_entry(&mailbox).delete();
            let _ = legacy_token_entry(&mailbox).delete();
        }
        Ok(self.store.delete_paper_mailbox(id)?)
    }

    pub fn set_paper_token(&self, id: i64, token: &str) -> Result<()> {
        let mailbox = self.mailbox(id)?;
        token_entry(&mailbox)
            .store(token)
            .map_err(|err| RpcError::Auth(err.to_string()))?;
        // An old entry left behind would be claimed later by whichever store
        // shares this id — the very mix-up the new key exists to end.
        let _ = legacy_token_entry(&mailbox).delete();
        Ok(())
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
        let overrides = self
            .store
            .paper_overrides(id)?
            .into_iter()
            .filter_map(|(document, category)| {
                core_rules::Category::parse(&category).map(|category| (document, category))
            })
            .collect();
        Ok(PaperSession {
            client,
            selector,
            classifier: self.paper_classifier()?,
            overrides,
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

    /// Records that the user filed a piece of post by hand.
    ///
    /// Two effects, deliberately. The document itself shows the chosen
    /// category from now on, whatever the rules say. And if it can be told who
    /// sent it — the correspondent, or the letterhead when Paperless has none —
    /// the sender is learned: the next letter from them is filed the same way
    /// without being asked. A document with neither teaches nothing, because
    /// there is nothing to key it on.
    ///
    /// Takes the correspondent and the shown category from the caller because
    /// finding them means asking Paperless, and a caller holding `Core` behind
    /// a lock must not hold it across that. [`Core::file_post`] does the asking
    /// for callers that can.
    pub fn record_paper_correction(
        &self,
        mailbox_id: i64,
        document_id: i64,
        correspondent: Option<&str>,
        shown: Option<&str>,
        category: &str,
    ) -> Result<()> {
        let category = core_rules::Category::parse(category)
            .ok_or_else(|| RpcError::Rejected(format!("not a category: {category}")))?;
        self.mailbox(mailbox_id)?;

        // Filed there by hand already: another row would be a correction that
        // corrects nothing, in the one dataset kept deliberately clean.
        let already = self
            .store
            .paper_overrides(mailbox_id)?
            .into_iter()
            .find(|(document, _)| *document == document_id)
            .map(|(_, filed)| filed);
        if already.as_deref() == Some(category.as_str()) {
            return Ok(());
        }

        self.store.record_paper_correction(
            mailbox_id,
            document_id,
            correspondent.filter(|name| !name.trim().is_empty()),
            shown,
            category.as_str(),
        )?;
        Ok(())
    }

    /// Files a piece of post by hand, asking Paperless who sent it.
    pub async fn file_post(&self, mailbox_id: i64, document_id: i64, category: &str) -> Result<()> {
        let detail = self
            .paper_session(mailbox_id)?
            .document(document_id)
            .await?;
        self.record_paper_correction(
            mailbox_id,
            document_id,
            detail.sender.as_deref(),
            detail.row.category.as_deref(),
            category,
        )
    }

    // -- internals ---------------------------------------------------------

    fn mailbox(&self, id: i64) -> Result<StoredPaperMailbox> {
        self.store
            .paper_mailbox(id)?
            .ok_or_else(|| RpcError::Rejected(format!("no postal address {id}")))
    }

    fn client(&self, id: i64) -> Result<(Paperless, Selector)> {
        let mailbox = self.mailbox(id)?;
        let token = stored_token(&mailbox)?
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
    /// came on paper. Corrections made on post are loaded after, so where the
    /// two disagree about a correspondent the letter's filing wins — it is the
    /// one made about paper.
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
        for (correspondent, category) in self.store.paper_learned()? {
            if let Some(category) = core_rules::Category::parse(&category) {
                // Documents use the correspondent as their address, so this is
                // the key the classifier will look them up by.
                learned.insert_sender(&correspondent, category);
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
    /// Documents the user filed by hand, and where. A person's filing of one
    /// letter is not evidence to weigh against the rules — it is the answer.
    overrides: std::collections::HashMap<i64, core_rules::Category>,
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
                .map(|document| to_row(document, &self.classifier, &self.overrides))
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
            row: to_row(&document, &self.classifier, &self.overrides),
            correspondent: document.correspondent.clone(),
            sender: document.sender().map(str::to_string),
            facts: crate::FactsView::from(&document.facts()),
            body_text: document.content.clone(),
            download_url: document.download_path.clone(),
        })
    }

    pub async fn check(&self) -> Result<PaperReport> {
        self.client.check(&self.selector).await.map_err(paper_error)
    }
}

fn to_row(
    document: &Document,
    classifier: &core_rules::Classifier,
    overrides: &std::collections::HashMap<i64, core_rules::Category>,
) -> PaperRow {
    let category = overrides
        .get(&document.id)
        .copied()
        .unwrap_or_else(|| classifier.classify(&document.facts()).category);
    PaperRow {
        id: document.id,
        date_utc: document.created_utc,
        from: document.sender().unwrap_or("—").to_string(),
        subject: document.subject().to_string(),
        unread: false,
        has_attachments: true,
        category: Some(category.as_str().to_string()),
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
