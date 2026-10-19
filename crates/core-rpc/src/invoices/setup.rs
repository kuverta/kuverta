//! Profile policy is explicit user input. Mail/model text cannot edit it.
//! A correction applies to exact bytes in one profile, never to a sender.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InvoiceSetup {
    pub company: String,
    pub aliases: Vec<String>,
    pub address: String,
    pub people: Vec<String>,
    pub incoming: bool,
    pub outgoing: bool,
    pub credits: bool,
    pub review_terms: Vec<String>,
    pub accounts: Vec<i64>,
    pub root: String,
    /// Old configurations keep their flat layout until explicitly changed.
    pub layout: String,
    pub period: String,
    pub custom_from: String,
    pub custom_to: String,
}
impl Default for InvoiceSetup {
    fn default() -> Self {
        Self {
            company: String::new(),
            aliases: vec![],
            address: String::new(),
            people: vec![],
            incoming: true,
            outgoing: false,
            credits: true,
            review_terms: [
                "Testrechnung",
                "test invoice",
                "sample invoice",
                "Musterrechnung",
                "pro forma",
                "proforma",
                "privat",
                "private",
            ]
            .map(str::to_owned)
            .to_vec(),
            accounts: vec![],
            root: String::new(),
            layout: "flat".into(),
            period: "month".into(),
            custom_from: String::new(),
            custom_to: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvoiceSetupState {
    pub revision: i64,
    pub confirmed: Option<InvoiceSetup>,
    pub draft: Option<InvoiceSetup>,
    pub needs_confirmation: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceReview {
    pub decision: String,
    pub invoice_date: String,
    pub kind: String,
    pub reason: String,
    pub setup_revision: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvoiceEligibility {
    pub status: String,
    pub direction: String,
    pub reason: String,
}
fn folded(value: &str) -> String {
    value
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn contains_name(value: &str, name: &str) -> bool {
    let name = folded(name);
    name.chars().count() >= 3 && format!(" {} ", folded(value)).contains(&format!(" {name} "))
}
pub fn eligibility(setup: &InvoiceSetup, row: &InvoiceRow) -> InvoiceEligibility {
    let verdict = |status: &str, direction: &str, reason: &str| InvoiceEligibility {
        status: status.into(),
        direction: direction.into(),
        reason: reason.into(),
    };
    let Some(a) = &row.analysis else {
        return verdict("review", "unknown", "invoice_facts_missing");
    };
    if a.kind == "other" {
        return verdict("excluded", "unknown", "invoice_not_invoice");
    }
    let context = format!(
        "{} {} {} {}",
        row.subject,
        row.attachment
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(""),
        a.reason,
        a.recipient
    );
    if setup
        .review_terms
        .iter()
        .any(|term| contains_name(&context, term))
    {
        return verdict("review", "unknown", "invoice_review_term");
    }
    let names = std::iter::once(&setup.company)
        .chain(&setup.aliases)
        .collect::<Vec<_>>();
    let issuer = names.iter().any(|n| contains_name(&a.issuer, n));
    // Old cached recipient text did not prove the role. Require a validated
    // source quote for automatic recipient matching; otherwise review manually.
    let recipient =
        !a.recipient_quote.is_empty() && names.iter().any(|n| contains_name(&a.recipient, n));
    let direction = match (issuer, recipient) {
        (true, false) if !a.recipient.is_empty() && !a.recipient_quote.is_empty() => "outgoing",
        (false, true) if !a.issuer.is_empty() => "incoming",
        _ => {
            return verdict(
                "review",
                "unknown",
                if setup.people.iter().any(|n| contains_name(&a.recipient, n)) {
                    "invoice_person_review"
                } else {
                    "invoice_party_unclear"
                },
            )
        }
    };
    if !setup.address.trim().is_empty() {
        let quote = if direction == "incoming" {
            &a.recipient_quote
        } else {
            &a.issuer_quote
        };
        if !contains_name(quote, &setup.address) {
            return verdict("review", direction, "invoice_address_unconfirmed");
        }
    }
    if (direction == "incoming" && !setup.incoming)
        || (direction == "outgoing" && !setup.outgoing)
        || (a.kind == "credit" && !setup.credits)
    {
        return verdict("excluded", direction, "invoice_type_excluded");
    }
    if !a.supported || row.analysis_needs_refresh {
        return verdict("review", direction, "invoice_facts_missing");
    }
    verdict("suggested", direction, "invoice_profile_matches")
}

impl Core {
    pub fn invoice_setup(&self, profile: i64) -> Result<InvoiceSetupState> {
        let mut accounts = self.invoice_profile(profile)?.accounts;
        accounts.sort_unstable();
        let saved: Option<(i64, Option<String>, Option<String>)> = db(self
            .store()
            .connection()
            .query_row(
                "SELECT revision,json,draft_json FROM invoice_setup WHERE profile_id=?1",
                [profile],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional())?;
        let (revision, confirmed, draft) = saved.unwrap_or((0, None, None));
        let confirmed: Option<InvoiceSetup> = confirmed.map(|s| parse(&s)).transpose()?;
        let draft = draft.map(|s| parse(&s)).transpose()?;
        let needs_confirmation = confirmed.as_ref().is_none_or(|s| s.accounts != accounts);
        Ok(InvoiceSetupState {
            revision,
            confirmed,
            draft,
            needs_confirmation,
        })
    }
    pub(super) fn require_invoice_setup(&self, profile: i64) -> Result<InvoiceSetupState> {
        let state = self.invoice_setup(profile)?;
        if state.needs_confirmation {
            return Err(reject("Set up invoice collection for this profile first."));
        }
        Ok(state)
    }
    pub fn save_invoice_setup(
        &self,
        profile: i64,
        mut setup: InvoiceSetup,
        expected_revision: i64,
        confirm: bool,
    ) -> Result<InvoiceSetupState> {
        let current = self.invoice_setup(profile)?;
        if current.revision != expected_revision {
            return Err(reject("Invoice setup changed. Reopen its settings."));
        }
        setup.company = setup.company.trim().into();
        setup.address = setup.address.trim().into();
        setup.root = setup.root.trim().into();
        for list in [
            &mut setup.aliases,
            &mut setup.people,
            &mut setup.review_terms,
        ] {
            *list = list
                .iter()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if list.len() > 50 || list.iter().any(|s| s.chars().count() > 300) {
                return Err(reject("Keep each list below 50 entries of 300 characters."));
            }
        }
        if json(&setup)?.len() > 32_768 {
            return Err(reject("Invoice setup is too large."));
        }
        if !["month", "quarter", "year", "custom"].contains(&setup.period.as_str()) {
            return Err(reject("Choose a standard or custom period."));
        }
        if !["flat", "year_quarter_month"].contains(&setup.layout.as_str()) {
            return Err(reject("Choose a supported invoice folder structure."));
        }
        setup.accounts.sort_unstable();
        setup.accounts.dedup();
        if confirm {
            if folded(&setup.company).chars().count() < 3 {
                return Err(reject("Enter the actual company or invoice recipient."));
            }
            if !setup.incoming && !setup.outgoing {
                return Err(reject("Choose incoming or outgoing invoices."));
            }
            let mut accounts = self.invoice_profile(profile)?.accounts;
            accounts.sort_unstable();
            if accounts.is_empty() || accounts != setup.accounts {
                return Err(reject("Confirm all current mail accounts in this profile."));
            }
            if setup.period == "custom" {
                range(&setup.custom_from, &setup.custom_to)?;
            }
            setup.root = crate::filing::destination(&setup.root, "")?
                .to_string_lossy()
                .into_owned();
        }
        let encoded = json(&setup)?;
        let conn = self.store().connection();
        let tx = db(conn.unchecked_transaction())?;
        if confirm {
            let revision = if current.confirmed.as_ref() == Some(&setup) {
                current.revision
            } else {
                current.revision + 1
            };
            db(tx.execute("INSERT INTO invoice_setup(profile_id,revision,json) VALUES(?1,?2,?3) ON CONFLICT(profile_id) DO UPDATE SET revision=excluded.revision,json=excluded.json,draft_json=NULL",params![profile,revision,encoded]))?;
            db(tx.execute("INSERT INTO preference(key,value,updated_at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![format!("invoice.root.{profile}"),setup.root,chrono::Utc::now().timestamp()]))?;
        } else {
            db(tx.execute("INSERT INTO invoice_setup(profile_id,draft_json) VALUES(?1,?2) ON CONFLICT(profile_id) DO UPDATE SET draft_json=excluded.draft_json",params![profile,encoded]))?;
        }
        db(tx.commit())?;
        self.invoice_setup(profile)
    }
    pub(super) fn resumable_invoice_preview(&self, profile: i64) -> Result<Option<InvoicePreview>> {
        let text: Option<String> = db(self
            .store()
            .connection()
            .query_row(
                "SELECT json FROM invoice_draft WHERE profile_id=?1",
                [profile],
                |r| r.get(0),
            )
            .optional())?;
        let Some(text) = text else {
            return Ok(None);
        };
        let mut draft: InvoicePreview = parse(&text)?;
        if !draft.complete || draft.limited || self.validate_invoice_scope(&draft).is_err() {
            return Ok(None);
        }
        self.enrich_invoice_rows(&mut draft)?;
        Ok(Some(draft))
    }
    pub(super) fn enrich_invoice_reviews(&self, draft: &mut InvoicePreview) -> Result<()> {
        let setup = self
            .require_invoice_setup(draft.request.profile)?
            .confirmed
            .unwrap();
        for row in &mut draft.rows {
            row.eligibility = eligibility(&setup, row);
            row.review = if let Some(hash) = &row.digest {
                let value: Option<String> = db(self
                    .store()
                    .connection()
                    .query_row(
                        "SELECT json FROM invoice_review WHERE profile_id=?1 AND digest=?2",
                        params![draft.request.profile, hash],
                        |r| r.get(0),
                    )
                    .optional())?;
                value.map(|s| parse(&s)).transpose()?
            } else {
                None
            };
        }
        Ok(())
    }
    /// Persist an explicit edit before the UI treats it as saved. No mail or
    /// export is touched. Empty/reset removes the current correction, not history.
    pub fn save_invoice_review(
        &self,
        token: &str,
        index: usize,
        mut review: Option<InvoiceReview>,
    ) -> Result<InvoicePreview> {
        let mut draft = self.invoice_draft(token)?;
        self.validate_invoice_scope(&draft)?;
        if !draft.complete || draft.limited {
            return Err(reject("Finish the preview first."));
        }
        let row = draft
            .rows
            .get(index)
            .ok_or_else(|| reject("Unknown preview row."))?;
        if row.issue.is_some() || row.previous_path.is_some() {
            return Err(reject("This document cannot be edited."));
        }
        let hash = row
            .digest
            .as_ref()
            .ok_or_else(|| reject("No attachment digest."))?;
        let attachment = row
            .attachment
            .as_ref()
            .ok_or_else(|| reject("No attachment."))?;
        if digest(
            &self
                .attachment(row.account, row.message, attachment.index)?
                .bytes,
        ) != *hash
        {
            return Err(reject("An attachment changed. Start a new preview."));
        }
        if let Some(r) = &mut review {
            if !["include", "exclude", "review"].contains(&r.decision.as_str())
                || !["invoice", "credit"].contains(&r.kind.as_str())
            {
                return Err(reject("Invalid correction."));
            }
            if !r.invoice_date.is_empty() {
                date(&r.invoice_date)?;
            }
            if r.decision == "include"
                && (r.invoice_date.is_empty()
                    || r.invoice_date < draft.request.from
                    || r.invoice_date > draft.request.to)
            {
                return Err(reject(
                    "The document date must be inside the selected accounting period.",
                ));
            }
            r.reason = r.reason.trim().into();
            if r.reason.chars().count() > 500 {
                return Err(reject("Keep the correction reason below 500 characters."));
            }
            r.setup_revision = draft.setup_revision;
            r.updated_at = chrono::Utc::now().timestamp();
        }
        let conn = self.store().connection();
        let tx = db(conn.unchecked_transaction())?;
        if let Some(r) = &review {
            db(tx.execute("INSERT INTO invoice_review(profile_id,digest,json) VALUES(?1,?2,?3) ON CONFLICT(profile_id,digest) DO UPDATE SET json=excluded.json",params![draft.request.profile,hash,json(r)?]))?;
        } else {
            db(tx.execute(
                "DELETE FROM invoice_review WHERE profile_id=?1 AND digest=?2",
                params![draft.request.profile, hash],
            ))?;
        }
        db(tx.execute("INSERT INTO invoice_review_event(profile_id,digest,json,created_at) VALUES(?1,?2,?3,?4)",params![draft.request.profile,hash,json(&review)?,chrono::Utc::now().timestamp()]))?;
        db(tx.commit())?;
        self.enrich_invoice_rows(&mut draft)?;
        self.save_invoice_draft(&draft)?;
        Ok(draft)
    }
}
