//! Reading protected mail: PGP/MIME encrypted and signed, and inline PGP.
//!
//! Everything here is local CPU work on a message already on disk — parsing,
//! a session-key decryption, a hash — so it runs when a message is opened
//! rather than at sync time. Nothing decrypted is ever written back to the
//! store: the plaintext exists for as long as the reading pane shows it.
//!
//! What was found is reported rather than raised. A message whose signature
//! does not verify, or that cannot be decrypted, is still a message the
//! person wants to see the headers of, and "the signature is bad" or "no
//! passphrase is stored for key …" is the answer they need next to it.
//!
//! ## What a signature covers
//!
//! For `multipart/signed` the first part's bytes, exactly as they sit between
//! the boundaries, with line endings made CRLF — so those bytes are cut out
//! of the raw message here rather than rebuilt from a parse, which would
//! normalise exactly the things a signature is sensitive to. The text shown
//! is the signed part's, so anything a list server tacked on outside it is
//! not presented as signed.

use mail_parser::{Message as MimeMessage, MessageParser, MessagePart, MimeHeaders};
use pgp::composed::{CleartextSignedMessage, Deserializable, DetachedSignature, Esk, Message};
use pgp::packet::Signature;
use pgp::types::KeyDetails;
use serde::{Deserialize, Serialize};

use crate::hex;
use crate::keyring::{armored_blocks, email_of, primary_user_id, user_ids, Cert, Keyring};
use crate::outgoing::crlf;

/// What protection a message had, and what became of it.
///
/// `None` in a message detail means ordinary mail: nothing here applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SecurityView {
    /// `"pgp/mime"` or `"inline"`.
    pub format: String,
    pub encrypted: bool,
    /// Whether decryption worked. Only meaningful when `encrypted`.
    pub decrypted: bool,
    pub signed: bool,
    /// The signature, when there was one and it could be read.
    pub signature: Option<SignatureView>,
    /// Key IDs the message was encrypted to, so a failure can say which key
    /// would have opened it.
    pub encrypted_to: Vec<String>,
    /// What went wrong, in a sentence, when something did.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureState {
    /// Verified against a certificate in the keyring.
    Valid,
    /// The certificate is known and the signature does not match: the
    /// message was altered, or the signature is forged.
    Invalid,
    /// Signed by a key that is not in the keyring, so it cannot be checked.
    UnknownKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignatureView {
    pub state: SignatureState,
    /// The issuer's key ID, as the signature names it.
    pub key_id: Option<String>,
    /// The signing certificate's fingerprint when it is known, else the
    /// issuer fingerprint the signature carries, if any.
    pub fingerprint: Option<String>,
    /// The certificate's primary user ID, when the certificate is known.
    pub signer: Option<String>,
    /// Whether the certificate belongs to the address in From. A valid
    /// signature from someone else's key is worth saying out loud.
    pub signer_matches_sender: Option<bool>,
    /// Unix seconds.
    pub created: Option<i64>,
}

/// What opening a message found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenedMessage {
    pub security: Option<SecurityView>,
    /// The text to show, when protection changed it — decrypted, or the
    /// signed part alone. `None` means "use the message's own text", or,
    /// for a message that could not be decrypted, that there is none.
    pub body_text: Option<String>,
    /// Armored public keys attached to the message (`application/pgp-keys`),
    /// including ones inside the encryption.
    pub pgp_keys: Vec<String>,
    /// The MIME entity that was encrypted, once decrypted: where an encrypted
    /// message's attachments are. Kept in memory for the one open and never
    /// written anywhere.
    pub decrypted: Option<Vec<u8>>,
}

struct Context<'a> {
    keyring: Option<&'a Keyring>,
    /// Read on first use: most mail is not protected, and opening it should
    /// not cost a directory listing and a parse of every key on file.
    certs: std::cell::OnceCell<Vec<Cert>>,
    sender: Option<String>,
}

impl Context<'_> {
    fn certs(&self) -> &[Cert] {
        self.certs
            .get_or_init(|| match self.keyring.map(Keyring::certs).transpose() {
                Ok(certs) => certs.unwrap_or_default(),
                Err(err) => {
                    tracing::warn!(%err, "the keyring could not be read");
                    Vec::new()
                }
            })
    }
}

/// Opens a stored message: detects protection, decrypts, verifies.
pub fn open_message(raw: &[u8], keyring: Option<&Keyring>) -> OpenedMessage {
    match MessageParser::default().parse(raw) {
        Some(parsed) => open_parsed(&parsed, keyring),
        None => OpenedMessage::default(),
    }
}

/// [`open_message`] for a message the caller has already parsed, so opening
/// one does not parse it twice.
pub fn open_parsed(parsed: &MimeMessage<'_>, keyring: Option<&Keyring>) -> OpenedMessage {
    let context = Context {
        keyring,
        certs: std::cell::OnceCell::new(),
        sender: parsed
            .from()
            .and_then(|from| from.first())
            .and_then(|addr| addr.address())
            .map(str::to_lowercase),
    };
    let mut keys = attached_keys(parsed);

    // The outermost PGP/MIME structure decides: parts are in document order.
    for part in &parsed.parts {
        let Some(content_type) = part.content_type() else {
            continue;
        };
        if !content_type.ctype().eq_ignore_ascii_case("multipart") {
            continue;
        }
        let subtype = content_type
            .subtype()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let protocol = content_type
            .attribute("protocol")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if subtype == "encrypted"
            && (protocol.is_empty() || protocol == "application/pgp-encrypted")
        {
            let mut opened = open_encrypted(parsed, part, &context);
            keys.append(&mut opened.pgp_keys);
            opened.pgp_keys = keys;
            return opened;
        }
        if subtype == "signed" && protocol == "application/pgp-signature" {
            let (security, text) = open_signed(parsed, part, &context);
            return OpenedMessage {
                security: Some(security),
                body_text: text,
                pgp_keys: keys,
                decrypted: None,
            };
        }
    }

    if let Some(text) = parsed.body_text(0) {
        if let Some((security, text)) = open_inline(&text, &context) {
            return OpenedMessage {
                security: Some(security),
                body_text: Some(text),
                pgp_keys: keys,
                decrypted: None,
            };
        }
    }

    OpenedMessage {
        security: None,
        body_text: None,
        pgp_keys: keys,
        decrypted: None,
    }
}

fn open_encrypted(
    parsed: &MimeMessage<'_>,
    part: &MessagePart<'_>,
    context: &Context<'_>,
) -> OpenedMessage {
    let mut security = SecurityView {
        format: "pgp/mime".into(),
        encrypted: true,
        ..SecurityView::default()
    };
    let armored = part
        .sub_parts()
        .unwrap_or_default()
        .iter()
        .filter_map(|id| parsed.part(*id))
        .map(|sub| String::from_utf8_lossy(sub.contents()).into_owned())
        .find(|text| text.contains("-----BEGIN PGP MESSAGE-----"));
    let Some(armored) = armored else {
        security.error = Some("the encrypted message has no OpenPGP data in it".into());
        return OpenedMessage {
            security: Some(security),
            ..OpenedMessage::default()
        };
    };

    let attempt = unwrap_armored(&armored, context);
    security.encrypted_to = attempt.encrypted_to;
    let unwrapped = match attempt.result {
        Ok(unwrapped) => unwrapped,
        Err(err) => {
            security.error = Some(err);
            return OpenedMessage {
                security: Some(security),
                ..OpenedMessage::default()
            };
        }
    };
    security.decrypted = true;
    security.signed = unwrapped.signature.is_some();
    security.signature = unwrapped.signature;

    // What was encrypted is a MIME entity, which parses as a message.
    let Some(inner) = MessageParser::default().parse(&unwrapped.data) else {
        return OpenedMessage {
            security: Some(security),
            body_text: Some(String::from_utf8_lossy(&unwrapped.data).into_owned()),
            pgp_keys: Vec::new(),
            decrypted: None,
        };
    };
    let pgp_keys = attached_keys(&inner);

    // Signed, then encrypted as a whole (RFC 3156 §6.1) rather than in one
    // OpenPGP message: the signature is a multipart/signed inside.
    if !security.signed {
        let signed = inner.parts.iter().find(|part| {
            part.content_type().is_some_and(|ct| {
                ct.ctype().eq_ignore_ascii_case("multipart")
                    && ct
                        .subtype()
                        .is_some_and(|s| s.eq_ignore_ascii_case("signed"))
            })
        });
        if let Some(signed) = signed {
            let (inner_security, text) = open_signed(&inner, signed, context);
            security.signed = true;
            security.signature = inner_security.signature;
            if security.error.is_none() {
                security.error = inner_security.error;
            }
            return OpenedMessage {
                security: Some(security),
                body_text: text,
                pgp_keys,
                decrypted: Some(unwrapped.data.to_vec()),
            };
        }
    }

    let body_text = inner.body_text(0).map(|text| text.into_owned());
    OpenedMessage {
        security: Some(security),
        body_text,
        pgp_keys,
        decrypted: Some(unwrapped.data.to_vec()),
    }
}

fn open_signed(
    parsed: &MimeMessage<'_>,
    part: &MessagePart<'_>,
    context: &Context<'_>,
) -> (SecurityView, Option<String>) {
    let mut security = SecurityView {
        format: "pgp/mime".into(),
        signed: true,
        ..SecurityView::default()
    };
    let boundary = part
        .content_type()
        .and_then(|ct| ct.attribute("boundary"))
        .unwrap_or_default();
    let region = parsed
        .raw_message()
        .get(part.raw_body_offset() as usize..part.raw_end_offset() as usize)
        .unwrap_or_default();
    let pieces = split_multipart(region, boundary);
    let Some(signed_part) = pieces.first() else {
        security.error = Some("the signed message has no signed part".into());
        return (security, None);
    };
    let signed = crlf(signed_part);

    let armored = part
        .sub_parts()
        .unwrap_or_default()
        .iter()
        .filter_map(|id| parsed.part(*id))
        .map(|sub| String::from_utf8_lossy(sub.contents()).into_owned())
        .find(|text| text.contains("-----BEGIN PGP SIGNATURE-----"));

    let text = MessageParser::default()
        .parse(&signed)
        .and_then(|entity| entity.body_text(0).map(|t| t.into_owned()));

    let Some(armored) = armored else {
        security.error = Some("the signed message has no signature in it".into());
        return (security, text);
    };
    let signatures: Vec<DetachedSignature> = match DetachedSignature::from_string_many(&armored) {
        Ok((signatures, _)) => signatures.filter_map(|s| s.ok()).collect(),
        Err(err) => {
            security.error = Some(format!("the signature could not be read: {err}"));
            return (security, text);
        }
    };
    let views = signatures
        .iter()
        .map(|detached| {
            judge(&detached.signature, context, |component| match component {
                Component::Primary(key) => detached.verify(key, &signed).is_ok(),
                Component::Subkey(key) => detached.verify(key, &signed).is_ok(),
            })
        })
        .collect();
    security.signature = best(views);
    if security.signature.is_none() {
        security.error = Some("the signature could not be read".into());
    }
    (security, text)
}

/// Inline PGP in a text body: an armored message, or a cleartext signature.
///
/// The block is replaced by what it stood for; text around it — a greeting
/// a client put outside the armor, a list footer — is kept as it was.
fn open_inline(text: &str, context: &Context<'_>) -> Option<(SecurityView, String)> {
    let mut security = SecurityView {
        format: "inline".into(),
        ..SecurityView::default()
    };

    if let Some((start, end)) = block(
        text,
        "-----BEGIN PGP MESSAGE-----",
        "-----END PGP MESSAGE-----",
    ) {
        let attempt = unwrap_armored(&text[start..end], context);
        security.encrypted = attempt.encrypted;
        security.encrypted_to = attempt.encrypted_to;
        return Some(match attempt.result {
            Ok(unwrapped) => {
                security.decrypted = attempt.encrypted;
                security.signed = unwrapped.signature.is_some();
                security.signature = unwrapped.signature;
                let plain = String::from_utf8_lossy(&unwrapped.data).replace("\r\n", "\n");
                (
                    security,
                    format!("{}{}{}", &text[..start], plain, &text[end..]),
                )
            }
            Err(err) => {
                security.error = Some(err);
                (security, text.to_string())
            }
        });
    }

    if let Some((start, end)) = block(
        text,
        "-----BEGIN PGP SIGNED MESSAGE-----",
        "-----END PGP SIGNATURE-----",
    ) {
        security.signed = true;
        return Some(
            match CleartextSignedMessage::from_string(&text[start..end]) {
                Ok((message, _)) => {
                    let signed_text = message.signed_text();
                    let views = message
                        .signatures()
                        .iter()
                        .map(|signature| {
                            judge(signature, context, |component| match component {
                                Component::Primary(key) => {
                                    signature.verify(key, signed_text.as_bytes()).is_ok()
                                }
                                Component::Subkey(key) => {
                                    signature.verify(key, signed_text.as_bytes()).is_ok()
                                }
                            })
                        })
                        .collect();
                    security.signature = best(views);
                    let plain = signed_text.replace("\r\n", "\n");
                    (
                        security,
                        format!("{}{}{}", &text[..start], plain, &text[end..]),
                    )
                }
                Err(err) => {
                    security.error = Some(format!("the signed text could not be read: {err}"));
                    (security, text.to_string())
                }
            },
        );
    }
    None
}

/// The span of the first `begin` … `end` block, `end` included.
fn block(text: &str, begin: &str, end: &str) -> Option<(usize, usize)> {
    let start = text.find(begin)?;
    let stop = start + text[start..].find(end)? + end.len();
    Some((start, stop))
}

// -- OpenPGP messages --------------------------------------------------------------

struct Unwrapped {
    data: Vec<u8>,
    signature: Option<SignatureView>,
}

struct Attempt {
    encrypted: bool,
    encrypted_to: Vec<String>,
    result: Result<Unwrapped, String>,
}

/// Decrypts (when encrypted), decompresses and verifies an armored message.
fn unwrap_armored(armored: &str, context: &Context<'_>) -> Attempt {
    let esk = match Message::from_armor(armored.as_bytes()) {
        Ok((Message::Encrypted { esk, .. }, _)) => esk,
        Ok((message, _)) => {
            return Attempt {
                encrypted: false,
                encrypted_to: Vec::new(),
                result: read_contents(message, context),
            }
        }
        Err(err) => {
            return Attempt {
                encrypted: false,
                encrypted_to: Vec::new(),
                result: Err(format!("the OpenPGP message could not be read: {err}")),
            }
        }
    };

    let encrypted_to: Vec<String> = esk
        .iter()
        .filter_map(|esk| match esk {
            Esk::PublicKeyEncryptedSessionKey(pkesk) => pkesk.id().ok().map(|id| hex(id.as_ref())),
            Esk::SymKeyEncryptedSessionKey(_) => None,
        })
        .collect();

    let mut problem: Option<String> = None;
    for cert in context.certs() {
        let Some(secret) = &cert.secret else {
            continue;
        };
        let addressed = esk.iter().any(|esk| match esk {
            Esk::PublicKeyEncryptedSessionKey(pkesk) => {
                pkesk.match_identity(secret.primary_key.public_key())
                    || secret
                        .secret_subkeys
                        .iter()
                        .any(|sub| pkesk.match_identity(sub.key.public_key()))
            }
            Esk::SymKeyEncryptedSessionKey(_) => false,
        });
        if !addressed {
            continue;
        }
        let Some(keyring) = context.keyring else {
            continue;
        };
        let password = match keyring.password_for(cert, secret) {
            Ok(password) => password,
            Err(err) => {
                problem = Some(err.to_string());
                continue;
            }
        };
        let message = match Message::from_armor(armored.as_bytes()) {
            Ok((message, _)) => message,
            Err(err) => {
                problem = Some(format!("the OpenPGP message could not be read: {err}"));
                continue;
            }
        };
        match message.decrypt(&password, secret) {
            Ok(decrypted) => {
                return Attempt {
                    encrypted: true,
                    encrypted_to,
                    result: read_contents(decrypted, context),
                }
            }
            Err(err) => {
                problem = Some(format!(
                    "key {} could not decrypt the message: {err}",
                    cert.fingerprint
                ));
            }
        }
    }

    let problem = problem.unwrap_or_else(|| {
        if encrypted_to.is_empty() {
            "no secret key for this message".to_string()
        } else {
            format!(
                "no secret key for this message; it is encrypted to {}",
                encrypted_to.join(", ")
            )
        }
    });
    Attempt {
        encrypted: true,
        encrypted_to,
        result: Err(problem),
    }
}

/// The literal data of a decrypted (or never encrypted) message, and its
/// signature's verdict.
fn read_contents(message: Message<'_>, context: &Context<'_>) -> Result<Unwrapped, String> {
    let mut message = message;
    if message.is_compressed() {
        message = message
            .decompress()
            .map_err(|err| format!("the message could not be decompressed: {err}"))?;
    }
    let data = message
        .as_data_vec()
        .map_err(|err| format!("the message could not be read: {err}"))?;

    let signature = match &message {
        Message::Signed { reader, .. } => {
            let views = (0..reader.num_signatures())
                .filter_map(|index| {
                    let signature = reader.signature(index)?;
                    Some(judge(signature, context, |component| match component {
                        Component::Primary(key) => {
                            message.verify_nested_explicit(index, key).is_ok()
                        }
                        Component::Subkey(key) => {
                            message.verify_nested_explicit(index, key).is_ok()
                        }
                    }))
                })
                .collect();
            best(views)
        }
        _ => None,
    };
    Ok(Unwrapped { data, signature })
}

// -- signatures --------------------------------------------------------------------

enum Component<'a> {
    Primary(&'a pgp::composed::SignedPublicKey),
    Subkey(&'a pgp::composed::SignedPublicSubKey),
}

fn issued_by(signature: &Signature, key: &impl KeyDetails) -> bool {
    let fingerprint = key.fingerprint();
    let key_id = key.legacy_key_id();
    signature
        .issuer_fingerprint()
        .iter()
        .any(|fp| **fp == fingerprint)
        || signature.issuer_key_id().iter().any(|id| **id == key_id)
}

/// Finds the certificate that issued `signature` and asks `verify` about it.
fn judge(
    signature: &Signature,
    context: &Context<'_>,
    verify: impl Fn(Component<'_>) -> bool,
) -> SignatureView {
    let key_id = signature.issuer_key_id().first().map(|id| hex(id.as_ref()));
    let issuer_fingerprint = signature
        .issuer_fingerprint()
        .first()
        .map(|fp| hex(fp.as_bytes()));
    let created = signature.created().map(|t| t.as_secs() as i64);

    for cert in context.certs() {
        let public = &cert.public;
        let component = if issued_by(signature, public) {
            Some(Component::Primary(public))
        } else {
            public
                .public_subkeys
                .iter()
                .find(|sub| issued_by(signature, *sub))
                .map(Component::Subkey)
        };
        let Some(component) = component else {
            continue;
        };
        let valid = verify(component);
        let matches_sender = context.sender.as_ref().map(|sender| {
            user_ids(public)
                .iter()
                .filter_map(|uid| email_of(uid))
                .any(|email| &email == sender)
        });
        return SignatureView {
            state: if valid {
                SignatureState::Valid
            } else {
                SignatureState::Invalid
            },
            key_id: key_id
                .or_else(|| Some(cert.fingerprint[cert.fingerprint.len() - 16..].to_string())),
            fingerprint: Some(cert.fingerprint.clone()),
            signer: primary_user_id(public),
            signer_matches_sender: matches_sender,
            created,
        };
    }

    SignatureView {
        state: SignatureState::UnknownKey,
        key_id: key_id.or_else(|| {
            issuer_fingerprint
                .as_ref()
                .map(|fp| fp[fp.len().saturating_sub(16)..].to_string())
        }),
        fingerprint: issuer_fingerprint,
        signer: None,
        signer_matches_sender: None,
        created,
    }
}

/// Of several signatures, the one to report: a valid one if any is.
fn best(views: Vec<SignatureView>) -> Option<SignatureView> {
    views
        .iter()
        .find(|view| view.state == SignatureState::Valid)
        .or_else(|| views.first())
        .cloned()
}

// -- MIME --------------------------------------------------------------------------

/// The parts of a multipart body, each exactly as it appears between its
/// delimiters (RFC 2046 §5.1.1: the line break before a delimiter belongs to
/// the delimiter, not to the part).
pub(crate) fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    let mut line_start = 0;

    while line_start < body.len() {
        let line_end = body[line_start..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| line_start + i)
            .unwrap_or(body.len());
        let line = &body[line_start..line_end];
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if !boundary.is_empty() && line.starts_with(delimiter) {
            let rest = &line[delimiter.len()..];
            let closing = rest.starts_with(b"--");
            if closing || rest.iter().all(|b| *b == b' ' || *b == b'\t') {
                if let Some(from) = start {
                    let mut to = line_start;
                    if to > from && body[to - 1] == b'\n' {
                        to -= 1;
                        if to > from && body[to - 1] == b'\r' {
                            to -= 1;
                        }
                    }
                    parts.push(&body[from..to]);
                }
                if closing {
                    break;
                }
                start = Some((line_end + 1).min(body.len()));
            }
        }
        line_start = line_end + 1;
    }
    parts
}

/// Public keys attached to a message.
fn attached_keys(parsed: &MimeMessage<'_>) -> Vec<String> {
    let mut keys = Vec::new();
    for part in &parsed.parts {
        let is_keys = part.content_type().is_some_and(|ct| {
            ct.ctype().eq_ignore_ascii_case("application")
                && ct
                    .subtype()
                    .is_some_and(|s| s.eq_ignore_ascii_case("pgp-keys"))
        });
        let is_attachment = part.attachment_name().is_some();
        if !is_keys && !is_attachment {
            continue;
        }
        let text = String::from_utf8_lossy(part.contents());
        for block in armored_blocks(&text) {
            if block.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----")
                && !keys.iter().any(|k| k == block)
            {
                keys.push(block.to_string());
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multipart_body_splits_at_its_delimiters_only() {
        let body =
            b"preamble\r\n--b\r\nfirst\r\n--bb not me\r\n\r\n--b\r\nsecond\r\n--b--\r\nepilogue";
        let parts = split_multipart(body, "b");
        assert_eq!(
            parts,
            vec![&b"first\r\n--bb not me\r\n"[..], &b"second"[..]]
        );
    }
}
