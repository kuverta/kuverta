//! Documents the assistant makes, and the signature it signs them with.
//!
//! A document is a PDF kept on an account: written from text the assistant
//! composed, or an attachment — or an earlier document — with the person's
//! signature put on it. The person sees each as a card in the chat, opens it
//! in the viewer, saves it, attaches it to a message, or throws it away; the
//! assistant lists, reads, renames and deletes them by id.
//!
//! The signature is a PNG or JPEG of the person's handwritten signature, kept
//! once for the whole store under Settings → General, with the place written
//! in front of the date beneath it. Signing is placing that picture: what a
//! printed, signed and scanned page is, not a certificate. It happens only
//! when the person asks for it — the assistant is told so, and the tool says
//! which document it signed — and the signed PDF goes nowhere by itself: the
//! assistant cannot send mail.

use base64::Engine;
use serde::{Deserialize, Serialize};

use core_store::documents::NewDocument;
use core_store::model::AccountId;

use crate::attachments::safe_file_name;
use crate::pdf::{self, Pixels, Placement};
use crate::{Core, Result, RpcError};

/// A document as the window and the assistant see it: everything but the
/// bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentView {
    pub id: i64,
    pub account_id: AccountId,
    pub name: String,
    pub note: Option<String>,
    pub signed: bool,
    pub pages: Option<i64>,
    pub size: i64,
    pub created_at: i64,
    /// Always `application/pdf`, for the viewer, which takes files by type.
    pub content_type: String,
}

/// A document with its bytes.
#[derive(Debug, Clone)]
pub struct DocumentFile {
    pub view: DocumentView,
    pub bytes: Vec<u8>,
}

/// What a signature goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSource {
    Attachment { message: i64, index: usize },
    Document { id: i64 },
}

/// The signature as settings show it: whether there is one, its measurements,
/// the place, and — for the preview — the picture, as base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureSettings {
    pub present: bool,
    pub content_type: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub place: Option<String>,
    pub image: Option<String>,
}

/// The most a signature picture may be, on disk and on a side. A signature
/// is a few hundred pixels across; a photograph of one is cropped first.
const MOST_SIGNATURE_BYTES: usize = 4 * 1024 * 1024;
const MOST_SIGNATURE_SIDE: u32 = 4000;
/// The most text one written document takes.
const MOST_TEXT_CHARS: usize = 60_000;

fn reject(message: impl Into<String>) -> RpcError {
    RpcError::Rejected(message.into())
}

/// A file name for a document: safe, and ending in `.pdf`.
pub fn document_name(name: &str) -> String {
    let safe = safe_file_name(name.trim());
    let safe = safe.trim_end_matches('.').to_string();
    if safe.to_lowercase().ends_with(".pdf") {
        safe
    } else {
        format!("{safe}.pdf")
    }
}

fn view(stored: core_store::documents::StoredDocument) -> DocumentView {
    DocumentView {
        id: stored.id,
        account_id: stored.account_id,
        name: stored.name,
        note: stored.note,
        signed: stored.signed,
        pages: stored.pages,
        size: stored.size,
        created_at: stored.created_at,
        content_type: "application/pdf".into(),
    }
}

impl Core {
    /// The account's documents, newest first.
    pub fn documents(&self, account: AccountId) -> Result<Vec<DocumentView>> {
        Ok(self
            .store()
            .documents(account)?
            .into_iter()
            .map(view)
            .collect())
    }

    pub fn document_view(&self, id: i64) -> Result<DocumentView> {
        self.store()
            .document(id)?
            .map(view)
            .ok_or_else(|| reject(format!("there is no document {id}")))
    }

    pub fn document(&self, id: i64) -> Result<DocumentFile> {
        let view = self.document_view(id)?;
        let bytes = self
            .store()
            .document_bytes(id)?
            .ok_or_else(|| reject(format!("there is no document {id}")))?;
        Ok(DocumentFile { view, bytes })
    }

    /// Writes a document from text. `sign` puts the signature where the text
    /// marks it with `[signature]`, or at the end.
    pub fn write_document(
        &self,
        account: AccountId,
        name: &str,
        text: &str,
        note: Option<&str>,
        sign: bool,
    ) -> Result<DocumentView> {
        if text.trim().is_empty() {
            return Err(reject("a document needs some text"));
        }
        if text.chars().count() > MOST_TEXT_CHARS {
            return Err(reject(format!(
                "that is more than one document holds: at most {MOST_TEXT_CHARS} characters"
            )));
        }
        let pixels = if sign { self.signature_pixels()? } else { None };
        if sign && pixels.is_none() {
            return Err(reject(NO_SIGNATURE));
        }
        let name = document_name(name);
        let title = name.trim_end_matches(".pdf");
        let caption = if sign {
            Some(self.signature_caption()?)
        } else {
            None
        };
        let written =
            pdf::write(title, text, pixels.as_ref(), sign, caption.as_deref()).map_err(reject)?;
        let id = self.store().add_document(&NewDocument {
            account_id: account,
            name: &name,
            note,
            signed: written.signed,
            pages: Some(written.pages as i64),
            pdf: &written.pdf,
        })?;
        tracing::info!(
            id,
            pages = written.pages,
            signed = written.signed,
            "wrote a document"
        );
        self.document_view(id)
    }

    /// Puts the signature on a copy of `source`, kept as a new document.
    /// `with_date` writes the place and today's date beneath it, unless the
    /// placement brings a caption of its own.
    pub fn sign(
        &self,
        account: AccountId,
        source: DocumentSource,
        mut placement: Placement,
        with_date: bool,
    ) -> Result<DocumentView> {
        let pixels = self
            .signature_pixels()?
            .ok_or_else(|| reject(NO_SIGNATURE))?;
        let (name, bytes, from) = match source {
            DocumentSource::Attachment { message, index } => {
                let attachment = self.attachment(account, message, index)?;
                if attachment.view.content_type != "application/pdf"
                    && !attachment.bytes.starts_with(b"%PDF")
                {
                    return Err(reject(format!(
                        "{} is {}, not a PDF; only a PDF can be signed",
                        attachment.view.name, attachment.view.content_type
                    )));
                }
                let detail = self.message(account, message)?;
                let from = detail.from.unwrap_or_default();
                (
                    attachment.view.name,
                    attachment.bytes,
                    format!(
                        "from the message “{}” by {from}",
                        detail.subject.unwrap_or_default()
                    ),
                )
            }
            DocumentSource::Document { id } => {
                let file = self.document(id)?;
                if file.view.account_id != account {
                    return Err(reject(format!("there is no document {id} on this account")));
                }
                (
                    file.view.name,
                    file.bytes,
                    "a document made here".to_string(),
                )
            }
        };
        if with_date && placement.caption.is_none() {
            placement.caption = Some(self.signature_caption()?);
        }
        let (signed, page) = pdf::stamp(&bytes, &pixels, &placement).map_err(reject)?;
        let stem = name.trim_end_matches(".pdf").trim_end_matches(".PDF");
        let stem = stem.strip_suffix(" (signed)").unwrap_or(stem);
        let signed_name = document_name(&format!("{stem} (signed)"));
        let pages = pdf::page_count(&signed).map(|n| n as i64);
        let id = self.store().add_document(&NewDocument {
            account_id: account,
            name: &signed_name,
            note: Some(&format!(
                "signed copy of {name}, {from}; signature on page {page}"
            )),
            signed: true,
            pages,
            pdf: &signed,
        })?;
        tracing::info!(id, page, "signed a PDF");
        self.document_view(id)
    }

    /// What is written beneath a signature: the place and today's date, the
    /// way a letter is signed, or the date alone without a place.
    pub fn signature_caption(&self) -> Result<String> {
        let place = self.store().signature()?.and_then(|s| s.place);
        let date = chrono::Local::now().format("%d.%m.%Y");
        Ok(match place {
            Some(place) => format!("{place}, {date}"),
            None => date.to_string(),
        })
    }

    /// A one-page PDF with the signature on it, for the settings to show
    /// what signing looks like. Not kept.
    pub fn signature_sample(&self) -> Result<Vec<u8>> {
        let pixels = self
            .signature_pixels()?
            .ok_or_else(|| reject(NO_SIGNATURE))?;
        let caption = self.signature_caption()?;
        let text = "# A signed page\n\nThis is how the assistant signs: the signature where the text puts it, the place and the date beneath, on a page it wrote or on one that came as an attachment.\n\nWith kind regards\n\n[signature]\nErika Mustermann";
        let written =
            pdf::write("Signature", text, Some(&pixels), true, Some(&caption)).map_err(reject)?;
        Ok(written.pdf)
    }

    pub fn rename_document(&self, id: i64, name: &str) -> Result<DocumentView> {
        let name = document_name(name);
        if !self.store().rename_document(id, &name)? {
            return Err(reject(format!("there is no document {id}")));
        }
        self.document_view(id)
    }

    pub fn delete_document(&self, id: i64) -> Result<()> {
        if !self.store().delete_document(id)? {
            return Err(reject(format!("there is no document {id}")));
        }
        Ok(())
    }

    // -- the signature ----------------------------------------------------

    pub fn signature(&self) -> Result<SignatureSettings> {
        let stored = self.cleaned_signature()?;
        Ok(match stored {
            Some(s) => SignatureSettings {
                present: s.image.is_some(),
                image: s
                    .image
                    .as_deref()
                    .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
                content_type: s.content_type,
                width: s.width,
                height: s.height,
                place: s.place,
            },
            None => SignatureSettings {
                present: false,
                content_type: None,
                width: None,
                height: None,
                place: None,
                image: None,
            },
        })
    }

    /// Keeps a picture of the signature: a PNG or a JPEG, cut out of its
    /// paper (see [`crate::signature`]) and kept as a PNG with the paper
    /// transparent — which is what the settings show and what goes on a
    /// page.
    pub fn set_signature(&self, bytes: &[u8], content_type: &str) -> Result<SignatureSettings> {
        if bytes.len() > MOST_SIGNATURE_BYTES {
            return Err(reject(format!(
                "the picture is too large: at most {} MB",
                MOST_SIGNATURE_BYTES / 1024 / 1024
            )));
        }
        let is_picture = matches!(
            content_type.trim().to_ascii_lowercase().as_str(),
            "image/png" | "image/jpeg" | "image/jpg"
        ) || bytes.starts_with(&[0x89, b'P', b'N', b'G'])
            || bytes.starts_with(&[0xff, 0xd8, 0xff]);
        if !is_picture {
            return Err(reject(format!(
                "{content_type} is not a picture of a signature: a PNG or a JPEG is"
            )));
        }
        let decoded = image::load_from_memory(bytes)
            .map_err(|err| reject(format!("the picture could not be read: {err}")))?;
        let (width, height) = (decoded.width(), decoded.height());
        if width == 0 || height == 0 || width > MOST_SIGNATURE_SIDE || height > MOST_SIGNATURE_SIDE
        {
            return Err(reject(format!(
                "the picture is {width} × {height} pixels; a signature is at most {MOST_SIGNATURE_SIDE} on a side"
            )));
        }
        let cleaned = crate::signature::clean(&decoded);
        if cleaned.pixels().all(|p| p[3] == 0) {
            return Err(reject(
                "no signature could be made out in the picture: it wants dark ink on light paper, photographed straight on",
            ));
        }
        let mut png = std::io::Cursor::new(Vec::new());
        cleaned
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|err| reject(format!("the signature could not be kept: {err}")))?;
        self.store().set_signature(
            png.get_ref(),
            "image/png",
            i64::from(cleaned.width()),
            i64::from(cleaned.height()),
        )?;
        tracing::info!(
            width = cleaned.width(),
            height = cleaned.height(),
            "kept a signature"
        );
        self.signature()
    }

    /// The same, from base64 — what the window has once it has read the
    /// file the person picked.
    pub fn set_signature_base64(
        &self,
        data: &str,
        content_type: &str,
    ) -> Result<SignatureSettings> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|err| reject(format!("the picture could not be read: {err}")))?;
        self.set_signature(&bytes, content_type)
    }

    pub fn set_signature_place(&self, place: Option<&str>) -> Result<()> {
        Ok(self.store().set_signature_place(place)?)
    }

    pub fn clear_signature(&self) -> Result<()> {
        self.store().clear_signature()?;
        Ok(())
    }

    /// The signature as pixels to draw, or `None` when there is none.
    fn signature_pixels(&self) -> Result<Option<Pixels>> {
        match self.cleaned_signature()?.and_then(|s| s.image) {
            Some(bytes) => decode(&bytes).map(Some),
            None => Ok(None),
        }
    }

    /// The stored signature, cut out of its paper. A picture kept by a
    /// kuverta before 0.5.37 is the photograph as it was chosen; it is
    /// cleaned the first time it is wanted and kept cleaned, so nobody has
    /// to choose it again.
    fn cleaned_signature(&self) -> Result<Option<core_store::documents::StoredSignature>> {
        let stored = self.store().signature()?;
        let Some(signature) = &stored else {
            return Ok(None);
        };
        let (Some(image), Some(content_type)) = (&signature.image, &signature.content_type) else {
            return Ok(stored);
        };
        if content_type == "image/png" {
            return Ok(stored);
        }
        tracing::info!(
            content_type,
            "cutting a signature kept as a photograph out of its paper"
        );
        self.set_signature(image, content_type)?;
        self.store().signature().map_err(Into::into)
    }
}

/// What to tell the person when there is no signature.
pub const NO_SIGNATURE: &str =
    "there is no signature yet: the person can add a picture of theirs under Settings → General → Signature";

/// A picture as pixels for the PDF: RGB, with the alpha channel when any of
/// it is not opaque.
fn decode(bytes: &[u8]) -> Result<Pixels> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|err| reject(format!("the picture could not be read: {err}")))?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 || width > MOST_SIGNATURE_SIDE || height > MOST_SIGNATURE_SIDE {
        return Err(reject(format!(
            "the picture is {width} × {height} pixels; a signature is at most {MOST_SIGNATURE_SIDE} on a side"
        )));
    }
    let rgba = decoded.into_rgba8();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    let mut alpha = Vec::with_capacity((width * height) as usize);
    let mut translucent = false;
    for pixel in rgba.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
        translucent |= pixel.0[3] != 255;
    }
    Ok(Pixels {
        width,
        height,
        rgb,
        alpha: translucent.then_some(alpha),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_end_in_pdf_once_and_are_safe() {
        assert_eq!(document_name("Kündigung"), "Kündigung.pdf");
        assert_eq!(document_name("Vertrag.pdf"), "Vertrag.pdf");
        assert_eq!(document_name("Vertrag.PDF"), "Vertrag.PDF");
        assert_eq!(document_name("../x/y.pdf"), "y.pdf");
        assert_eq!(document_name(""), "attachment.pdf");
        assert_eq!(document_name("a."), "a.pdf");
    }

    #[test]
    fn a_signature_kept_as_a_photograph_is_cleaned_when_first_wanted() {
        let store = core_store::Store::open_in_memory().unwrap();
        let mut photo = image::RgbaImage::from_pixel(200, 60, image::Rgba([170, 170, 170, 255]));
        for x in 20..180 {
            for y in 28..34 {
                photo.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(photo)
            .to_rgb8()
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        // As a kuverta before 0.5.37 kept it: the file as chosen.
        store
            .set_signature(jpeg.get_ref(), "image/jpeg", 200, 60)
            .unwrap();
        let core = Core::new(store, core_store::Blobs::new(std::env::temp_dir()));
        let settings = core.signature().unwrap();
        assert_eq!(settings.content_type.as_deref(), Some("image/png"));
        assert!(settings.width.unwrap() < 200, "{settings:?}");
        assert_eq!(
            core.store()
                .signature()
                .unwrap()
                .unwrap()
                .content_type
                .as_deref(),
            Some("image/png")
        );
    }

    #[test]
    fn a_png_with_transparency_keeps_its_alpha_and_an_opaque_one_does_not() {
        let mut opaque = image::RgbaImage::new(2, 1);
        opaque.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        opaque.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        opaque.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let pixels = decode(png.get_ref()).unwrap();
        assert_eq!((pixels.width, pixels.height), (2, 1));
        assert_eq!(pixels.rgb, vec![0, 0, 0, 255, 255, 255]);
        assert!(pixels.alpha.is_none());

        let mut clear = image::RgbaImage::new(1, 1);
        clear.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let mut png = std::io::Cursor::new(Vec::new());
        clear.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(decode(png.get_ref()).unwrap().alpha, Some(vec![0]));
        assert!(decode(b"not a picture").is_err());
    }
}
