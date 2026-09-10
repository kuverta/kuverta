//! Identity for a message within an account.
//!
//! Getting this wrong is expensive to undo, so it is settled here rather than
//! being spread across the sync code. Two rules:
//!
//! 1. A message is identified by its RFC 5322 `Message-ID`. Gmail exposes
//!    labels as folders, so the *same* message is delivered to the client
//!    several times under different UIDs; without this every count in the UI
//!    would be inflated.
//! 2. `Message-ID` is mandatory in practice but not guaranteed. When it is
//!    absent we synthesise a stable key from the headers that identify the
//!    message in combination.
//!
//! Scope is per account, never global: the same message received on two of your
//! accounts really is two copies, and collapsing them would lose information.

use sha2::{Digest, Sha256};

use crate::model::NewMessage;

/// Strips the angle brackets and surrounding whitespace from a `Message-ID`.
///
/// Case is preserved. RFC 5322 treats the local part as case-sensitive, and
/// folding case here would risk merging two genuinely distinct messages — a
/// worse failure than missing a duplicate.
pub fn normalize_message_id(raw: &str) -> &str {
    raw.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
}

/// Computes the per-account identity key for a message.
///
/// Returns either `mid:<message-id>` or, as a fallback, `synth:<sha256>`. The
/// prefixes make it obvious in the database which path a row took.
pub fn dedup_key(message: &NewMessage) -> String {
    if let Some(raw) = message.rfc822_message_id.as_deref() {
        let normalized = normalize_message_id(raw);
        if !normalized.is_empty() {
            return format!("mid:{normalized}");
        }
    }

    // No usable Message-ID. Hash the fields that together identify a message:
    // two mails matching on all four are duplicates for any practical purpose.
    let mut hasher = Sha256::new();
    for field in [
        message.date_utc.map(|d| d.to_string()).unwrap_or_default(),
        message.from_addr.clone().unwrap_or_default(),
        message.subject.clone().unwrap_or_default(),
        message
            .size_bytes
            .map(|s| s.to_string())
            .unwrap_or_default(),
    ] {
        hasher.update(field.as_bytes());
        // Separator prevents ("ab", "c") colliding with ("a", "bc").
        hasher.update([0u8]);
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("synth:{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg() -> NewMessage {
        NewMessage {
            subject: Some("Rechnung".into()),
            from_addr: Some("a@example.de".into()),
            date_utc: Some(1_757_000_000),
            size_bytes: Some(4211),
            ..Default::default()
        }
    }

    #[test]
    fn angle_brackets_and_whitespace_are_stripped() {
        assert_eq!(
            normalize_message_id("  <abc@example.com> "),
            "abc@example.com"
        );
        assert_eq!(normalize_message_id("abc@example.com"), "abc@example.com");
    }

    #[test]
    fn message_id_wins_when_present() {
        let mut m = msg();
        m.rfc822_message_id = Some("<abc@example.com>".into());
        assert_eq!(dedup_key(&m), "mid:abc@example.com");
    }

    #[test]
    fn same_message_id_different_metadata_still_matches() {
        // This is the Gmail label case: the server may report a different size
        // or internal date per folder, but it is one message.
        let mut a = msg();
        a.rfc822_message_id = Some("<abc@example.com>".into());
        let mut b = msg();
        b.rfc822_message_id = Some("<abc@example.com>".into());
        b.size_bytes = Some(9999);
        b.subject = Some("different".into());

        assert_eq!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn missing_message_id_falls_back_to_a_stable_hash() {
        let key = dedup_key(&msg());
        assert!(key.starts_with("synth:"), "got {key}");
        assert_eq!(key, dedup_key(&msg()), "must be deterministic");
    }

    #[test]
    fn fallback_distinguishes_different_messages() {
        let mut other = msg();
        other.subject = Some("Mahnung".into());
        assert_ne!(dedup_key(&msg()), dedup_key(&other));
    }

    #[test]
    fn empty_message_id_falls_back_rather_than_colliding() {
        // An empty or bracket-only Message-ID must not produce the shared key
        // "mid:", which would collapse every such message into one.
        let mut a = msg();
        a.rfc822_message_id = Some("<>".into());
        let mut b = msg();
        b.rfc822_message_id = Some("   ".into());
        b.subject = Some("something else".into());

        assert!(dedup_key(&a).starts_with("synth:"));
        assert!(dedup_key(&b).starts_with("synth:"));
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }
}
