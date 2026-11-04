//! Documents posted on paper, through LetterXpress.
//!
//! A PDF document — a letter the assistant wrote with its address in the
//! envelope window, or a signed form — goes to LetterXpress, which prints
//! it, puts it in a window envelope and hands it to Deutsche Post. The
//! settings are one for the whole store: the LetterXpress user name in a
//! preference, the API key in the keychain, and whether letters are printed
//! at all. They are not until the person says so: in test mode a letter
//! waits in the Postbox on the LetterXpress site and costs nothing.
//!
//! Posting is the person's to do and nobody else's. Nothing here is offered
//! to the assistant; the window asks first, with the price LetterXpress
//! quoted, and sends only what the person confirmed.
//!
//! The network half is in free functions over a [`LetterOrder`], not in
//! `Core`, for the reason sync is: a call to LetterXpress is held as long as
//! the network takes, and the store must not be locked while it is. So a
//! send is three steps for the caller — make the order, write the letter
//! down as `sending` ([`Core::begin_letter`]), then ask LetterXpress and
//! record what it said — and a crash between the last two leaves a row that
//! says a letter may have gone, which is the truth.

use core_letter::{Balance, Credentials, Job, Letter, LetterError, LetterXpress, Mode, Spec};
use core_store::model::AccountId;
use core_store::{LetterJob, NewLetter, StoredLetter};
use serde::{Deserialize, Serialize};

use crate::{pdf, Core, Result, RpcError};

pub use core_letter::Registered;

const USERNAME: &str = "letter.username";
const MODE: &str = "letter.mode";
const SENDER: &str = "letter.sender";
/// The keychain entry's name, made once per store: two stores on one
/// computer must not share one API key by accident, which a fixed name would
/// have them do.
const KEY_NAME: &str = "letter.key_name";

/// How long LetterXpress lets a job be taken back.
pub const CANCEL_WINDOW_SECS: i64 = 15 * 60;

/// How far a page may be off A4 and still be printed as A4: what a scan or
/// a rounding in someone's PDF writer comes to, not a different size.
const A4_SLACK_MM: f32 = 3.0;

/// The settings, as a settings form shows them. The key never crosses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetterSettings {
    pub username: Option<String>,
    pub has_key: bool,
    /// `test` until the person switches to `live`.
    pub mode: String,
    /// The return address last given by hand, to offer again.
    pub sender: Option<String>,
}

/// An address given by hand, for a PDF that does not carry one where the
/// envelope window is, or carries the wrong one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LetterAddress {
    /// One line each, at most six.
    pub recipient: String,
    #[serde(default)]
    pub sender: Option<String>,
    /// On a sheet of its own in front, rather than over the first page's
    /// address field.
    #[serde(default)]
    pub cover: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LetterSettingsInput {
    pub username: String,
    pub mode: String,
}

/// A letter as the window shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetterView {
    pub id: i64,
    pub account_id: AccountId,
    pub document_id: Option<i64>,
    pub name: String,
    pub mode: String,
    pub spec: Spec,
    pub pages: i64,
    pub quoted: Option<f64>,
    /// `sending`, `sent`, `failed` or `cancelled`.
    pub state: String,
    pub job_id: Option<i64>,
    pub job_status: Option<String>,
    pub address: Option<String>,
    pub cost: Option<f64>,
    pub tracking_code: Option<String>,
    pub tracking_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Until when it can still be taken back, while it can.
    pub cancellable_until: Option<i64>,
}

/// What sending a document would be, before it is sent.
#[derive(Debug, Clone, Serialize)]
pub struct LetterQuote {
    pub name: String,
    pub pages: usize,
    pub mode: String,
    /// What LetterXpress quotes for it.
    pub price: f64,
    pub balance: Option<Balance>,
    /// Letters already sent from this document, so posting it twice is a
    /// choice and not an accident.
    pub sent_before: Vec<LetterView>,
}

/// A document ready to go: the bytes, how, and who to send it with.
pub struct LetterOrder {
    pub account: AccountId,
    pub document_id: i64,
    pub name: String,
    pub pdf: Vec<u8>,
    pub pages: usize,
    pub spec: Spec,
    pub client: LetterXpress,
    pub sent_before: Vec<LetterView>,
    /// The return address given by hand, kept for the next letter.
    pub sender: Option<String>,
}

impl LetterOrder {
    pub fn mode(&self) -> Mode {
        self.client.mode()
    }
}

fn reject(message: impl Into<String>) -> RpcError {
    RpcError::Rejected(message.into())
}

pub(crate) fn letter_error(err: LetterError) -> RpcError {
    match err {
        LetterError::Network(message) => RpcError::Network(message),
        LetterError::Auth => RpcError::Auth(err.to_string()),
        other => RpcError::Rejected(other.to_string()),
    }
}

/// Whether every page is A4 upright, which is all LetterXpress prints: or
/// which page is not, and what it is instead.
pub fn check_a4(pdf_bytes: &[u8]) -> std::result::Result<usize, String> {
    let sizes = pdf::page_sizes_mm(pdf_bytes);
    if sizes.is_empty() {
        return Err("this PDF has no pages kuverta can read".into());
    }
    for (at, (w, h)) in sizes.iter().enumerate() {
        let a4 = (w - 210.0).abs() <= A4_SLACK_MM && (h - 297.0).abs() <= A4_SLACK_MM;
        if !a4 {
            return Err(format!(
                "page {} is {:.0} × {:.0} mm; LetterXpress prints A4 upright only (210 × 297 mm)",
                at + 1,
                w,
                h
            ));
        }
    }
    Ok(sizes.len())
}

fn view(letter: StoredLetter) -> LetterView {
    let open = letter.state == "sent"
        && !matches!(letter.job_status.as_deref(), Some("done" | "canceled"));
    let cancellable_until = (open && letter.job_id.is_some())
        .then_some(letter.created_at + CANCEL_WINDOW_SECS)
        .filter(|until| *until > crate::now_utc());
    LetterView {
        id: letter.id,
        account_id: letter.account_id,
        document_id: letter.document_id,
        name: letter.name,
        mode: letter.mode,
        spec: serde_json::from_str(&letter.spec).unwrap_or_default(),
        pages: letter.pages,
        quoted: letter.quoted,
        state: letter.state,
        job_id: letter.job_id,
        job_status: letter.job_status,
        address: letter.address,
        cost: letter.cost,
        tracking_code: letter.tracking_code,
        tracking_status: letter.tracking_status,
        last_error: letter.last_error,
        created_at: letter.created_at,
        updated_at: letter.updated_at,
        cancellable_until,
    }
}

impl Core {
    pub fn letter_settings(&self) -> Result<LetterSettings> {
        let has_key = match self.store().preference(KEY_NAME)? {
            Some(name) => key_entry(&name)
                .peek()
                .map_err(|err| RpcError::Auth(err.to_string()))?
                .is_some(),
            None => false,
        };
        Ok(LetterSettings {
            username: self
                .store()
                .preference(USERNAME)?
                .filter(|name| !name.is_empty()),
            has_key,
            mode: self.letter_mode()?.as_str().to_string(),
            sender: self.store().preference(SENDER)?.filter(|s| !s.is_empty()),
        })
    }

    pub fn save_letter_settings(&self, input: &LetterSettingsInput) -> Result<LetterSettings> {
        let mode = Mode::parse(&input.mode)
            .ok_or_else(|| reject(format!("{} is neither test nor live", input.mode)))?;
        self.store()
            .set_preference(USERNAME, input.username.trim())?;
        self.store().set_preference(MODE, mode.as_str())?;
        tracing::info!(mode = mode.as_str(), "letter settings saved");
        self.letter_settings()
    }

    pub fn set_letter_key(&self, key: &str) -> Result<()> {
        let key = key.trim();
        if key.is_empty() {
            return Err(reject("the API key is empty"));
        }
        let name = match self.store().preference(KEY_NAME)? {
            Some(name) => name,
            None => {
                let name = crate::memory::new_key();
                self.store().set_preference(KEY_NAME, &name)?;
                name
            }
        };
        key_entry(&name)
            .store(key)
            .map_err(|err| RpcError::Auth(err.to_string()))
    }

    pub fn clear_letter_key(&self) -> Result<()> {
        if let Some(name) = self.store().preference(KEY_NAME)? {
            let _ = key_entry(&name).delete();
        }
        Ok(())
    }

    fn letter_mode(&self) -> Result<Mode> {
        Ok(self
            .store()
            .preference(MODE)?
            .as_deref()
            .and_then(Mode::parse)
            .unwrap_or(Mode::Test))
    }

    /// A LetterXpress client with the stored credentials, or what is missing.
    pub fn letter_client(&self) -> Result<LetterXpress> {
        const SET_UP: &str = "LetterXpress is not set up: add the user name and API key under Settings → General → Letters by post";
        let username = self
            .store()
            .preference(USERNAME)?
            .filter(|name| !name.is_empty())
            .ok_or_else(|| reject(SET_UP))?;
        let name = self
            .store()
            .preference(KEY_NAME)?
            .ok_or_else(|| reject(SET_UP))?;
        let api_key = key_entry(&name)
            .peek()
            .map_err(|err| RpcError::Auth(err.to_string()))?
            .ok_or_else(|| reject(SET_UP))?;
        Ok(LetterXpress::new(Credentials {
            username,
            api_key,
            mode: self.letter_mode()?,
        }))
    }

    /// A document, checked for what LetterXpress can print, with the client
    /// to send it with.
    pub fn letter_order(
        &self,
        account: AccountId,
        document_id: i64,
        spec: Spec,
        address: Option<&LetterAddress>,
    ) -> Result<LetterOrder> {
        let client = self.letter_client()?;
        self.letter_order_with(account, document_id, spec, address, client)
    }

    /// The same, with a client given: for tests.
    pub fn letter_order_with(
        &self,
        account: AccountId,
        document_id: i64,
        spec: Spec,
        address: Option<&LetterAddress>,
        client: LetterXpress,
    ) -> Result<LetterOrder> {
        spec.check().map_err(letter_error)?;
        let (name, pdf, pages) = self.letter_pdf(account, document_id, &spec, address)?;
        let sent_before = self.letters_of_document(document_id)?;
        Ok(LetterOrder {
            account,
            document_id,
            name,
            pdf,
            pages,
            spec,
            client,
            sent_before,
            sender: address.and_then(|a| a.sender.clone()),
        })
    }

    /// A document as it would go in the envelope: as it is, or with the
    /// address given by hand put where the window shows it. Its name, its
    /// bytes and how many pages LetterXpress will print.
    pub fn letter_pdf(
        &self,
        account: AccountId,
        document_id: i64,
        spec: &Spec,
        address: Option<&LetterAddress>,
    ) -> Result<(String, Vec<u8>, usize)> {
        let file = self.document(document_id)?;
        if file.view.account_id != account {
            return Err(reject(format!("there is no document {document_id} here")));
        }
        if file.view.content_type != "application/pdf" {
            return Err(reject(format!(
                "{} is not a PDF; only a PDF can go by post",
                file.view.name
            )));
        }
        check_a4(&file.bytes).map_err(reject)?;
        let pdf = match address {
            None => file.bytes,
            Some(address) => {
                let window = pdf::Window {
                    recipient: &address.recipient,
                    sender: address.sender.as_deref(),
                    registered: spec.registered.is_some(),
                };
                let at = if address.cover {
                    pdf::AddressAt::Cover {
                        blank_back: spec.duplex,
                    }
                } else {
                    pdf::AddressAt::FirstPage
                };
                pdf::address(&file.bytes, &window, at).map_err(reject)?
            }
        };
        let pages = check_a4(&pdf).map_err(reject)?;
        Ok((file.view.name, pdf, pages))
    }

    /// Writes the letter down as on its way, before LetterXpress is asked.
    pub fn begin_letter(&self, order: &LetterOrder, quoted: Option<f64>) -> Result<i64> {
        if let Some(sender) = order.sender.as_deref().map(str::trim) {
            self.store().set_preference(SENDER, sender)?;
        }
        let spec = serde_json::to_string(&order.spec).expect("a spec is plain data");
        let id = self.store().begin_letter(&NewLetter {
            account_id: order.account,
            document_id: Some(order.document_id),
            name: &order.name,
            mode: order.mode().as_str(),
            spec: &spec,
            pages: order.pages as i64,
            quoted,
        })?;
        tracing::info!(
            id,
            document = order.document_id,
            mode = order.mode().as_str(),
            pages = order.pages,
            "posting a letter"
        );
        Ok(id)
    }

    /// What LetterXpress said about a letter's job.
    pub fn record_letter(&self, id: i64, job: &Job) -> Result<LetterView> {
        let (tracking_code, tracking_status) = job.tracking();
        let cost = job.total();
        self.store().record_letter_job(
            id,
            &LetterJob {
                job_id: job.id,
                job_status: &job.status,
                address: job.address(),
                cost: (cost > 0.0).then_some(cost),
                tracking_code,
                tracking_status,
            },
        )?;
        tracing::info!(id, job = job.id, status = %job.status, "letter recorded");
        self.letter_view(id)
    }

    pub fn letter_failed(&self, id: i64, error: &str) -> Result<LetterView> {
        self.store().letter_failed(id, error)?;
        tracing::warn!(id, error, "letter not posted");
        self.letter_view(id)
    }

    pub fn letter_cancelled(&self, id: i64) -> Result<LetterView> {
        self.store().letter_cancelled(id)?;
        tracing::info!(id, "letter taken back");
        self.letter_view(id)
    }

    pub fn letter_view(&self, id: i64) -> Result<LetterView> {
        self.store()
            .letter(id)?
            .map(view)
            .ok_or_else(|| reject(format!("there is no letter {id}")))
    }

    pub fn letters(&self, account: AccountId) -> Result<Vec<LetterView>> {
        Ok(self
            .store()
            .letters(account)?
            .into_iter()
            .map(view)
            .collect())
    }

    pub fn letters_of_document(&self, document_id: i64) -> Result<Vec<LetterView>> {
        Ok(self
            .store()
            .letters_of_document(document_id)?
            .into_iter()
            .map(view)
            .collect())
    }

    /// A letter's job id, with a client to ask after it: what refreshing
    /// and taking back need. A letter in test mode is asked about in test
    /// mode, whatever the setting is now, or LetterXpress would not find it.
    pub fn letter_job(&self, id: i64) -> Result<(LetterXpress, i64)> {
        let letter = self.letter_view(id)?;
        let job = letter.job_id.ok_or_else(|| {
            reject("LetterXpress never took this letter, so there is nothing to ask about")
        })?;
        let client = self.letter_client()?;
        let mode = Mode::parse(&letter.mode).unwrap_or(Mode::Test);
        Ok((client.with_mode(mode), job))
    }
}

fn key_entry(name: &str) -> core_accounts::KeychainPassword {
    core_accounts::KeychainPassword::new(format!("letter:{name}"))
}

/// What sending it would cost, and what the account holds.
pub async fn quote(order: &LetterOrder) -> Result<LetterQuote> {
    let price = order
        .client
        .price(&order.spec, order.pages)
        .await
        .map_err(letter_error)?;
    // What the account holds is good to know, not needed to send.
    let balance = order.client.balance().await.ok();
    Ok(LetterQuote {
        name: order.name.clone(),
        pages: order.pages,
        mode: order.mode().as_str().to_string(),
        price,
        balance,
        sent_before: order.sent_before.clone(),
    })
}

/// Sends it. The caller has written it down with [`Core::begin_letter`].
pub async fn submit(order: &LetterOrder) -> std::result::Result<Job, LetterError> {
    order
        .client
        .submit(&Letter {
            pdf: &order.pdf,
            spec: order.spec,
            filename: Some(&order.name),
            notice: Some("kuverta"),
        })
        .await
}

/// Whether the credentials work: what the account holds, if they do.
pub async fn check(client: &LetterXpress) -> Result<Balance> {
    client.balance().await.map_err(letter_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object};

    #[test]
    fn only_a4_upright_is_printed() {
        let written = pdf::write("x", "Text", None, false, None, None).unwrap();
        assert_eq!(check_a4(&written.pdf), Ok(1));

        // The same page in US Letter.
        let mut doc = Document::load_mem(&written.pdf).unwrap();
        let page = doc.get_pages()[&1];
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "MediaBox",
                vec![0.into(), 0.into(), Object::Real(612.0), Object::Real(792.0)],
            );
        let mut letter = Vec::new();
        doc.save_to(&mut letter).unwrap();
        let err = check_a4(&letter).unwrap_err();
        assert!(err.contains("page 1 is 216 × 279 mm"), "{err}");

        assert!(check_a4(b"not a pdf").is_err());
    }
}
