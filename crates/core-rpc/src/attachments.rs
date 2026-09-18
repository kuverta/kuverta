//! Attachments: what a message carries besides its text.
//!
//! The raw message is on disk from the sync, so an attachment is a part of it
//! read back on demand: listed when the message opens, its bytes handed over
//! when the window shows, saves or opens it. Nothing is extracted ahead of
//! time and nothing is stored twice.
//!
//! For encrypted mail the attachments are inside the encryption, so they are
//! listed from what decrypting yields — in memory, for the one request, like
//! the decrypted text. Mail that cannot be decrypted lists none: its only
//! part is the ciphertext.
//!
//! An attachment is written by whoever sent the mail. Its name is theirs, so
//! it is made safe to use as a file name; its type is theirs, so the bytes are
//! looked at when the declared type says nothing; and a file that would run a
//! program when opened — or a web page, which would open in a browser with
//! whatever it asks for — is marked **risky**: the window offers to save it,
//! never to open it.

use mail_parser::{Message as MimeMessage, MessageParser, MessagePart, MimeHeaders};
use serde::{Deserialize, Serialize};

use core_store::model::{AccountId, MessageId};

use crate::{Core, Result, RpcError};

/// One attachment, as the reading pane lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentView {
    /// Its position among the message's attachments: what
    /// [`Core::attachment`] takes.
    pub index: usize,
    /// Safe to use as a file name: no directories, no control characters.
    pub name: String,
    /// `type/subtype`, lowercase. The declared type, or — when the sender's
    /// program declared nothing useful — what the bytes are.
    pub content_type: String,
    pub size: usize,
    /// Part of the text rather than attached for its own sake: an image the
    /// message shows inline, a logo in a signature.
    pub inline: bool,
    /// What the window can show itself: `image`, `pdf`, `text` or `none`.
    pub preview: String,
    /// Opening it could run something. Saved, never opened.
    pub risky: bool,
}

/// An attachment's bytes, with what to call them.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub view: AttachmentView,
    pub bytes: Vec<u8>,
}

/// The attachments of a parsed message, in order, each with its part.
///
/// The one place that decides what counts, so that listing and fetching can
/// never disagree about which index is which.
fn attachment_parts<'a>(parsed: &'a MimeMessage<'a>) -> Vec<&'a MessagePart<'a>> {
    parsed
        .attachments
        .iter()
        .filter_map(|id| {
            let part = parsed.parts.get(*id as usize)?;
            if part.is_multipart() {
                return None;
            }
            let content_type = declared_type(part);
            // OpenPGP's own structure: shown as a lock or a seal, not a file.
            if content_type == "application/pgp-signature"
                || content_type == "application/pgp-encrypted"
            {
                return None;
            }
            // A text part without a name that is also one of the bodies is
            // already on screen as the text.
            let is_body = parsed.text_body.contains(id) || parsed.html_body.contains(id);
            if is_body && part.attachment_name().is_none() && !disposed(part, "attachment") {
                return None;
            }
            Some(part)
        })
        .collect()
}

/// Whether the part's Content-Disposition is `kind`.
fn disposed(part: &MessagePart<'_>, kind: &str) -> bool {
    part.content_disposition()
        .is_some_and(|cd| cd.ctype().eq_ignore_ascii_case(kind))
}

fn declared_type(part: &MessagePart<'_>) -> String {
    if part.is_message() {
        return "message/rfc822".into();
    }
    part.content_type()
        .map(|ct| match ct.subtype() {
            Some(subtype) => format!("{}/{}", ct.ctype(), subtype),
            None => ct.ctype().to_string(),
        })
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// What the bytes are, for the few types that matter to the window.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if starts(b"%PDF") {
        Some("application/pdf")
    } else if starts(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if starts(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && starts(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A type for a name, when neither the sender nor the bytes said.
fn type_for_name(name: &str) -> Option<&'static str> {
    let ext = extension(name)?;
    Some(match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "text" | "log" => "text/plain",
        "csv" => "text/csv",
        "ics" => "text/calendar",
        "vcf" => "text/vcard",
        "eml" => "message/rfc822",
        _ => return None,
    })
}

fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 12).then(|| ext.to_ascii_lowercase())
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "application/pdf" => ".pdf",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "text/plain" => ".txt",
        "text/html" => ".html",
        "text/calendar" => ".ics",
        "text/csv" => ".csv",
        "text/vcard" | "text/x-vcard" => ".vcf",
        "message/rfc822" => ".eml",
        _ => ".bin",
    }
}

/// Extensions that run something, or open as a web page, when opened.
const RISKY_EXTENSIONS: &[&str] = &[
    // Programs, installers and scripts.
    "exe",
    "msi",
    "msix",
    "appx",
    "bat",
    "cmd",
    "com",
    "scr",
    "pif",
    "cpl",
    "dll",
    "sys",
    "vbs",
    "vbe",
    "js",
    "jse",
    "mjs",
    "wsf",
    "wsh",
    "ps1",
    "psm1",
    "psd1",
    "hta",
    "jar",
    "app",
    "command",
    "tool",
    "sh",
    "bash",
    "zsh",
    "csh",
    "fish",
    "pkg",
    "mpkg",
    "dmg",
    "apk",
    "deb",
    "rpm",
    "appimage",
    "run",
    "bin",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "scpt",
    "applescript",
    "workflow",
    "action",
    "terminal",
    "desktop",
    "lnk",
    "url",
    "webloc",
    "inetloc",
    "reg",
    "msc",
    "gadget",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "chm",
    "inf",
    "ins",
    "isp",
    "settingcontent-ms",
    "library-ms",
    "xll",
    "xlam",
    "ppam",
    "docm",
    "dotm",
    "xlsm",
    "xltm",
    "pptm",
    "potm",
    "one",
    // Web pages: they open in a browser, and a page in a mail is how a fake
    // sign-in form arrives.
    "html",
    "htm",
    "xhtml",
    "shtml",
    "svg",
    "mht",
    "mhtml",
    "xml",
    "xsl",
];

const RISKY_TYPES: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "image/svg+xml",
    "application/x-msdownload",
    "application/x-msdos-program",
    "application/x-executable",
    "application/x-sh",
    "application/x-shellscript",
    "application/javascript",
    "text/javascript",
    "application/java-archive",
    "application/x-apple-diskimage",
    "application/vnd.microsoft.portable-executable",
];

fn is_risky(name: &str, content_type: &str) -> bool {
    RISKY_TYPES.contains(&content_type)
        || extension(name).is_some_and(|ext| RISKY_EXTENSIONS.contains(&ext.as_str()))
}

fn preview_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => "image",
        "application/pdf" => "pdf",
        "text/plain" | "text/csv" | "text/calendar" | "text/vcard" | "text/x-vcard" => "text",
        _ => "none",
    }
}

/// A name the sender chose, made safe to write to disk: the last path
/// component, no control or reserved characters, no leading dots, not too
/// long — with the extension kept, since that is what opens it.
pub fn safe_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    if cleaned.is_empty() {
        return "attachment".into();
    }
    const MAX: usize = 120;
    if cleaned.chars().count() <= MAX {
        return cleaned.to_string();
    }
    let ext = extension(cleaned)
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let stem: String = cleaned.chars().take(MAX - ext.chars().count()).collect();
    format!("{stem}{ext}")
}

fn describe(index: usize, part: &MessagePart<'_>) -> AttachmentView {
    let bytes = part.contents();
    let declared = declared_type(part);
    let given = part.attachment_name().map(str::to_string).or_else(|| {
        part.message()
            .and_then(|m| m.subject())
            .map(|s| format!("{s}.eml"))
    });
    let vague = declared.is_empty() || declared == "application/octet-stream";
    let content_type = match (vague, sniff(bytes)) {
        (_, Some(sniffed)) if vague || declared.starts_with("image/") => sniffed.to_string(),
        (true, None) => given
            .as_deref()
            .and_then(type_for_name)
            .unwrap_or("application/octet-stream")
            .to_string(),
        _ => declared,
    };
    let name =
        safe_file_name(&given.unwrap_or_else(|| {
            format!("attachment-{}{}", index + 1, extension_for(&content_type))
        }));
    AttachmentView {
        index,
        risky: is_risky(&name, &content_type),
        preview: preview_for(&content_type).into(),
        inline: disposed(part, "inline")
            || (part.content_id().is_some() && !disposed(part, "attachment")),
        size: bytes.len(),
        name,
        content_type,
    }
}

/// Lists a parsed message's attachments.
pub fn list(parsed: &MimeMessage<'_>) -> Vec<AttachmentView> {
    attachment_parts(parsed)
        .into_iter()
        .enumerate()
        .map(|(index, part)| describe(index, part))
        .collect()
}

/// Whether a message is PGP/MIME encrypted: its attachments are then inside
/// the encryption, and the outer parts are only the ciphertext.
pub(crate) fn is_encrypted(parsed: &MimeMessage<'_>) -> bool {
    parsed.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case("multipart")
            && ct
                .subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("encrypted"))
    })
}

/// A message's attachments, given the parse of the stored message and what
/// opening it found.
pub(crate) fn list_opened(
    parsed: &MimeMessage<'_>,
    opened: &core_pgp::OpenedMessage,
) -> Vec<AttachmentView> {
    match &opened.decrypted {
        Some(inner) => MessageParser::default()
            .parse(inner)
            .map(|inner| list(&inner))
            .unwrap_or_default(),
        None if is_encrypted(parsed) => Vec::new(),
        None => list(parsed),
    }
}

impl Core {
    /// One attachment of a message, with its bytes.
    pub fn attachment(
        &self,
        account: AccountId,
        id: MessageId,
        index: usize,
    ) -> Result<Attachment> {
        let stored = self
            .store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;
        let path = stored.body_path.as_deref().ok_or_else(|| {
            RpcError::Rejected(format!("message {id} was stored without its body"))
        })?;
        let raw = self
            .blobs
            .get(path)
            .map_err(|e| RpcError::Rejected(format!("reading the stored body of {id}: {e}")))?;
        let parsed = MessageParser::default().parse(&raw).ok_or_else(|| {
            RpcError::Rejected(format!("the stored body of {id} is not a message"))
        })?;

        let opened = if is_encrypted(&parsed) {
            core_pgp::open_parsed(&parsed, self.keyring.as_ref())
        } else {
            core_pgp::OpenedMessage::default()
        };
        let inner = match &opened.decrypted {
            Some(bytes) => Some(MessageParser::default().parse(bytes).ok_or_else(|| {
                RpcError::Rejected("the decrypted message could not be read".into())
            })?),
            None if is_encrypted(&parsed) => {
                return Err(RpcError::Rejected(
                    "this message is encrypted and could not be decrypted".into(),
                ))
            }
            None => None,
        };
        let source = inner.as_ref().unwrap_or(&parsed);
        let part = attachment_parts(source)
            .into_iter()
            .nth(index)
            .ok_or_else(|| {
                RpcError::Rejected(format!("message {id} has no attachment {}", index + 1))
            })?;
        Ok(Attachment {
            view: describe(index, part),
            bytes: part.contents().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_senders_are_made_safe() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Users\\x\\esim.png"), "esim.png");
        assert_eq!(safe_file_name(".hidden"), "hidden");
        assert_eq!(safe_file_name("a\u{0}b:c.pdf"), "a_b_c.pdf");
        assert_eq!(safe_file_name("  "), "attachment");
        let long = format!("{}.pdf", "x".repeat(300));
        let safe = safe_file_name(&long);
        assert_eq!(safe.chars().count(), 120);
        assert!(safe.ends_with(".pdf"));
    }

    #[test]
    fn programs_and_pages_are_risky_documents_are_not() {
        assert!(is_risky("invoice.pdf.exe", "application/octet-stream"));
        assert!(is_risky("login.html", "text/html"));
        assert!(is_risky("x", "text/html"));
        assert!(is_risky("Rechnung.docm", "application/octet-stream"));
        assert!(!is_risky("esim.png", "image/png"));
        assert!(!is_risky("Rechnung.pdf", "application/pdf"));
        assert!(!is_risky("fotos.zip", "application/zip"));
    }
}
