//! MIME message -> `NewMessage`.
//!
//! Parsing is delegated entirely to `mail-parser`: hand-rolled MIME handling is
//! the classic source of memory-safety bugs in mail clients, and this is the
//! best-tested implementation in the Rust ecosystem.

use core_rules::MessageFacts;
use core_store::model::NewMessage;
use mail_parser::{Address, HeaderName, MessageParser};

/// Everything the classifier needs, owned, so it outlives the borrowed parse.
///
/// Kept next to parsing because that is the one place the whole message is in
/// hand; re-reading these headers from the store later would mean persisting
/// them for no other purpose.
#[derive(Debug, Clone, Default)]
pub struct ClassifyFacts {
    pub from_addr: Option<String>,
    pub from_name: Option<String>,
    pub subject: Option<String>,
    pub list_id: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub precedence: Option<String>,
    pub auto_submitted: Option<String>,
    pub in_reply_to: Option<String>,
    pub has_attachments: bool,
    pub recipient_count: usize,
    pub snippet: Option<String>,
}

impl ClassifyFacts {
    pub fn as_message_facts(&self) -> MessageFacts<'_> {
        MessageFacts {
            from_addr: self.from_addr.as_deref(),
            from_name: self.from_name.as_deref(),
            subject: self.subject.as_deref(),
            list_id: self.list_id.as_deref(),
            list_unsubscribe: self.list_unsubscribe.as_deref(),
            precedence: self.precedence.as_deref(),
            auto_submitted: self.auto_submitted.as_deref(),
            in_reply_to: self.in_reply_to.as_deref(),
            has_attachments: self.has_attachments,
            recipient_count: self.recipient_count,
            snippet: self.snippet.as_deref(),
        }
    }
}

/// A parsed message plus the facts needed to classify it.
pub struct Parsed {
    pub message: NewMessage,
    pub facts: ClassifyFacts,
}

/// Longest snippet kept for the message list. Enough for a useful preview
/// without pulling whole bodies into the list query.
const SNIPPET_LEN: usize = 220;

/// Parses a raw RFC 5322 message.
///
/// Returns `None` only when the bytes are not a message at all. Malformed but
/// recognisable mail still parses — two decades of broken senders mean strict
/// parsing would reject real mail.
pub fn parse_message(raw: &[u8], size: Option<i64>) -> Option<Parsed> {
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

    // Recipients across To and Cc, for the "addressed only to you" signal.
    let recipient_count = count_addresses(parsed.to()) + count_addresses(parsed.cc());

    let list_id = parsed.header_raw("List-Id").map(clean_list_id);
    let in_reply_to = parsed
        .header_raw("In-Reply-To")
        .map(|v| core_store::normalize_message_id(v).to_string());
    // Bound to a local: the iterator borrows `parsed`, and as a temporary
    // inside the struct literal it would outlive it.
    let has_attachments = parsed.attachments().next().is_some();

    let message = NewMessage {
        rfc822_message_id: parsed.message_id().map(str::to_string),
        subject: parsed.subject().map(str::to_string),
        from_name: from_name.clone(),
        from_addr: from_addr.clone(),
        date_utc: parsed.date().map(|d| d.to_timestamp()),
        size_bytes: size.or(Some(raw.len() as i64)),
        snippet: body_text.as_deref().map(snippet),
        has_attachments,
        list_id: list_id.clone(),
        in_reply_to: in_reply_to.clone(),
        body_path: None,
        search_text: body_text.clone(),
    };

    let facts = ClassifyFacts {
        from_addr,
        from_name,
        subject: message.subject.clone(),
        list_id,
        list_unsubscribe: header_text(&parsed, "List-Unsubscribe"),
        precedence: header_text(&parsed, "Precedence"),
        auto_submitted: header_text(&parsed, "Auto-Submitted"),
        in_reply_to,
        has_attachments,
        recipient_count,
        snippet: message.snippet.clone(),
    };

    Some(Parsed { message, facts })
}

fn count_addresses(address: Option<&Address<'_>>) -> usize {
    match address {
        Some(Address::List(addrs)) => addrs.len(),
        Some(Address::Group(groups)) => groups.iter().map(|g| g.addresses.len()).sum(),
        None => 0,
    }
}

fn header_text(message: &mail_parser::Message<'_>, name: &str) -> Option<String> {
    message
        .header(HeaderName::parse(name)?)
        .and_then(|h| h.as_text())
        .map(|s| s.trim().to_string())
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
        let m = parse_message(raw, None).unwrap().message;

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
        let m = parse_message(raw, None).unwrap().message;
        assert_eq!(m.subject.as_deref(), Some("Ihre Abschlagszahlung für März"));
    }

    #[test]
    fn extracts_the_bracketed_list_id() {
        let raw = b"Message-ID: <n@example>\r\n\
                    List-Id: Rust Weekly <news.rustweekly.example>\r\n\
                    \r\n\
                    body\r\n";
        let m = parse_message(raw, None).unwrap().message;
        assert_eq!(m.list_id.as_deref(), Some("news.rustweekly.example"));
    }

    #[test]
    fn a_missing_message_id_is_not_an_error() {
        let raw = b"From: cron@example.org\r\nSubject: nightly\r\n\r\nok\r\n";
        let m = parse_message(raw, None).unwrap().message;
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
        let m = parse_message(raw, None).unwrap().message;
        assert!(m.has_attachments);
    }

    #[test]
    fn snippets_collapse_whitespace_and_truncate_safely() {
        let long = "ä".repeat(400);
        let raw = format!("Subject: s\r\n\r\n{long}\r\n");
        let m = parse_message(raw.as_bytes(), None).unwrap().message;
        // Truncation must land on a character boundary, not a UTF-8 byte.
        assert!(m.snippet.unwrap().chars().count() <= SNIPPET_LEN + 1);
    }
}
