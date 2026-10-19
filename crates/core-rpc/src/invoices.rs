//! A reviewed invoice collection across the accounts of ONE profile.
//!
//! Mail dates delimit discovery only. Optional document analysis suggests
//! dates and identities; a person reviews the joint selection before copying.
//! Drafts are server-owned, receipts survive partial failures, and confirmation
//! rechecks membership, destination and attachment hashes.
use crate::{attachments::AttachmentView, Core, ProfileView, Result, RpcError};
use chrono::{Datelike, NaiveDate};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

pub mod analysis;
pub mod setup;
pub use setup::*;

const BATCH: usize = 40;
const MAX_ROWS: usize = 2000;
const MAX_MAIL: u64 = 30 * 1024 * 1024;

fn reject(s: impl Into<String>) -> RpcError {
    RpcError::Rejected(s.into())
}
fn db<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(core_store::StoreError::from).map_err(Into::into)
}
fn json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| reject(e.to_string()))
}
fn parse<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| reject(e.to_string()))
}
fn date(s: &str) -> Result<NaiveDate> {
    let value = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| reject("Use a date in YYYY-MM-DD format."))?;
    if value.to_string() != s {
        return Err(reject("Use a date in YYYY-MM-DD format."));
    }
    Ok(value)
}
fn range(from: &str, to: &str) -> Result<(i64, i64)> {
    let start = date(from)?;
    let end = date(to)?;
    if end < start {
        return Err(reject("The end date is before the start date."));
    }
    Ok((
        start.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
        end.succ_opt()
            .ok_or_else(|| reject("Date out of range."))?
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp(),
    ))
}
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceRequest {
    pub profile: i64,
    pub root: String,
    pub from: String,
    pub to: String,
    pub mail_from: String,
    pub mail_to: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceAccount {
    pub id: i64,
    pub email: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceRow {
    pub account: i64,
    pub message: i64,
    pub subject: String,
    pub sender: String,
    pub mail_date: Option<i64>,
    pub attachment: Option<AttachmentView>,
    pub digest: Option<String>,
    /// A code, translated by the shell. Such rows cannot be selected.
    pub issue: Option<String>,
    pub previous_path: Option<String>,
    pub duplicate_of: Option<usize>,
    pub same_name: bool,
    #[serde(default)]
    pub analysis: Option<analysis::InvoiceAnalysis>,
    #[serde(default)]
    pub analysis_error: Option<String>,
    #[serde(default)]
    pub analysis_needs_refresh: bool,
    #[serde(default)]
    pub versions: Vec<usize>,
    #[serde(default)]
    pub previous_versions: Vec<String>,
    #[serde(default)]
    pub review: Option<InvoiceReview>,
    #[serde(default)]
    pub eligibility: InvoiceEligibility,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePreview {
    pub token: String,
    pub request: InvoiceRequest,
    pub profile_name: String,
    pub accounts: Vec<InvoiceAccount>,
    pub scanned: usize,
    pub total: usize,
    pub complete: bool,
    pub limited: bool,
    pub rows: Vec<InvoiceRow>,
    // A stable upper id prevents newly synced mail from making pagination drift.
    pub through_id: i64,
    pub after_id: i64,
    #[serde(default)]
    pub setup_revision: i64,
    #[serde(default = "flat_layout")]
    pub layout: String,
}
fn flat_layout() -> String {
    "flat".into()
}

/// Resolve only generated date segments. Preview/preflight is read-only;
/// creation is reserved for an explicitly confirmed export. Every existing
/// component is checked by filing::destination (including junctions/links).
fn invoice_directory(
    root: &str,
    layout: &str,
    when: &str,
    create: bool,
) -> Result<std::path::PathBuf> {
    let mut dir = crate::filing::destination(root, "")?;
    if layout == "flat" {
        return Ok(dir);
    }
    if layout != "year_quarter_month" {
        return Err(reject("Unknown invoice folder structure."));
    }
    let when = date(when)?;
    if !(1..=9999).contains(&when.year()) {
        return Err(reject("Invoice year must be between 0001 and 9999."));
    }
    let mut relative = String::new();
    for segment in [
        format!("{:04}", when.year()),
        format!("Q{}", (when.month() - 1) / 3 + 1),
        format!("{:02}", when.month()),
    ] {
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(&segment);
        dir.push(&segment);
        match std::fs::symlink_metadata(&dir) {
            Ok(_) => {
                dir = crate::filing::destination(root, &relative)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if create {
                    // A concurrent creator is fine only if it created an ordinary directory.
                    match std::fs::create_dir(&dir) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                        Err(e) => return Err(e.into()),
                    }
                    dir = crate::filing::destination(root, &relative)?;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(dir)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSelection {
    pub row: usize,
    pub invoice_date: String,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceOutcome {
    pub row: usize,
    pub account: i64,
    pub message: i64,
    pub name: String,
    pub digest: String,
    pub invoice_date: String,
    pub kind: String,
    pub status: String,
    pub path: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub analysis: Option<analysis::InvoiceAnalysis>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceExport {
    pub token: String,
    pub profile_name: String,
    pub request: InvoiceRequest,
    pub accounts: Vec<InvoiceAccount>,
    pub created_at: i64,
    pub outcomes: Vec<InvoiceOutcome>,
    pub manifest: Option<String>,
    pub manifest_error: Option<String>,
    pub scanned: usize,
    pub total: usize,
    pub unselected: Vec<InvoiceRow>,
    pub finished: bool,
}
#[derive(Debug, Serialize)]
pub struct InvoiceSettings {
    pub root: Option<String>,
    pub exports: Vec<InvoiceExport>,
    pub setup: InvoiceSetupState,
    pub resumable: Option<InvoicePreview>,
}

/// A chat hand-off to the same reviewed collection used in profile settings.
/// It is not a draft or an authorization to read documents or write files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceCollectionProposal {
    pub profile: i64,
    pub profile_name: String,
    pub from: String,
    pub to: String,
}

impl Core {
    pub fn propose_invoice_collection(
        &self,
        profile_name: &str,
        from: &str,
        to: &str,
    ) -> Result<InvoiceCollectionProposal> {
        range(from, to)?;
        let profile = self.profiles()?.into_iter().find(|p| {
            p.name.to_lowercase() == profile_name.trim().to_lowercase()
        }).ok_or_else(|| reject("Profile not found. Use invoice_profiles and ask which profile the person means; do not substitute the current account."))?;
        if profile.accounts.is_empty() {
            return Err(reject("This profile has no mail accounts."));
        }
        Ok(InvoiceCollectionProposal {
            profile: profile.id,
            profile_name: profile.name,
            from: from.into(),
            to: to.into(),
        })
    }

    fn invoice_profile(&self, id: i64) -> Result<ProfileView> {
        self.profiles()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| reject("Choose an existing profile."))
    }
    fn invoice_token(&self) -> Result<String> {
        db(self
            .store()
            .connection()
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0)))
    }
    pub fn invoice_settings(&self, profile: i64) -> Result<InvoiceSettings> {
        self.invoice_profile(profile)?;
        let conn = self.store().connection();
        let mut stmt = db(conn.prepare("SELECT json FROM invoice_export WHERE profile_id=?1 ORDER BY created_at DESC, rowid DESC LIMIT 20"))?;
        let saved = db(stmt.query_map([profile], |r| r.get::<_, String>(0)))?;
        let mut exports = Vec::new();
        for row in saved {
            exports.push(parse(&db(row)?)?);
        }
        Ok(InvoiceSettings {
            root: self
                .store()
                .preference(&format!("invoice.root.{profile}"))?,
            exports,
            setup: self.invoice_setup(profile)?,
            resumable: self.resumable_invoice_preview(profile)?,
        })
    }
    fn invoice_draft(&self, token: &str) -> Result<InvoicePreview> {
        let text: String = db(self.store().connection().query_row(
            "SELECT json FROM invoice_draft WHERE token=?1",
            [token],
            |r| r.get(0),
        ))?;
        parse(&text)
    }
    fn save_invoice_draft(&self, draft: &InvoicePreview) -> Result<()> {
        db(self.store().connection().execute("INSERT INTO invoice_draft(token,profile_id,json) VALUES(?1,?2,?3) ON CONFLICT(token) DO UPDATE SET json=excluded.json",params![draft.token,draft.request.profile,json(draft)?]))?;
        Ok(())
    }
    fn validate_invoice_scope(&self, draft: &InvoicePreview) -> Result<()> {
        let setup = self.require_invoice_setup(draft.request.profile)?;
        if setup.revision != draft.setup_revision {
            return Err(reject("Invoice setup changed. Start a new preview."));
        }
        let mut current = self.invoice_profile(draft.request.profile)?.accounts;
        let mut before: Vec<_> = draft.accounts.iter().map(|a| a.id).collect();
        current.sort_unstable();
        before.sort_unstable();
        if current != before {
            return Err(reject("Profile accounts changed. Start a new preview."));
        }
        if self
            .store()
            .preference(&format!("invoice.root.{}", draft.request.profile))?
            .as_deref()
            != Some(&draft.request.root)
        {
            return Err(reject(
                "The profile destination changed. Start a new preview.",
            ));
        }
        crate::filing::destination(&draft.request.root, "")?;
        Ok(())
    }
    pub fn start_invoice_collection(&self, mut request: InvoiceRequest) -> Result<InvoicePreview> {
        let setup = self.require_invoice_setup(request.profile)?;
        range(&request.from, &request.to)?;
        let (from, to) = range(&request.mail_from, &request.mail_to)?;
        let profile = self.invoice_profile(request.profile)?;
        if profile.accounts.is_empty() {
            return Err(reject("This profile has no mail accounts."));
        }
        request.root = crate::filing::destination(request.root.trim(), "")?
            .to_string_lossy()
            .into_owned();
        let accounts = self
            .accounts()?
            .into_iter()
            .filter(|a| profile.accounts.contains(&a.id))
            .map(|a| InvoiceAccount {
                id: a.id,
                email: a.email,
            })
            .collect();
        let (through_id,total): (i64,usize) = db(self.store().connection().query_row(
            "SELECT COALESCE(MAX(m.id),0),COUNT(*) FROM message m JOIN account a ON a.id=m.account_id WHERE a.profile_id=?1 AND (m.date_utc IS NULL OR (m.date_utc>=?2 AND m.date_utc<?3))",
            params![profile.id,from,to], |r|Ok((r.get(0)?,r.get::<_,i64>(1)? as usize))))?;
        self.store()
            .set_preference(&format!("invoice.root.{}", profile.id), &request.root)?;
        // Keep just the current preview per profile. Old confirmation tokens
        // consequently cannot act on a newly chosen date range/destination.
        db(self.store().connection().execute(
            "DELETE FROM invoice_draft WHERE profile_id=?1",
            [profile.id],
        ))?;
        let preview = InvoicePreview {
            token: self.invoice_token()?,
            request,
            profile_name: profile.name,
            accounts,
            scanned: 0,
            total,
            complete: total == 0,
            limited: false,
            rows: vec![],
            through_id,
            after_id: 0,
            setup_revision: setup.revision,
            layout: setup.confirmed.as_ref().unwrap().layout.clone(),
        };
        self.save_invoice_draft(&preview)?;
        Ok(preview)
    }
    /// Forty metadata rows per call; only attachment-bearing messages are read.
    /// The UI can stop between calls. Missing bodies/portal-only notices remain
    /// visible rather than silently claiming complete invoice coverage.
    pub fn scan_invoice_collection(&self, token: &str) -> Result<InvoicePreview> {
        let mut draft = self.invoice_draft(token)?;
        self.validate_invoice_scope(&draft)?;
        if draft.complete || draft.limited {
            return Ok(draft);
        }
        let (from, to) = range(&draft.request.mail_from, &draft.request.mail_to)?;
        let mut stmt = db(self.store().connection().prepare(
            "SELECT m.id,m.account_id,m.subject,m.from_addr,m.date_utc,m.has_attachments,m.body_path,m.snippet FROM message m JOIN account a ON a.id=m.account_id WHERE a.profile_id=?1 AND m.id>?2 AND m.id<=?3 AND (m.date_utc IS NULL OR (m.date_utc>=?4 AND m.date_utc<?5)) ORDER BY m.id LIMIT ?6"))?;
        let mapped = db(stmt.query_map(
            params![
                draft.request.profile,
                draft.after_id,
                draft.through_id,
                from,
                to,
                BATCH as i64
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                ))
            },
        ))?;
        let mails = db(mapped.collect::<rusqlite::Result<Vec<_>>>())?;
        let count = mails.len();
        for (id, account, subject, sender, mail_date, attached, path, snippet) in mails {
            if draft.rows.len() >= MAX_ROWS {
                draft.limited = true;
                break;
            }
            draft.scanned += 1;
            draft.after_id = id;
            let base = InvoiceRow {
                account,
                message: id,
                subject,
                sender,
                mail_date,
                attachment: None,
                digest: None,
                issue: None,
                previous_path: None,
                duplicate_of: None,
                same_name: false,
                analysis: None,
                analysis_error: None,
                analysis_needs_refresh: false,
                versions: Vec::new(),
                previous_versions: Vec::new(),
                review: None,
                eligibility: InvoiceEligibility::default(),
            };
            let hint = format!("{} {snippet}", base.subject).to_lowercase();
            let looks_like_invoice = [
                "rechnung",
                "invoice",
                "receipt",
                "gutschrift",
                "credit note",
                "beleg",
            ]
            .iter()
            .any(|w| hint.contains(w));
            if !attached {
                if looks_like_invoice {
                    draft.rows.push(InvoiceRow {
                        issue: Some("no_attachment".into()),
                        ..base
                    });
                }
                continue;
            }
            let attachments = (|| -> Result<Vec<AttachmentView>> {
                let path = path.ok_or_else(|| reject("missing body"))?;
                if std::fs::metadata(self.blobs().root().join(&path))?.len() > MAX_MAIL {
                    return Err(reject("message too large"));
                }
                let raw = self.blobs().get(path)?;
                let parsed = mail_parser::MessageParser::default()
                    .parse(&raw)
                    .ok_or_else(|| reject("invalid MIME"))?;
                if crate::attachments::is_encrypted(&parsed) {
                    return Err(reject("encrypted"));
                }
                Ok(crate::attachments::list(&parsed))
            })();
            let attachments = match attachments {
                Ok(a) => a,
                Err(_) => {
                    draft.rows.push(InvoiceRow {
                        issue: Some("unavailable".into()),
                        ..base
                    });
                    continue;
                }
            };
            let before = draft.rows.len();
            for attachment in attachments.into_iter().filter(|a| !a.inline) {
                if draft.rows.len() >= MAX_ROWS {
                    draft.limited = true;
                    break;
                }
                let mut row = InvoiceRow {
                    attachment: Some(attachment.clone()),
                    ..base.clone()
                };
                // Archives/programs aren't invoices to open or execute here.
                let extension = attachment
                    .name
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if attachment.risky
                    || !["pdf", "png", "jpg", "jpeg", "webp", "txt", "xml"]
                        .contains(&extension.as_str())
                {
                    if !looks_like_invoice {
                        continue;
                    }
                    row.issue = Some("unsupported".into());
                } else {
                    match self.attachment(account, id, attachment.index) {
                        Ok(file) => {
                            let hash = digest(&file.bytes);
                            row.previous_path = self
                                .invoice_receipt(draft.request.profile, &hash)?
                                .filter(|r| r.1)
                                .map(|r| r.0);
                            row.duplicate_of = draft
                                .rows
                                .iter()
                                .position(|r| r.digest.as_deref() == Some(&hash));
                            row.digest = Some(hash);
                        }
                        Err(_) => row.issue = Some("unavailable".into()),
                    }
                }
                draft.rows.push(row);
            }
            if draft.rows.len() == before && looks_like_invoice {
                draft.rows.push(InvoiceRow {
                    issue: Some("no_attachment".into()),
                    ..base
                });
            }
        }
        draft.complete = !draft.limited && (count < BATCH || draft.after_id >= draft.through_id);
        // Different bytes with the same filename merit review, never automatic
        // suppression. Different formats of the same invoice need human review.
        let mut names = std::collections::HashMap::<String, HashSet<String>>::new();
        for r in &draft.rows {
            if let (Some(a), Some(h)) = (&r.attachment, &r.digest) {
                names
                    .entry(a.name.to_lowercase())
                    .or_default()
                    .insert(h.clone());
            }
        }
        for r in &mut draft.rows {
            r.same_name = r.attachment.as_ref().is_some_and(|a| {
                names
                    .get(&a.name.to_lowercase())
                    .is_some_and(|h| h.len() > 1)
            });
        }
        self.enrich_invoice_rows(&mut draft)?;
        self.save_invoice_draft(&draft)?;
        Ok(draft)
    }
    fn invoice_receipt(&self, profile: i64, hash: &str) -> Result<Option<(String, bool)>> {
        db(self
            .store()
            .connection()
            .query_row(
                "SELECT path,saved FROM invoice_copy WHERE profile_id=?1 AND digest=?2",
                params![profile, hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional())
    }
    fn save_invoice_export(&self, export: &InvoiceExport) -> Result<()> {
        db(self.store().connection().execute("INSERT INTO invoice_export(token,profile_id,created_at,json) VALUES(?1,?2,?3,?4) ON CONFLICT(token) DO UPDATE SET json=excluded.json",params![export.token,export.request.profile,export.created_at,json(export)?]))?;
        Ok(())
    }
    /// `write` must exclusively create the exact path (or verify identical
    /// bytes on recovery), mark it as downloaded and flush it before returning.
    pub fn confirm_invoice_collection(
        &self,
        token: &str,
        selection: &[InvoiceSelection],
        mut write: impl FnMut(&Path, &[u8]) -> std::result::Result<(), String>,
    ) -> Result<InvoiceExport> {
        let mut draft = self.invoice_draft(token)?;
        self.validate_invoice_scope(&draft)?;
        self.enrich_invoice_rows(&mut draft)?;
        if !draft.complete || draft.limited {
            return Err(reject(
                "Finish the preview first; narrow the mail period if the limit was reached.",
            ));
        }
        if selection.is_empty() || selection.len() > 500 {
            return Err(reject("Select between 1 and 500 reviewed documents."));
        }
        let mut indexes = HashSet::new();
        let mut bytes = 0;
        // Validate the ENTIRE selection before writing anything.
        for selected in selection {
            let row = draft
                .rows
                .get(selected.row)
                .ok_or_else(|| reject("Unknown preview row."))?;
            if !indexes.insert(selected.row) || row.issue.is_some() || row.digest.is_none() {
                return Err(reject("Invalid selection."));
            }
            if let Some(review) = &row.review {
                if review.setup_revision != draft.setup_revision
                    || review.decision != "include"
                    || review.invoice_date != selected.invoice_date
                    || review.kind != selected.kind
                {
                    return Err(reject(
                        "A saved correction changed. Review the selection again.",
                    ));
                }
            }
            date(&selected.invoice_date)?;
            invoice_directory(
                &draft.request.root,
                &draft.layout,
                &selected.invoice_date,
                false,
            )?;
            if selected.invoice_date < draft.request.from
                || selected.invoice_date > draft.request.to
            {
                return Err(reject(
                    "The document date must be inside the selected accounting period.",
                ));
            }
            if !["invoice", "credit"].contains(&selected.kind.as_str()) {
                return Err(reject("Choose invoice or credit note."));
            }
            let a = row
                .attachment
                .as_ref()
                .ok_or_else(|| reject("No attachment."))?;
            let file = self.attachment(row.account, row.message, a.index)?;
            if digest(&file.bytes) != *row.digest.as_ref().unwrap() || file.view != *a {
                return Err(reject("An attachment changed. Start a new preview."));
            }
            bytes += file.bytes.len();
            if bytes > 512 * 1024 * 1024 {
                return Err(reject("Select less than 512 MiB per export."));
            }
        }
        let mut export = InvoiceExport {
            token: self.invoice_token()?,
            profile_name: draft.profile_name.clone(),
            request: draft.request.clone(),
            accounts: draft.accounts.clone(),
            created_at: chrono::Utc::now().timestamp(),
            outcomes: vec![],
            manifest: None,
            manifest_error: None,
            scanned: draft.scanned,
            total: draft.total,
            unselected: draft
                .rows
                .iter()
                .enumerate()
                .filter(|(i, _)| !indexes.contains(i))
                .map(|(_, r)| r.clone())
                .collect(),
            finished: false,
        };
        self.save_invoice_export(&export)?;
        for selected in selection {
            let row = &draft.rows[selected.row];
            let attachment = row.attachment.as_ref().unwrap();
            let hash = row.digest.as_ref().unwrap();
            let mut outcome = InvoiceOutcome {
                row: selected.row,
                account: row.account,
                message: row.message,
                name: attachment.name.clone(),
                digest: hash.clone(),
                invoice_date: selected.invoice_date.clone(),
                kind: selected.kind.clone(),
                status: "failed".into(),
                path: None,
                error: None,
                analysis: row.analysis.clone(),
            };
            let result = (|| -> Result<()> {
                self.validate_invoice_scope(&draft)?;
                if let Some((path, true)) = self.invoice_receipt(draft.request.profile, hash)? {
                    outcome.status = "duplicate".into();
                    outcome.path = Some(path);
                    return Ok(());
                }
                let dir = invoice_directory(
                    &draft.request.root,
                    &draft.layout,
                    &selected.invoice_date,
                    true,
                )?;
                let path = dir.join(format!(
                    "{}-p{}-{}-{}",
                    selected.invoice_date,
                    draft.request.profile,
                    &hash[..16],
                    attachment.name
                ));
                let path = if let Some((pending, false)) =
                    self.invoice_receipt(draft.request.profile, hash)?
                {
                    let pending = std::path::PathBuf::from(pending);
                    if pending.parent() != Some(dir.as_path()) {
                        return Err(reject("An interrupted copy belongs to another destination. Restore that destination to finish it."));
                    }
                    pending
                } else {
                    path
                };
                db(self.store().connection().execute("INSERT INTO invoice_copy(profile_id,digest,path,invoice_date,kind) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(profile_id,digest) DO UPDATE SET invoice_date=excluded.invoice_date,kind=excluded.kind",params![draft.request.profile,hash,path.to_string_lossy(),selected.invoice_date,selected.kind]))?;
                let file = self.attachment(row.account, row.message, attachment.index)?;
                if digest(&file.bytes) != *hash {
                    return Err(reject("Attachment changed during export."));
                }
                write(&path, &file.bytes).map_err(reject)?;
                db(self.store().connection().execute(
                    "UPDATE invoice_copy SET saved=1 WHERE profile_id=?1 AND digest=?2",
                    params![draft.request.profile, hash],
                ))?;
                outcome.status = "saved".into();
                outcome.path = Some(path.to_string_lossy().into_owned());
                Ok(())
            })();
            if let Err(error) = result {
                outcome.error = Some(error.to_string());
            }
            export.outcomes.push(outcome);
            self.save_invoice_export(&export)?;
        }
        export.finished = true;
        let manifest_result = (|| -> Result<String> {
            self.validate_invoice_scope(&draft)?;
            let path = crate::filing::destination(&draft.request.root, "")?
                .join(format!("kuverta-rechnungen-{}.json", export.token));
            let text = serde_json::to_vec_pretty(&export).map_err(|e| reject(e.to_string()))?;
            write(&path, &text).map_err(reject)?;
            Ok(path.to_string_lossy().into_owned())
        })();
        match manifest_result {
            Ok(path) => export.manifest = Some(path),
            Err(e) => export.manifest_error = Some(e.to_string()),
        }
        self.save_invoice_export(&export)?;
        Ok(export)
    }
}
