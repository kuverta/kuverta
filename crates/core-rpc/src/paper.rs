//! Postal addresses, as a mailbox the shell can read.
//!
//! A physical address is an account and Paperless-ngx is its server, so this
//! sits exactly where the IMAP side's account handling sits: the store holds
//! where the instance is and which of its documents belong to this address,
//! the keychain holds the token, and the rows that come back have the shape a
//! message row has.
//!
//! Nothing here writes to Paperless. It owns the documents; this reads them.

use std::collections::{HashMap, HashSet};

use core_paper::{Document, Order, PaperReport, Paperless, Selector};
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
    /// The profile it is in, when it is in one.
    pub profile_id: Option<i64>,
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
    /// The date on the letter.
    pub date_utc: Option<i64>,
    /// When it reached Paperless — when it was scanned.
    pub added_utc: Option<i64>,
    pub from: String,
    pub subject: String,
    /// Until it is opened in kuverta. Paperless keeps no read state, so this
    /// is kuverta's own, which is what a freshly scanned letter should be.
    pub unread: bool,
    /// The vision model whose transcript the row was read from, if any.
    pub transcribed_by: Option<String>,
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
    /// The body: a vision model's transcript when there is one, Paperless's
    /// OCR text otherwise.
    pub body_text: Option<String>,
    /// Paperless's OCR text, whatever the body is.
    pub ocr_text: Option<String>,
    /// The model that transcribed the body, when it is a transcript.
    pub transcript_model: Option<String>,
    /// Where the PDF is, for a viewer that wants it.
    pub download_url: String,
}

impl Core {
    pub fn paper_mailboxes(&self) -> Result<Vec<PaperMailboxView>> {
        let profiles: std::collections::HashMap<i64, Option<i64>> =
            self.store.paper_profiles()?.into_iter().collect();
        Ok(self
            .store
            .paper_mailboxes()?
            .into_iter()
            .map(|stored| PaperMailboxView {
                profile_id: profiles.get(&stored.id).copied().flatten(),
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
        let mailbox = self.mailbox(id)?;
        let (client, selector) = self.client(id)?;
        let overrides = self
            .store
            .paper_overrides(id)?
            .into_iter()
            .filter_map(|(document, category)| {
                core_rules::Category::parse(&category).map(|category| (document, category))
            })
            .collect();
        let read = self
            .store
            .paper_read_ids(&mailbox.base_url)?
            .into_iter()
            .collect();
        let transcripts = self
            .store
            .paper_transcripts(&mailbox.base_url)?
            .into_iter()
            .map(|(document, model, text)| (document, Transcript { model, text }))
            .collect();
        Ok(PaperSession {
            client,
            selector,
            classifier: self.paper_classifier()?,
            overrides,
            read,
            transcripts,
        })
    }

    /// Marks a letter read or unread in kuverta.
    ///
    /// Paperless keeps no read state, so this is kuverta's own — kept per
    /// instance, so a letter read under one address is read under any other
    /// that shows it.
    pub fn set_paper_read(&self, mailbox_id: i64, document_id: i64, read: bool) -> Result<()> {
        let mailbox = self.mailbox(mailbox_id)?;
        Ok(self
            .store
            .set_paper_read(&mailbox.base_url, document_id, read)?)
    }

    /// Keeps a vision model's transcript of a letter. From then on it is what
    /// the letter is shown, searched by eye and sorted by, in place of
    /// Paperless's OCR.
    pub fn save_paper_transcript(
        &self,
        mailbox_id: i64,
        document_id: i64,
        model: &str,
        text: &str,
    ) -> Result<()> {
        let mailbox = self.mailbox(mailbox_id)?;
        Ok(self
            .store
            .save_paper_transcript(&mailbox.base_url, document_id, model, text)?)
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

/// A vision model's reading of a scan.
#[derive(Debug, Clone)]
struct Transcript {
    model: String,
    text: String,
}

/// One address, ready to read.
pub struct PaperSession {
    client: Paperless,
    selector: Selector,
    classifier: core_rules::Classifier,
    /// Documents the user filed by hand, and where. A person's filing of one
    /// letter is not evidence to weigh against the rules — it is the answer.
    overrides: HashMap<i64, core_rules::Category>,
    /// Letters opened in kuverta.
    read: HashSet<i64>,
    /// Transcripts kept for this instance, by document.
    transcripts: HashMap<i64, Transcript>,
}

impl PaperSession {
    pub async fn documents(
        &self,
        offset: usize,
        limit: usize,
        query: Option<&str>,
    ) -> Result<PaperPage> {
        self.documents_in_order(offset, limit, query, Order::Created, None)
            .await
    }

    /// One window of the address's post, newest first by the letter's date or
    /// by when it was scanned, and within one category when one is given.
    ///
    /// A category is kuverta's, not Paperless's, so it cannot be a filter in
    /// the request: the letters are classified here and the window taken from
    /// those in the category. That means fetching them all, which for a
    /// household's post is a few requests.
    pub async fn documents_in_order(
        &self,
        offset: usize,
        limit: usize,
        query: Option<&str>,
        order: Order,
        category: Option<&str>,
    ) -> Result<PaperPage> {
        if let Some(category) = category {
            let rows: Vec<PaperRow> = self
                .all_rows(query, order)
                .await?
                .into_iter()
                .filter(|row| row.category.as_deref() == Some(category))
                .collect();
            return Ok(PaperPage {
                total: rows.len(),
                offset,
                rows: rows.into_iter().skip(offset).take(limit).collect(),
            });
        }

        let page = self
            .client
            .documents_in_order(&self.selector, offset, limit, query, order)
            .await
            .map_err(paper_error)?;
        Ok(PaperPage {
            total: page.total,
            offset: page.offset,
            rows: page
                .documents
                .iter()
                .map(|document| self.row(document))
                .collect(),
        })
    }

    /// [`PaperSession::documents_in_order`] with the order by name — `"added"`
    /// for when a letter was scanned, anything else for its date — for callers
    /// on the far side of a bridge.
    pub async fn documents_by(
        &self,
        offset: usize,
        limit: usize,
        query: Option<&str>,
        order: Option<&str>,
        category: Option<&str>,
    ) -> Result<PaperPage> {
        let order = if order == Some("added") {
            Order::Added
        } else {
            Order::Created
        };
        self.documents_in_order(offset, limit, query, order, category)
            .await
    }

    /// How many of the address's letters are in each category, most first.
    pub async fn category_counts(&self) -> Result<Vec<(String, usize)>> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for row in self.all_rows(None, Order::Created).await? {
            let Some(category) = row.category else {
                continue;
            };
            match counts.iter_mut().find(|(name, _)| *name == category) {
                Some((_, count)) => *count += 1,
                None => counts.push((category, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(counts)
    }

    /// How many of the address's letters have not been opened in kuverta.
    pub async fn unread_count(&self) -> Result<usize> {
        let ids = self
            .client
            .document_ids(&self.selector)
            .await
            .map_err(paper_error)?;
        Ok(ids.iter().filter(|id| !self.read.contains(id)).count())
    }

    pub async fn document(&self, document_id: i64) -> Result<PaperDetail> {
        let document = self
            .client
            .document(document_id)
            .await
            .map_err(paper_error)?;
        let readable = self.readable(&document);

        Ok(PaperDetail {
            row: self.row(&document),
            correspondent: document.correspondent.clone(),
            sender: readable.sender().map(str::to_string),
            facts: crate::FactsView::from(&readable.facts()),
            body_text: readable.content.clone(),
            ocr_text: document.content.clone(),
            transcript_model: self
                .transcripts
                .get(&document_id)
                .map(|transcript| transcript.model.clone()),
            download_url: document.download_path.clone(),
        })
    }

    /// Has a vision model read the letter's scan, page by page, and returns
    /// the text. Keeping it is the caller's: this holds no store.
    ///
    /// From the original upload rather than Paperless's archive — for a
    /// scannerd letter, the photographs as the camera took them rather than a
    /// re-encoding. A document that is a photograph is read as it is; a PDF
    /// with no photographs in it was never a scan, has nothing for a vision
    /// model to read, and is refused.
    pub async fn transcribe(
        &self,
        document_id: i64,
        provider: &core_ai::Provider,
        model: &str,
    ) -> Result<String> {
        let original = self
            .client
            .download_original(document_id)
            .await
            .map_err(paper_error)?;
        let pages = if original.bytes.starts_with(&[0xFF, 0xD8]) {
            vec![original.bytes]
        } else {
            core_paper::scan::jpeg_pages(&original.bytes)
        };
        if pages.is_empty() {
            return Err(RpcError::Rejected(
                "this document has no photographed pages to read; Paperless's text is all there is"
                    .into(),
            ));
        }

        let mut texts = Vec::with_capacity(pages.len());
        for page in &pages {
            texts.push(crate::ai::read_page(provider, model, page).await?);
        }
        Ok(texts.join("\n\n"))
    }

    /// Every letter the address holds, as rows — for what Paperless cannot
    /// filter or count by. Capped, so a runaway instance is not read whole.
    async fn all_rows(&self, query: Option<&str>, order: Order) -> Result<Vec<PaperRow>> {
        const PAGE: usize = 100;
        const MOST: usize = 5000;
        let mut rows = Vec::new();
        loop {
            let page = self
                .client
                .documents_in_order(&self.selector, rows.len(), PAGE, query, order)
                .await
                .map_err(paper_error)?;
            let got = page.documents.len();
            rows.extend(page.documents.iter().map(|document| self.row(document)));
            if got < PAGE || rows.len() >= page.total || rows.len() >= MOST {
                break;
            }
        }
        Ok(rows)
    }

    fn row(&self, document: &Document) -> PaperRow {
        to_row(
            &self.readable(document),
            &self.classifier,
            &self.overrides,
            self.read.contains(&document.id),
            self.transcripts
                .get(&document.id)
                .map(|transcript| transcript.model.as_str()),
        )
    }

    /// The document as it is best read: with a transcript in place of
    /// Paperless's OCR text when there is one, so the sender, subject, snippet
    /// and category all come from text a person could actually read.
    fn readable(&self, document: &Document) -> Document {
        match self.transcripts.get(&document.id) {
            Some(transcript) => Document {
                content: Some(transcript.text.clone()),
                ..document.clone()
            },
            None => document.clone(),
        }
    }

    pub async fn check(&self) -> Result<PaperReport> {
        self.client.check(&self.selector).await.map_err(paper_error)
    }

    /// The document's file — the scan itself, for when its OCR text is no use.
    pub async fn download(&self, document_id: i64) -> Result<core_paper::Download> {
        self.client.download(document_id).await.map_err(paper_error)
    }
}

fn to_row(
    document: &Document,
    classifier: &core_rules::Classifier,
    overrides: &HashMap<i64, core_rules::Category>,
    read: bool,
    transcribed_by: Option<&str>,
) -> PaperRow {
    let category = overrides
        .get(&document.id)
        .copied()
        .unwrap_or_else(|| classifier.classify(&document.facts()).category);
    PaperRow {
        id: document.id,
        date_utc: document.created_utc,
        added_utc: document.added_utc,
        from: document.sender().unwrap_or("—").to_string(),
        subject: document.subject().to_string(),
        unread: !read,
        transcribed_by: transcribed_by.map(str::to_string),
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

pub(crate) fn selector_from(kind: &str, value: Option<&str>) -> Result<Selector> {
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
