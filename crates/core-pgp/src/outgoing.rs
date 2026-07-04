//! PGP/MIME for outgoing mail (RFC 3156).
//!
//! Takes a message that has already been built and wraps its body. The outer
//! headers — From, To, Cc, Subject, Date, Message-ID, the threading headers —
//! are kept exactly as they were written; what moves inside the protection is
//! the MIME entity that was the body: its `Content-*` headers and content.
//!
//! ## Signed
//!
//! `multipart/signed`, with the entity as the first part and a detached
//! signature as the second. The signature covers the entity's bytes exactly
//! as they appear between the boundaries, with CRLF line endings — so those
//! bytes must survive every hop unchanged. A relay may re-encode 8-bit text,
//! strip trailing whitespace, or turn a line starting `From ` into `>From `,
//! and any of those breaks the signature. So a text body is re-encoded as
//! quoted-printable with exactly those hazards escaped, before it is signed.
//!
//! ## Encrypted
//!
//! `multipart/encrypted`: a version part and the armored OpenPGP message.
//! Signed *and* encrypted is one OpenPGP message that is both (RFC 3156
//! §6.2), which is what GnuPG, Thunderbird and most others write, and what
//! they read most reliably.
//!
//! Encrypted to every To and Cc recipient and to the sender, so the copy
//! filed in Sent can be read again. Bcc is refused; see the crate note.

use pgp::composed::{ArmorOptions, DetachedSignature, MessageBuilder};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::types::{Password, SigningKey};

use crate::keyring::{
    bare_address, encryption_targets, now, pick_for_email, pick_signer, signing_component,
    EncryptionTarget, Keyring, SigningComponent,
};
use crate::{PgpError, Result};

/// What to do to a message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protection {
    pub sign: bool,
    pub encrypt: bool,
}

impl Protection {
    pub fn any(&self) -> bool {
        self.sign || self.encrypt
    }
}

/// Signs and/or encrypts a built message.
///
/// `recipients` are the visible ones, To and Cc; `bcc` only matters in that
/// it must be empty when encrypting. `sender` signs, and is added to the
/// recipients of an encrypted message.
///
/// Returns the message unchanged when nothing was asked for.
pub fn protect(
    keyring: &Keyring,
    rfc822: &[u8],
    sender: &str,
    recipients: &[String],
    bcc: &[String],
    protection: Protection,
) -> Result<Vec<u8>> {
    if !protection.any() {
        return Ok(rfc822.to_vec());
    }
    if protection.encrypt && bcc.iter().any(|address| !address.trim().is_empty()) {
        return Err(PgpError::BccWithEncryption);
    }

    let (outer, entity) = split_entity(rfc822)?;
    let now = now();
    let certs = keyring.certs()?;
    let sender = bare_address(sender);

    // The signing key and its passphrase, when signing. Found before any
    // encryption work so that "no passphrase" is the error a person sees,
    // rather than whatever went wrong later.
    let signer = if protection.sign {
        let (cert, secret) = pick_signer(&certs, &sender, now)
            .ok_or_else(|| PgpError::NoSecretKey(sender.clone()))?;
        let password = keyring.password_for(cert, secret)?;
        let component =
            signing_component(secret, now).ok_or_else(|| PgpError::NoSecretKey(sender.clone()))?;
        // Unlocked once up front so a wrong passphrase in the keychain is
        // named as such, not reported as a failed signature.
        secret
            .primary_key
            .unlock(&password, |_, _| Ok(()))
            .map_err(|_| PgpError::WrongPassphrase(cert.fingerprint.clone()))?
            .map_err(|_| PgpError::WrongPassphrase(cert.fingerprint.clone()))?;
        Some((component, password))
    } else {
        None
    };
    let signing_key: Option<Box<&dyn SigningKey>> = signer.as_ref().map(|(component, _)| {
        Box::new(match component {
            SigningComponent::Primary(key) => *key as &dyn SigningKey,
            SigningComponent::Subkey(key) => *key as &dyn SigningKey,
        })
    });

    let mut rng = rand::thread_rng();
    let body = if protection.encrypt {
        // Everyone visible, and ourselves, each exactly once.
        let mut addresses: Vec<String> = Vec::new();
        for address in recipients
            .iter()
            .map(|a| bare_address(a))
            .chain(std::iter::once(sender.clone()))
        {
            if !address.is_empty() && !addresses.contains(&address) {
                addresses.push(address);
            }
        }
        let mut missing = Vec::new();
        let mut targets: Vec<EncryptionTarget<'_>> = Vec::new();
        for address in &addresses {
            let target = pick_for_email(&certs, address, now)
                .and_then(|cert| encryption_targets(&cert.public, now).into_iter().next());
            match target {
                Some(target) => {
                    if !targets
                        .iter()
                        .any(|t| t.fingerprint() == target.fingerprint())
                    {
                        targets.push(target);
                    }
                }
                None => missing.push(address.clone()),
            }
        }
        // Other people first: their keys are the ones to go and get, and
        // the sender's own is a different fix.
        let own_missing = missing.contains(&sender);
        missing.retain(|address| address != &sender);
        if !missing.is_empty() {
            return Err(PgpError::NoKeyFor(missing));
        }
        if own_missing {
            return Err(PgpError::NoOwnKey(sender));
        }

        let mut builder = MessageBuilder::from_bytes("", entity.clone())
            .seipd_v1(&mut rng, SymmetricKeyAlgorithm::AES256);
        for target in &targets {
            match target {
                EncryptionTarget::Primary(key) => builder.encrypt_to_key(&mut rng, *key)?,
                EncryptionTarget::Subkey(key) => builder.encrypt_to_key(&mut rng, *key)?,
            };
        }
        if let (Some(key), Some((_, password))) = (signing_key.as_ref(), signer.as_ref()) {
            let password = Password::from(password.read().as_slice());
            builder.sign(**key, password, HashAlgorithm::Sha256);
        }
        let armored = builder.to_armored_string(&mut rng, ArmorOptions::default())?;
        encrypted_entity(&armored)
    } else {
        let (Some(key), Some((_, password))) = (signing_key.as_ref(), signer.as_ref()) else {
            return Err(PgpError::NoSecretKey(sender));
        };
        let signature = DetachedSignature::sign_binary_data(
            &mut rng,
            key,
            password,
            HashAlgorithm::Sha256,
            entity.as_slice(),
        )?;
        signed_entity(
            &entity,
            &signature.to_armored_string(ArmorOptions::default())?,
        )
    };

    let mut out = outer;
    out.extend_from_slice(&body);
    Ok(out)
}

/// Splits a message into its outer headers and its body entity.
///
/// The entity is the `Content-*` headers and the content. A text body is
/// re-encoded here, as quoted-printable that no relay has a reason to touch;
/// anything else (a multipart with attachments, say) is carried over as it
/// was written, which for `mail-builder` output is base64 or 7-bit already.
fn split_entity(rfc822: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let split = find(rfc822, b"\r\n\r\n")
        .ok_or_else(|| PgpError::Invalid("the message has no body to protect".into()))?;
    let header_block = &rfc822[..split + 2];
    let body = &rfc822[split + 4..];

    let mut outer = Vec::new();
    let mut content: Vec<(String, Vec<u8>)> = Vec::new();
    for field in header_fields(header_block) {
        let name = field_name(field);
        if name.to_ascii_lowercase().starts_with("content-") {
            content.push((name.to_ascii_lowercase(), field.to_vec()));
        } else {
            outer.extend_from_slice(field);
        }
    }

    let content_type = content
        .iter()
        .find(|(name, _)| name == "content-type")
        .map(|(_, raw)| String::from_utf8_lossy(raw).to_ascii_lowercase());
    let is_text = content_type.as_deref().is_none_or(|ct| {
        ct["content-type:".len()..]
            .trim_start()
            .starts_with("text/")
    });

    let mut entity = Vec::new();
    if is_text {
        let text = mail_parser::MessageParser::default()
            .parse(rfc822)
            .and_then(|parsed| parsed.root_part().text_contents().map(str::to_string))
            .unwrap_or_default();
        match content.iter().find(|(name, _)| name == "content-type") {
            Some((_, raw)) => entity.extend_from_slice(raw),
            None => entity.extend_from_slice(b"Content-Type: text/plain; charset=\"utf-8\"\r\n"),
        }
        for (name, raw) in &content {
            if name != "content-type" && name != "content-transfer-encoding" {
                entity.extend_from_slice(raw);
            }
        }
        entity.extend_from_slice(b"Content-Transfer-Encoding: quoted-printable\r\n\r\n");
        entity.extend_from_slice(quoted_printable(&text).as_bytes());
    } else {
        for (_, raw) in &content {
            entity.extend_from_slice(raw);
        }
        entity.extend_from_slice(b"\r\n");
        entity.extend_from_slice(&crlf(body));
        if !entity.ends_with(b"\r\n") {
            entity.extend_from_slice(b"\r\n");
        }
    }
    Ok((outer, entity))
}

fn signed_entity(entity: &[u8], armored_signature: &str) -> Vec<u8> {
    let boundary = boundary();
    let mut out = format!(
        "Content-Type: multipart/signed; micalg=pgp-sha256;\r\n \
         protocol=\"application/pgp-signature\";\r\n boundary=\"{boundary}\"\r\n\r\n\
         This is an OpenPGP/MIME signed message (RFC 4880 and 3156)\r\n\
         --{boundary}\r\n"
    )
    .into_bytes();
    // The entity ends in CRLF; the CRLF before the next delimiter belongs to
    // the delimiter, so what was signed is exactly `entity`.
    out.extend_from_slice(entity);
    out.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\n\
             Content-Type: application/pgp-signature; name=\"signature.asc\"\r\n\
             Content-Description: OpenPGP digital signature\r\n\
             Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n\
             {}\r\n--{boundary}--\r\n",
            crlf_str(armored_signature.trim_end())
        )
        .as_bytes(),
    );
    out
}

fn encrypted_entity(armored: &str) -> Vec<u8> {
    let boundary = boundary();
    format!(
        "Content-Type: multipart/encrypted;\r\n \
         protocol=\"application/pgp-encrypted\";\r\n boundary=\"{boundary}\"\r\n\r\n\
         This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156)\r\n\
         --{boundary}\r\n\
         Content-Type: application/pgp-encrypted\r\n\
         Content-Description: PGP/MIME version identification\r\n\r\n\
         Version: 1\r\n\r\n\
         --{boundary}\r\n\
         Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\
         Content-Description: OpenPGP encrypted message\r\n\
         Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n\
         {}\r\n\r\n--{boundary}--\r\n",
        crlf_str(armored.trim_end())
    )
    .into_bytes()
}

/// A boundary nothing in the content can contain: random, and with `=_`,
/// which quoted-printable and base64 never produce at the start of a line.
fn boundary() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("=_kuverta_{}", crate::hex(&bytes).to_lowercase())
}

/// Quoted-printable that survives relays unchanged, for signing.
///
/// Beyond RFC 2045: whitespace at the end of a line is always encoded
/// (relays strip it), `From ` at the start of a line has its `F` encoded
/// (mbox-minded relays turn it into `>From `), and so does a lone leading
/// `.`, which some transports treat as the end of the data. Output lines end
/// in CRLF and stay under 76 characters.
pub(crate) fn quoted_printable(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }

    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for line in lines {
        let bytes = line.as_bytes();
        let mut column = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            let last = i + 1 == bytes.len();
            let at_line_start = column == 0;
            let escape = byte == b'='
                || byte >= 0x7f
                || (byte < 0x20 && byte != b'\t')
                || ((byte == b' ' || byte == b'\t') && last)
                || (at_line_start && byte == b'F' && bytes[i..].starts_with(b"From "))
                || (at_line_start && byte == b'.');
            let width = if escape { 3 } else { 1 };
            // Room for the soft break's `=` on this line.
            if column + width > 75 {
                out.push_str("=\r\n");
                column = 0;
                // Re-check the start-of-line hazards on the new line.
                if byte == b'.' || (byte == b'F' && bytes[i..].starts_with(b"From ")) {
                    out.push_str(&format!("={byte:02X}"));
                    column = 3;
                    continue;
                }
            }
            if escape {
                out.push_str(&format!("={byte:02X}"));
            } else {
                out.push(byte as char);
            }
            column += width;
        }
        out.push_str("\r\n");
    }
    out
}

/// Header fields, each with its continuation lines and its final CRLF.
fn header_fields(block: &[u8]) -> Vec<&[u8]> {
    let mut fields: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start < block.len() {
        let end = find(&block[start..], b"\r\n")
            .map(|i| start + i + 2)
            .unwrap_or(block.len());
        let continuation = matches!(block[start], b' ' | b'\t');
        match fields.last_mut() {
            Some(field) if continuation => field.1 = end,
            _ => fields.push((start, end)),
        }
        start = end;
    }
    fields.into_iter().map(|(s, e)| &block[s..e]).collect()
}

fn field_name(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == b':').unwrap_or(0);
    String::from_utf8_lossy(&field[..end]).trim().to_string()
}

pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Bare LF to CRLF, leaving existing CRLF alone.
pub(crate) fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    let mut previous = 0u8;
    for &byte in bytes {
        if byte == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(byte);
        previous = byte;
    }
    out
}

fn crlf_str(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_printable_escapes_what_relays_would_change() {
        let encoded = quoted_printable("From here on\ntrailing space \n.\nGrüße = gut\n");
        assert!(encoded.starts_with("=46rom here on\r\n"), "{encoded}");
        assert!(encoded.contains("trailing space=20\r\n"), "{encoded}");
        assert!(encoded.contains("\r\n=2E\r\n"), "{encoded}");
        assert!(encoded.contains("Gr=C3=BC=C3=9Fe =3D gut\r\n"), "{encoded}");
        assert!(encoded.lines().all(|line| line.len() <= 76));
        assert!(encoded.is_ascii());
    }

    #[test]
    fn long_lines_are_softly_broken_and_decode_back() {
        let line = "x".repeat(200);
        let encoded = quoted_printable(&line);
        assert!(encoded.lines().all(|l| l.len() <= 76), "{encoded}");
        let decoded = encoded.replace("=\r\n", "").replace("\r\n", "");
        assert_eq!(decoded, line);
    }

    #[test]
    fn folded_headers_stay_together() {
        let block = b"Subject: one\r\n two\r\nTo: a@example.com\r\n";
        let fields = header_fields(block);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], b"Subject: one\r\n two\r\n");
    }
}
