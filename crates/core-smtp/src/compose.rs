//! Building an outgoing message.
//!
//! Pure and synchronous: no network, no clock beyond the Message-ID and the
//! `Date` header. Everything that is easy to get subtly wrong about outgoing
//! mail — Bcc disclosure, threading headers, header injection, `Re:` stacking —
//! is decided here, where it can be tested without a server.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use mail_builder::headers::address::Address;
use mail_builder::MessageBuilder;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error("a message needs at least one recipient")]
    NoRecipients,

    #[error("{0:?} is not a usable email address")]
    InvalidAddress(String),

    #[error("the {header} header contains a line break, which would forge headers")]
    HeaderInjection { header: &'static str },

    #[error("serialising the message: {0}")]
    Serialise(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ComposeError>;

/// A display name and an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub name: Option<String>,
    pub address: String,
}

impl Mailbox {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            name: None,
            address: address.into(),
        }
    }

    pub fn named(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            address: address.into(),
        }
    }

    /// Parses `Jane Doe <jane@example.com>`, `<jane@example.com>` or a bare
    /// `jane@example.com`.
    ///
    /// Not a full RFC 5322 address parser — it does not handle groups, comments
    /// or quoted-string local parts with embedded angle brackets. It handles
    /// what a person types on a command line or into a To: field, and
    /// [`Draft::build`] rejects anything that slips through as malformed.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();

        let Some(open) = input.rfind('<') else {
            return Ok(Self::new(input));
        };
        let Some(close) = input.rfind('>') else {
            return Err(ComposeError::InvalidAddress(input.to_string()));
        };
        if close < open {
            return Err(ComposeError::InvalidAddress(input.to_string()));
        }

        let address = input[open + 1..close].trim();
        let name = input[..open].trim().trim_matches('"').trim();

        Ok(Self {
            name: (!name.is_empty()).then(|| name.to_string()),
            address: address.to_string(),
        })
    }

    /// Lowercased address, for the comparisons that decide who is already a
    /// recipient. Case-insensitive across the whole address rather than only
    /// the domain: local parts are technically case-sensitive, but no provider
    /// treats them that way, and being wrong here means mailing someone twice.
    fn key(&self) -> String {
        self.address.to_lowercase()
    }

    fn validate(&self, header: &'static str) -> Result<()> {
        if let Some(name) = &self.name {
            if name.chars().any(is_header_break) {
                return Err(ComposeError::HeaderInjection { header });
            }
        }
        validate_address(&self.address)
    }

    fn to_builder(&self) -> Address<'static> {
        match &self.name {
            Some(name) => Address::new_address(Some(name.clone()), self.address.clone()),
            None => Address::new_address(None::<String>, self.address.clone()),
        }
    }
}

impl std::fmt::Display for Mailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} <{}>", self.address),
            None => write!(f, "<{}>", self.address),
        }
    }
}

fn is_header_break(c: char) -> bool {
    c == '\r' || c == '\n'
}

/// Rejects anything that is not a plain `local@domain`.
///
/// This is a security check, not a politeness check. `mail-builder` writes the
/// address into the header verbatim — unlike display names, which it RFC 2047
/// encodes — so a CR or LF here would let the caller forge headers, and a comma
/// would silently add a recipient. Being conservative costs us exotic-but-legal
/// addresses that no personal mailbox uses.
fn validate_address(address: &str) -> Result<()> {
    let invalid = || ComposeError::InvalidAddress(address.to_string());

    let (local, domain) = address.split_once('@').ok_or_else(invalid)?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(invalid());
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err(invalid());
    }
    if address
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace() || "<>,;:\"\\".contains(c))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Whether a reply goes to the sender alone or to everyone on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyMode {
    Sender,
    All,
}

/// The parts of a received message that a reply or forward is built from.
///
/// Plain data rather than a `mail_parser::Message`, for the same reason
/// `core-store::model` is: it keeps the composer testable without constructing
/// a MIME document, and keeps the parser out of this crate's public API.
#[derive(Debug, Clone, Default)]
pub struct ReplySource {
    /// Message-ID of the original, without angle brackets.
    pub message_id: Option<String>,
    /// The original's References chain, oldest first, without angle brackets.
    pub references: Vec<String>,
    pub subject: Option<String>,
    pub from: Option<Mailbox>,
    /// Honoured ahead of `from` when replying, per RFC 5322 section 3.6.2.
    pub reply_to: Option<Mailbox>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    /// Unix seconds, UTC, for the attribution line.
    pub date_utc: Option<i64>,
    /// Plain-text body of the original, for quoting.
    pub body: Option<String>,
}

/// An outgoing message, before it is serialised.
#[derive(Debug, Clone)]
pub struct Draft {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    /// text/plain body. HTML composition is an explicit non-goal.
    pub body: String,
    /// Message-ID being replied to, without angle brackets.
    pub in_reply_to: Option<String>,
    /// References chain, oldest first, without angle brackets.
    pub references: Vec<String>,
}

impl Draft {
    pub fn new(from: Mailbox) -> Self {
        Self {
            from,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: String::new(),
            in_reply_to: None,
            references: Vec::new(),
        }
    }

    pub fn to(mut self, mailbox: Mailbox) -> Self {
        self.to.push(mailbox);
        self
    }

    pub fn cc(mut self, mailbox: Mailbox) -> Self {
        self.cc.push(mailbox);
        self
    }

    pub fn bcc(mut self, mailbox: Mailbox) -> Self {
        self.bcc.push(mailbox);
        self
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Builds a reply: recipients, `Re:` subject, threading headers and a
    /// quoted body.
    pub fn reply(from: Mailbox, source: &ReplySource, mode: ReplyMode) -> Self {
        let mut draft = Self::new(from);

        // Reply-To wins over From when the sender asked for it.
        let author = source.reply_to.clone().or_else(|| source.from.clone());

        // Never address ourselves, and never the same person twice — a
        // reply-all that CCs you on your own reply is the classic annoyance.
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(draft.from.key());

        if let Some(author) = author.clone() {
            if seen.insert(author.key()) {
                draft.to.push(author);
            }
        }

        if mode == ReplyMode::All {
            for mailbox in &source.to {
                if seen.insert(mailbox.key()) {
                    draft.to.push(mailbox.clone());
                }
            }
            for mailbox in &source.cc {
                if seen.insert(mailbox.key()) {
                    draft.cc.push(mailbox.clone());
                }
            }
        }

        draft.subject = reply_subject(source.subject.as_deref().unwrap_or_default());
        draft.body = quote(source, author.as_ref());

        // RFC 5322 section 3.6.4: References is the parent's References plus
        // the parent's Message-ID; In-Reply-To is the parent's Message-ID.
        draft.in_reply_to = source.message_id.clone();
        draft.references = trim_references(&source.references, source.message_id.as_deref());

        draft
    }

    /// Builds a forward. Recipients are left to the caller — a forward has no
    /// implied audience.
    pub fn forward(from: Mailbox, source: &ReplySource) -> Self {
        let mut draft = Self::new(from);
        draft.subject = forward_subject(source.subject.as_deref().unwrap_or_default());
        draft.body = forwarded_body(source);
        // A forward is a new message, not a reply: it carries no In-Reply-To.
        // References is kept so clients that thread on it can still relate the
        // forward to the conversation it came from.
        draft.references = trim_references(&source.references, source.message_id.as_deref());
        draft
    }

    /// Serialises the draft and works out the SMTP envelope.
    ///
    /// The two are computed together because they must not disagree: the
    /// envelope decides who receives the message, the headers decide who the
    /// recipients can see, and Bcc is precisely the case where those differ.
    pub fn build(&self) -> Result<BuiltMessage> {
        self.from.validate("From")?;
        for mailbox in &self.to {
            mailbox.validate("To")?;
        }
        for mailbox in &self.cc {
            mailbox.validate("Cc")?;
        }
        for mailbox in &self.bcc {
            mailbox.validate("Bcc")?;
        }
        if self.subject.chars().any(is_header_break) {
            return Err(ComposeError::HeaderInjection { header: "Subject" });
        }

        // Deduplicated across all three header fields: somebody who is both a
        // To and a Bcc must still get exactly one copy.
        let mut seen = HashSet::new();
        let recipients: Vec<String> = self
            .to
            .iter()
            .chain(&self.cc)
            .chain(&self.bcc)
            .filter(|mailbox| seen.insert(mailbox.key()))
            .map(|mailbox| mailbox.address.clone())
            .collect();

        if recipients.is_empty() {
            return Err(ComposeError::NoRecipients);
        }

        let domain = self
            .from
            .address
            .rsplit_once('@')
            .map(|(_, domain)| domain)
            .unwrap_or("localhost");
        let message_id = generate_message_id(domain);

        let mut builder = MessageBuilder::new()
            .message_id(message_id.clone())
            .from(self.from.to_builder())
            .subject(self.subject.clone())
            // RFC 5322 wants CRLF line endings, and a bare LF in the DATA
            // payload is not something to leave to the transport to notice.
            .text_body(normalize_newlines(&self.body));

        if !self.to.is_empty() {
            builder = builder.to(address_list(&self.to));
        }
        if !self.cc.is_empty() {
            builder = builder.cc(address_list(&self.cc));
        }

        // Bcc is deliberately absent from the headers. `mail-builder` would
        // happily write it, and `mail-send`'s own MessageBuilder envelope
        // conversion reads Bcc into the envelope *and* leaves the header in the
        // body — which discloses every blind recipient to everyone. Building
        // the envelope ourselves above is what avoids that.

        if let Some(parent) = &self.in_reply_to {
            builder = builder.in_reply_to(parent.clone());
        }
        if !self.references.is_empty() {
            builder = builder.references(self.references.clone());
        }

        Ok(BuiltMessage {
            message_id,
            rfc822: builder.write_to_vec()?,
            recipients,
            sender: self.from.address.clone(),
        })
    }
}

/// A serialised message and the envelope it must be submitted with.
#[derive(Debug, Clone)]
pub struct BuiltMessage {
    /// The Message-ID this message was assigned, without angle brackets. Worth
    /// keeping: it is what the store dedups on when the message comes back
    /// round from the Sent folder on the next sync.
    pub message_id: String,
    /// The complete RFC 5322 message, as submitted and as appended to Sent.
    pub rfc822: Vec<u8>,
    /// SMTP envelope recipients: To, Cc and Bcc, deduplicated. Not derivable
    /// from `rfc822`, which carries no Bcc header.
    pub recipients: Vec<String>,
    /// SMTP envelope sender, i.e. `MAIL FROM`. The From address: this client
    /// sends on behalf of one mailbox at a time, so there is no case where the
    /// envelope sender and the header sender differ.
    pub sender: String,
}

fn address_list(mailboxes: &[Mailbox]) -> Address<'static> {
    Address::new_list(
        mailboxes
            .iter()
            .map(Mailbox::to_builder)
            .collect::<Vec<_>>(),
    )
}

fn normalize_newlines(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Reply prefixes to collapse rather than stack.
///
/// German is here because the mailbox is bilingual and Outlook writes `AW:`;
/// without it a thread accumulates `Re: AW: Re: AW:`.
const REPLY_PREFIXES: &[&str] = &["re:", "aw:", "antw:"];
const FORWARD_PREFIXES: &[&str] = &["fwd:", "fw:", "wg:"];

fn reply_subject(subject: &str) -> String {
    format!("Re: {}", strip_prefixes(subject, REPLY_PREFIXES))
}

fn forward_subject(subject: &str) -> String {
    format!("Fwd: {}", strip_prefixes(subject, FORWARD_PREFIXES))
}

/// Strips every leading occurrence of any of `prefixes`, in any order.
///
/// A subject that has been round the houses arrives as `AW: Re: Fwd: …`; only
/// stripping the first one leaves the rest to accumulate forever.
fn strip_prefixes(subject: &str, prefixes: &[&str]) -> String {
    let mut rest = subject.trim();
    'outer: loop {
        for prefix in prefixes {
            if rest.len() >= prefix.len() && rest[..prefix.len()].eq_ignore_ascii_case(prefix) {
                rest = rest[prefix.len()..].trim_start();
                continue 'outer;
            }
        }
        break;
    }
    rest.to_string()
}

/// The References chain, capped.
///
/// RFC 5322 allows this to grow without bound, and on a long thread it does —
/// some servers then reject the message for an oversized header. The
/// widely-used mitigation is to keep the first reference, which identifies the
/// thread root, and the most recent ones, which is what clients thread on.
const MAX_REFERENCES: usize = 20;

fn trim_references(parent_references: &[String], parent_id: Option<&str>) -> Vec<String> {
    let mut chain: Vec<String> = parent_references.to_vec();
    if let Some(id) = parent_id {
        if chain.last().map(String::as_str) != Some(id) {
            chain.push(id.to_string());
        }
    }

    if chain.len() > MAX_REFERENCES {
        let tail = chain.split_off(chain.len() - (MAX_REFERENCES - 1));
        chain.truncate(1);
        chain.extend(tail);
    }
    chain
}

fn quote(source: &ReplySource, author: Option<&Mailbox>) -> String {
    let mut out = String::new();
    out.push_str("\n\n");
    out.push_str(&attribution(source, author));
    out.push('\n');

    for line in source.body.as_deref().unwrap_or_default().lines() {
        // No space after `>` on an already-quoted line, so nesting reads as
        // `>>` rather than `> > `.
        if line.starts_with('>') {
            out.push('>');
        } else {
            out.push_str("> ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn attribution(source: &ReplySource, author: Option<&Mailbox>) -> String {
    let who = author
        .map(Mailbox::to_string)
        .unwrap_or_else(|| "you".to_string());

    match source.date_utc.and_then(local_date) {
        Some(when) => format!("On {when}, {who} wrote:"),
        None => format!("{who} wrote:"),
    }
}

fn local_date(timestamp: i64) -> Option<String> {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(timestamp, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.format("%a, %-d %b %Y at %H:%M").to_string()),
        _ => None,
    }
}

fn forwarded_body(source: &ReplySource) -> String {
    let mut out = String::from("\n\n---------- Forwarded message ----------\n");
    if let Some(from) = &source.from {
        out.push_str(&format!("From: {from}\n"));
    }
    if let Some(when) = source.date_utc.and_then(local_date) {
        out.push_str(&format!("Date: {when}\n"));
    }
    if let Some(subject) = &source.subject {
        out.push_str(&format!("Subject: {subject}\n"));
    }
    if !source.to.is_empty() {
        let to: Vec<String> = source.to.iter().map(Mailbox::to_string).collect();
        out.push_str(&format!("To: {}\n", to.join(", ")));
    }
    out.push('\n');
    // Forwarded text is reproduced as sent, not quoted: the point of a forward
    // is that the recipient sees the original.
    out.push_str(source.body.as_deref().unwrap_or_default());
    out
}

/// Generates `<opaque@domain>`.
///
/// The domain is the sender's, not the local hostname: `mail-builder`'s
/// fallback would put this machine's name into every outgoing message, which
/// leaks something about the sender and is worthless for threading. The local
/// part carries no meaning by design — RFC 5322 only requires global
/// uniqueness, and anything readable here is metadata given away for free.
fn generate_message_id(domain: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(SEQUENCE.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();

    let mut local = String::with_capacity(32);
    for byte in &digest[..16] {
        local.push_str(&format!("{byte:02x}"));
    }
    format!("{local}@{domain}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Mailbox {
        Mailbox::named("Erika", "erika@fuckmail.test")
    }

    fn thread() -> ReplySource {
        ReplySource {
            message_id: Some("parent-1@example.com".into()),
            references: vec!["root-0@example.com".into()],
            subject: Some("Rechnung 2026-09".into()),
            from: Some(Mailbox::named("Jane Doe", "jane@example.com")),
            to: vec![me(), Mailbox::named("Bob", "bob@example.com")],
            cc: vec![Mailbox::new("carol@example.com")],
            date_utc: Some(1_757_000_000),
            body: Some("first line\nsecond line".into()),
            ..Default::default()
        }
    }

    fn header_block(built: &BuiltMessage) -> String {
        let text = String::from_utf8(built.rfc822.clone()).unwrap();
        text.split("\r\n\r\n").next().unwrap().to_string()
    }

    #[test]
    fn bcc_recipients_are_in_the_envelope_but_never_in_the_message() {
        // The one mistake in this crate that cannot be walked back: once a
        // blind recipient's address has been transmitted to the others, it has
        // been disclosed. mail-send's own builder-to-envelope conversion gets
        // this wrong, which is why Draft::build does it by hand.
        let built = Draft::new(me())
            .to(Mailbox::new("jane@example.com"))
            .bcc(Mailbox::new("secret@example.com"))
            .subject("Hallo")
            .body("hi")
            .build()
            .unwrap();

        let rendered = String::from_utf8(built.rfc822.clone()).unwrap();
        assert!(!rendered.contains("secret@example.com"));
        assert!(!rendered.to_lowercase().contains("bcc"));

        assert!(built.recipients.contains(&"secret@example.com".to_string()));
        assert!(built.recipients.contains(&"jane@example.com".to_string()));
    }

    #[test]
    fn a_recipient_listed_twice_is_sent_one_copy() {
        let built = Draft::new(me())
            .to(Mailbox::new("jane@example.com"))
            .cc(Mailbox::new("JANE@example.com"))
            .bcc(Mailbox::new("jane@Example.com"))
            .subject("Hallo")
            .build()
            .unwrap();

        assert_eq!(built.recipients, vec!["jane@example.com"]);
    }

    #[test]
    fn a_message_with_no_recipients_is_refused() {
        let draft = Draft::new(me()).subject("into the void");
        assert!(matches!(draft.build(), Err(ComposeError::NoRecipients)));
    }

    #[test]
    fn a_line_break_in_a_header_is_refused_rather_than_written() {
        // mail-builder writes addresses into the header verbatim, so this is
        // the difference between a rejected draft and a forged Bcc.
        let injected = Draft::new(me())
            .to(Mailbox::new("jane@example.com\r\nBcc: victim@example.com"))
            .subject("Hallo")
            .build();
        assert!(matches!(injected, Err(ComposeError::InvalidAddress(_))));

        let subject = Draft::new(me())
            .to(Mailbox::new("jane@example.com"))
            .subject("Hallo\r\nBcc: victim@example.com")
            .build();
        assert!(matches!(
            subject,
            Err(ComposeError::HeaderInjection { header: "Subject" })
        ));

        let name = Draft::new(me())
            .to(Mailbox::named(
                "Jane\r\nBcc: victim@example.com",
                "jane@example.com",
            ))
            .subject("Hallo")
            .build();
        assert!(matches!(
            name,
            Err(ComposeError::HeaderInjection { header: "To" })
        ));
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for bad in [
            "not-an-address",
            "@example.com",
            "jane@",
            "jane@localhost",
            "jane@@example.com",
            "jane doe@example.com",
            "jane@example.com, other@example.com",
        ] {
            let result = Draft::new(me()).to(Mailbox::new(bad)).build();
            assert!(
                matches!(result, Err(ComposeError::InvalidAddress(_))),
                "{bad:?} should have been refused, got {result:?}"
            );
        }
    }

    #[test]
    fn a_reply_goes_to_the_author_and_a_reply_all_to_everyone_but_us() {
        let source = thread();

        let one = Draft::reply(me(), &source, ReplyMode::Sender);
        assert_eq!(one.to, vec![Mailbox::named("Jane Doe", "jane@example.com")]);
        assert!(one.cc.is_empty());

        let all = Draft::reply(me(), &source, ReplyMode::All);
        let to: Vec<&str> = all.to.iter().map(|m| m.address.as_str()).collect();
        // Jane first, then the original To — minus ourselves.
        assert_eq!(to, vec!["jane@example.com", "bob@example.com"]);
        assert_eq!(all.cc.len(), 1);
        assert_eq!(all.cc[0].address, "carol@example.com");
        assert!(!to.contains(&"erika@fuckmail.test"));
    }

    #[test]
    fn a_reply_honours_reply_to_over_from() {
        let source = ReplySource {
            reply_to: Some(Mailbox::new("list@example.com")),
            ..thread()
        };
        let draft = Draft::reply(me(), &source, ReplyMode::Sender);
        assert_eq!(draft.to[0].address, "list@example.com");
    }

    #[test]
    fn reply_prefixes_do_not_stack() {
        assert_eq!(reply_subject("Rechnung"), "Re: Rechnung");
        assert_eq!(reply_subject("Re: Rechnung"), "Re: Rechnung");
        assert_eq!(reply_subject("RE: Rechnung"), "Re: Rechnung");
        // Outlook's German reply prefix, and a subject that has been through
        // several clients.
        assert_eq!(reply_subject("AW: Rechnung"), "Re: Rechnung");
        assert_eq!(reply_subject("AW: Re: AW: Rechnung"), "Re: Rechnung");
        // A forward prefix is not a reply prefix and must survive.
        assert_eq!(reply_subject("Fwd: Rechnung"), "Re: Fwd: Rechnung");
        assert_eq!(forward_subject("WG: Rechnung"), "Fwd: Rechnung");
    }

    #[test]
    fn a_reply_carries_the_threading_headers() {
        let draft = Draft::reply(me(), &thread(), ReplyMode::Sender);
        assert_eq!(draft.in_reply_to.as_deref(), Some("parent-1@example.com"));
        assert_eq!(
            draft.references,
            vec!["root-0@example.com", "parent-1@example.com"]
        );

        let built = draft.to(Mailbox::new("jane@example.com")).build().unwrap();
        let headers = header_block(&built);
        assert!(headers.contains("In-Reply-To: <parent-1@example.com>"));
        assert!(headers.contains("References: <root-0@example.com> <parent-1@example.com>"));
    }

    #[test]
    fn a_long_reference_chain_keeps_the_root_and_the_recent_end() {
        let refs: Vec<String> = (0..40).map(|n| format!("m{n}@example.com")).collect();
        let trimmed = trim_references(&refs, Some("parent@example.com"));

        assert_eq!(trimmed.len(), MAX_REFERENCES);
        // The thread root is what identifies the conversation.
        assert_eq!(trimmed[0], "m0@example.com");
        // And the parent is what the recipient's client threads on.
        assert_eq!(trimmed[trimmed.len() - 1], "parent@example.com");
    }

    #[test]
    fn a_parent_already_in_the_chain_is_not_repeated() {
        let trimmed = trim_references(
            &["root@example.com".into(), "parent@example.com".into()],
            Some("parent@example.com"),
        );
        assert_eq!(trimmed, vec!["root@example.com", "parent@example.com"]);
    }

    #[test]
    fn a_reply_quotes_the_original_under_an_attribution() {
        let draft = Draft::reply(me(), &thread(), ReplyMode::Sender);
        assert!(draft.body.contains("Jane Doe <jane@example.com> wrote:"));
        assert!(draft.body.contains("\n> first line\n"));
        assert!(draft.body.contains("\n> second line\n"));
        // The composer leaves the cursor above the quote.
        assert!(draft.body.starts_with("\n\n"));
    }

    #[test]
    fn nested_quoting_does_not_grow_spaces() {
        let source = ReplySource {
            body: Some("> already quoted".into()),
            ..thread()
        };
        let draft = Draft::reply(me(), &source, ReplyMode::Sender);
        assert!(draft.body.contains("\n>> already quoted\n"));
    }

    #[test]
    fn a_forward_reproduces_the_original_rather_than_quoting_it() {
        let draft = Draft::forward(me(), &thread());
        assert_eq!(draft.subject, "Fwd: Rechnung 2026-09");
        assert!(draft
            .body
            .contains("---------- Forwarded message ----------"));
        assert!(draft.body.contains("From: Jane Doe <jane@example.com>"));
        assert!(draft.body.contains("first line\nsecond line"));
        assert!(!draft.body.contains("> first line"));
        // A forward is a new message; it is not a reply to the original.
        assert!(draft.in_reply_to.is_none());
        assert!(draft.to.is_empty());
    }

    #[test]
    fn the_message_id_is_unique_and_uses_the_senders_domain() {
        let build = || {
            Draft::new(me())
                .to(Mailbox::new("jane@example.com"))
                .build()
                .unwrap()
        };
        let first = build();
        let second = build();

        assert!(first.message_id.ends_with("@fuckmail.test"));
        assert_ne!(first.message_id, second.message_id);
        // The local hostname must not leak into outgoing mail.
        let rendered = String::from_utf8(first.rfc822.clone()).unwrap();
        assert!(rendered.contains(&format!("Message-ID: <{}>", first.message_id)));
    }

    #[test]
    fn the_body_is_serialised_with_crlf_line_endings() {
        let built = Draft::new(me())
            .to(Mailbox::new("jane@example.com"))
            .body("one\ntwo\r\nthree")
            .build()
            .unwrap();
        let rendered = String::from_utf8(built.rfc822).unwrap();
        let body = rendered.split("\r\n\r\n").nth(1).unwrap();
        assert!(body.contains("one\r\ntwo\r\nthree"));
        // No bare LF anywhere: SMTP DATA is a CRLF protocol.
        assert!(!body.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn mailboxes_parse_from_what_a_person_types() {
        assert_eq!(
            Mailbox::parse("Jane Doe <jane@example.com>").unwrap(),
            Mailbox::named("Jane Doe", "jane@example.com")
        );
        assert_eq!(
            Mailbox::parse("  jane@example.com ").unwrap(),
            Mailbox::new("jane@example.com")
        );
        assert_eq!(
            Mailbox::parse("<jane@example.com>").unwrap(),
            Mailbox::new("jane@example.com")
        );
        assert_eq!(
            Mailbox::parse("\"Doe, Jane\" <jane@example.com>").unwrap(),
            Mailbox::named("Doe, Jane", "jane@example.com")
        );
        assert!(Mailbox::parse("Jane <jane@example.com").is_err());
    }
}
