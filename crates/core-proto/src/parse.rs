//! MIME message -> `NewMessage`.
//!
//! Parsing is delegated entirely to `mail-parser`: hand-rolled MIME handling is
//! the classic source of memory-safety bugs in mail clients, and this is the
//! best-tested implementation in the Rust ecosystem.

use core_store::model::NewMessage;
use mail_parser::{Address, MessageParser};

/// Longest snippet kept for the message list. Enough for a useful preview
/// without pulling whole bodies into the list query.
const SNIPPET_LEN: usize = 220;

/// Parses a raw RFC 5322 message.
///
/// Returns `None` only when the bytes are not a message at all. Malformed but
/// recognisable mail still parses — two decades of broken senders mean strict
/// parsing would reject real mail.
pub fn parse_message(raw: &[u8], size: Option<i64>) -> Option<NewMessage> {
    let parsed = MessageParser::default().parse(raw)?;

    let (from_name, from_addr) = match parsed.from() {
        Some(Address::List(addrs)) => addrs
            .first()
            .map(|a| {
                (
                    a.name().map(str::to_string),
                    a.address().map(str::to_string),
                )
            })
            .unwrap_or((None, None)),
        // A group sender is legal but vanishingly rare; take the first member.
        Some(Address::Group(groups)) => groups
            .first()
            .and_then(|g| g.addresses.first())
            .map(|a| {
                (
                    a.name().map(str::to_string),
                    a.address().map(str::to_string),
                )
            })
            .unwrap_or((None, None)),
        None => (None, None),
    };

    let body_text = parsed.body_text(0).map(|c| c.into_owned());
    // Bound to a local: the iterator borrows `parsed`, and as a temporary
    // inside the struct literal it would outlive it.
    let has_attachments = parsed.attachments().next().is_some();

    Some(NewMessage {
        rfc822_message_id: parsed.message_id().map(str::to_string),
        subject: parsed.subject().map(str::to_string),
        from_name,
        from_addr,
        date_utc: parsed.date().map(|d| d.to_timestamp()),
        size_bytes: size.or(Some(raw.len() as i64)),
        snippet: body_text.as_deref().map(snippet),
        has_attachments,
        // Read raw: mail-parser may interpret `List-Id: Name <id>` as an
        // address, and the bracketed identifier is what rules need to match on.
        list_id: parsed.header_raw("List-Id").map(clean_list_id),
        in_reply_to: parsed
            .header_raw("In-Reply-To")
            .map(|v| core_store::normalize_message_id(v).to_string()),
        body_path: None,
        search_text: body_text,
    })
}

/// Collapses whitespace and truncates on a character boundary.
fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SNIPPET_LEN {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(SNIPPET_LEN).collect();
    format!("{cut}…")
}

/// `List-Id: Rust Weekly <news.rustweekly.example>` -> `news.rustweekly.example`.
///
/// The bracketed identifier is the stable part; the display name is not.
fn clean_list_id(raw: &str) -> String {
    let raw = raw.trim();
    match (raw.rfind('<'), raw.rfind('>')) {
        (Some(open), Some(close)) if close > open + 1 => raw[open + 1..close].trim().to_string(),
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_common_headers() {
        let raw = b"Message-ID: <abc@example.com>\r\n\
                    From: Anna Weber <anna@example.de>\r\n\
                    Subject: Termin\r\n\
                    Date: Wed, 02 Sep 2026 11:42:33 +0200\r\n\
                    \r\n\
                    Dienstag passt.\r\n";
        let m = parse_message(raw, None).unwrap();

        assert_eq!(m.rfc822_message_id.as_deref(), Some("abc@example.com"));
        assert_eq!(m.from_name.as_deref(), Some("Anna Weber"));
        assert_eq!(m.from_addr.as_deref(), Some("anna@example.de"));
        assert_eq!(m.subject.as_deref(), Some("Termin"));
        // 2026-09-02 11:42:33 +0200 == 2026-09-02 09:42:33 UTC
        assert_eq!(m.date_utc, Some(1_788_342_153));
        assert!(!m.has_attachments);
    }

    #[test]
    fn decodes_rfc2047_encoded_subjects() {
        // German mail is full of these; storing the raw encoded word would make
        // both the message list and search useless.
        let raw = b"Message-ID: <x@example.de>\r\n\
                    Subject: =?UTF-8?Q?Ihre_Abschlagszahlung_f=C3=BCr_M=C3=A4rz?=\r\n\
                    \r\n\
                    body\r\n";
        let m = parse_message(raw, None).unwrap();
        assert_eq!(m.subject.as_deref(), Some("Ihre Abschlagszahlung für März"));
    }

    #[test]
    fn extracts_the_bracketed_list_id() {
        let raw = b"Message-ID: <n@example>\r\n\
                    List-Id: Rust Weekly <news.rustweekly.example>\r\n\
                    \r\n\
                    body\r\n";
        let m = parse_message(raw, None).unwrap();
        assert_eq!(m.list_id.as_deref(), Some("news.rustweekly.example"));
    }

    #[test]
    fn a_missing_message_id_is_not_an_error() {
        let raw = b"From: cron@example.org\r\nSubject: nightly\r\n\r\nok\r\n";
        let m = parse_message(raw, None).unwrap();
        assert_eq!(m.rfc822_message_id, None);
        assert_eq!(m.subject.as_deref(), Some("nightly"));
    }

    #[test]
    fn detects_attachments() {
        let raw = b"Message-ID: <a@example>\r\n\
                    Subject: Rechnung\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/mixed; boundary=\"bb\"\r\n\
                    \r\n\
                    --bb\r\n\
                    Content-Type: text/plain\r\n\
                    \r\n\
                    Anbei.\r\n\
                    --bb\r\n\
                    Content-Type: application/pdf; name=\"r.pdf\"\r\n\
                    Content-Disposition: attachment; filename=\"r.pdf\"\r\n\
                    \r\n\
                    %PDF-1.4\r\n\
                    --bb--\r\n";
        let m = parse_message(raw, None).unwrap();
        assert!(m.has_attachments);
    }

    #[test]
    fn snippets_collapse_whitespace_and_truncate_safely() {
        let long = "ä".repeat(400);
        let raw = format!("Subject: s\r\n\r\n{long}\r\n");
        let m = parse_message(raw.as_bytes(), None).unwrap();
        // Truncation must land on a character boundary, not a UTF-8 byte.
        assert!(m.snippet.unwrap().chars().count() <= SNIPPET_LEN + 1);
    }
}
