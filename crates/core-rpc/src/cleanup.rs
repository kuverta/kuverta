//! Cleanup: getting bulk mail out of the way, and out of the mailbox for good.
//!
//! Three things, each answering a question the Inbox raises:
//!
//! - **How much is there to clear?** [`Core::cleanup_counts`] — bulk mail
//!   still in the Inbox, and how many senders could be unsubscribed from.
//! - **Can I stop it coming?** [`Core::unsubscribe_senders`] lists everyone
//!   whose mail carries `List-Unsubscribe`, and [`Session::unsubscribe`] does
//!   it: RFC 8058's one-click POST where the sender offers it, a mail to the
//!   address it names where it offers that, and otherwise a link for the
//!   person to open. Nothing here ever follows an ordinary unsubscribe link on
//!   its own: a GET to one proves the address is read, and many of them land
//!   on a page that wants a click anyway. RFC 8058 exists because that was
//!   unsafe to automate.
//! - **Is this one of many?** [`Core::similar`] finds the messages that look
//!   like one just deleted — the same sender with the same kind of subject,
//!   the same list, or the same text from different senders, which is what
//!   spam looks like — so they can go together.

use std::collections::HashSet;
use std::time::Duration;

use core_store::model::{AccountId, MessageId};
use core_store::{SimilarityRow, SmartField, SmartOp, SmartQuery, SmartRule, Unsubscription};
use serde::{Deserialize, Serialize};

use crate::session::{DraftInput, Session};
use crate::{Core, Result, RpcError};

/// What the header next to Cleanup counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupCounts {
    /// Newsletters, marketing and notifications still in the Inbox.
    pub bulk_in_inbox: usize,
    /// Senders who can be unsubscribed from and have not been yet.
    pub unsubscribable: usize,
}

/// How a sender can be unsubscribed from, best first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnsubscribeMethod {
    /// RFC 8058: a POST to this address unsubscribes, no questions asked.
    OneClick { url: String },
    /// A mail to this address unsubscribes.
    Mailto {
        address: String,
        subject: String,
        body: String,
    },
    /// Only a web page, which a person has to open.
    Browser { url: String },
}

impl UnsubscribeMethod {
    pub fn automatic(&self) -> bool {
        !matches!(self, Self::Browser { .. })
    }

    fn name(&self) -> &'static str {
        match self {
            Self::OneClick { .. } => "one_click",
            Self::Mailto { .. } => "mailto",
            Self::Browser { .. } => "browser",
        }
    }

    fn target(&self) -> String {
        match self {
            Self::OneClick { url } | Self::Browser { url } => url.clone(),
            Self::Mailto { address, .. } => format!("mailto:{address}"),
        }
    }
}

/// The best way to unsubscribe, from the two headers a sender provides.
///
/// Returns `None` when neither holds anything usable — a header of only
/// unknown schemes is the same as no header.
pub fn unsubscribe_method(header: &str, post: Option<&str>) -> Option<UnsubscribeMethod> {
    unsubscribe_method_from(header, post, None)
}

/// As [`unsubscribe_method`], for mail from `sender` — a From address or a
/// `List-Id`. A mail unsubscribe is only offered when its address belongs to
/// the sender: the header is whatever the sender wrote, and a forged
/// newsletter could otherwise have this account mail anyone anything. With
/// no sender given, as in [`unsubscribe_method`], no mailto is trusted.
pub fn unsubscribe_method_from(
    header: &str,
    post: Option<&str>,
    sender: Option<&[&str]>,
) -> Option<UnsubscribeMethod> {
    let uris: Vec<&str> = header
        .split(',')
        .map(|part| {
            part.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
        })
        .filter(|uri| !uri.is_empty())
        .collect();
    let https = uris.iter().find(|uri| {
        uri.get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    });
    let http = uris.iter().find(|uri| {
        uri.get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
    });
    let mailto = uris.iter().find(|uri| {
        uri.get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
    });
    // RFC 8058 §3.1: the POST header must say exactly this, and the target
    // must be HTTPS. Anything else is not a promise that a POST is enough.
    let one_click = post.is_some_and(|value| {
        value.split(';').any(|part| {
            part.trim()
                .eq_ignore_ascii_case("List-Unsubscribe=One-Click")
        })
    });

    let https = https.filter(|url| public_host(url));
    let http = http.filter(|url| public_host(url));
    if let (true, Some(url)) = (one_click, https) {
        return Some(UnsubscribeMethod::OneClick {
            url: url.to_string(),
        });
    }
    if let Some(uri) = mailto.filter(|_| sender.is_some()) {
        let rest = &uri[7..];
        let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
        let address = percent_decode(address);
        let belongs = address
            .rsplit_once('@')
            .map(|(_, domain)| registrable(&domain.to_lowercase()))
            .is_some_and(|domain| {
                sender.unwrap_or_default().iter().any(|own| {
                    let own = own.rsplit_once('@').map_or(*own, |(_, d)| d);
                    registrable(&own.to_lowercase()) == domain
                })
            });
        if belongs {
            let mut subject = "unsubscribe".to_string();
            let mut body = "unsubscribe".to_string();
            for pair in query.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    match key.to_ascii_lowercase().as_str() {
                        "subject" => subject = percent_decode(value),
                        "body" => body = percent_decode(value),
                        _ => {}
                    }
                }
            }
            return Some(UnsubscribeMethod::Mailto {
                address,
                subject,
                body,
            });
        }
    }
    https.or(http).map(|url| UnsubscribeMethod::Browser {
        url: url.to_string(),
    })
}

/// Whether a URL points somewhere on the internet rather than at this
/// computer or its network. A forged header must not be able to make kuverta
/// post to a router's admin page.
fn public_host(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_lowercase();
    let host = if host.starts_with('[') {
        host.as_str()
    } else {
        host.split(':').next().unwrap_or_default()
    };
    let literal = host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok();
    !(host.is_empty()
        || literal
        || !host.contains('.')
        || host == "localhost"
        || [
            ".local",
            ".lan",
            ".home",
            ".internal",
            ".localhost",
            ".corp",
            ".intranet",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix)))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A sender as the Unsubscribe list shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeSenderView {
    pub key: String,
    /// Who it is, as a person would name them.
    pub name: String,
    pub address: Option<String>,
    pub list_id: Option<String>,
    pub messages: usize,
    pub unread: usize,
    pub latest_utc: Option<i64>,
    pub latest_subject: Option<String>,
    pub latest_message: MessageId,
    pub method: UnsubscribeMethod,
    /// The last attempt, if there was one.
    pub last_attempt: Option<UnsubscribeAttemptView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeAttemptView {
    pub method: String,
    pub state: String,
    pub detail: Option<String>,
    pub at_utc: i64,
}

impl From<&Unsubscription> for UnsubscribeAttemptView {
    fn from(attempt: &Unsubscription) -> Self {
        Self {
            method: attempt.method.clone(),
            state: attempt.state.clone(),
            detail: attempt.detail.clone(),
            at_utc: attempt.created_at,
        }
    }
}

/// What happened when unsubscribing from one sender.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeResult {
    pub key: String,
    pub name: String,
    /// `done`, `open` (a page for the person to open — `url` says which) or
    /// `failed`.
    pub state: String,
    pub detail: Option<String>,
    pub url: Option<String>,
}

/// Messages that look like one, and a smart mailbox that would catch them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarReport {
    pub matches: Vec<SimilarMessage>,
    /// Rules for a smart mailbox gathering this kind of message, for the
    /// person to adjust before saving.
    pub suggestion: SmartSuggestion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarMessage {
    pub id: MessageId,
    pub from: String,
    pub subject: String,
    pub date_utc: Option<i64>,
    /// Why it counts as similar, in words.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartSuggestion {
    pub name: String,
    pub query: SmartQuery,
}

/// How many recent messages the similarity check reads. Enough to cover what
/// a spam run leaves in a mailbox; a scan of everything would make deleting a
/// message wait on a mailbox's worth of reading.
const SIMILARITY_WINDOW: usize = 5_000;

impl Core {
    pub fn cleanup_counts(&self, account: AccountId) -> Result<CleanupCounts> {
        let tried: HashSet<String> = self
            .store()
            .unsubscriptions(account)?
            .into_iter()
            .filter(|attempt| attempt.state == "done")
            .map(|attempt| attempt.sender)
            .collect();
        let unsubscribable = self
            .store()
            .unsubscribe_senders(account)?
            .into_iter()
            .filter(|sender| !tried.contains(&sender.key))
            .filter(|sender| {
                let own: Vec<&str> = [sender.from_addr.as_deref(), sender.list_id.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                unsubscribe_method_from(
                    &sender.list_unsubscribe,
                    sender.list_unsubscribe_post.as_deref(),
                    Some(&own),
                )
                .is_some()
            })
            .count();
        Ok(CleanupCounts {
            bulk_in_inbox: self.store().cleanup_count(account)?,
            unsubscribable,
        })
    }

    /// Everyone on the account who can be unsubscribed from.
    pub fn unsubscribe_senders(&self, account: AccountId) -> Result<Vec<UnsubscribeSenderView>> {
        let attempts = self.store().unsubscriptions(account)?;
        Ok(self
            .store()
            .unsubscribe_senders(account)?
            .into_iter()
            .filter_map(|sender| {
                let own: Vec<&str> = [sender.from_addr.as_deref(), sender.list_id.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                let method = unsubscribe_method_from(
                    &sender.list_unsubscribe,
                    sender.list_unsubscribe_post.as_deref(),
                    Some(&own),
                )?;
                let last_attempt = attempts
                    .iter()
                    .find(|attempt| attempt.sender == sender.key)
                    .map(UnsubscribeAttemptView::from);
                Some(UnsubscribeSenderView {
                    name: display_name(sender.from_name.as_deref(), sender.from_addr.as_deref()),
                    address: sender.from_addr,
                    list_id: sender.list_id,
                    messages: sender.messages,
                    unread: sender.unread,
                    latest_utc: sender.latest_utc,
                    latest_subject: sender.latest_subject,
                    latest_message: sender.latest_message,
                    key: sender.key,
                    method,
                    last_attempt,
                })
            })
            .collect())
    }

    /// Queues every message from a sender for the Trash, each with its own
    /// undo like any other move. Returns how many were queued.
    pub fn trash_from_sender(&self, account: AccountId, key: &str, trash: &str) -> Result<usize> {
        let mut queued = 0;
        for id in self.store().messages_from_sender(account, key)? {
            // Already in the Trash, or in no synced folder: nothing to move.
            match self.move_to(account, id, trash, crate::DEFAULT_UNDO_WINDOW_SECS) {
                Ok(_) => queued += 1,
                Err(RpcError::Rejected(_)) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(queued)
    }

    /// Records a link the person was sent to open, since that is as far as
    /// this can follow it.
    pub fn record_unsubscribe_opened(
        &self,
        account: AccountId,
        key: &str,
        url: &str,
    ) -> Result<()> {
        self.store().record_unsubscription(
            account,
            &Unsubscription {
                sender: key.to_string(),
                method: "browser".into(),
                target: url.to_string(),
                state: "opened".into(),
                detail: None,
                created_at: crate::now_utc(),
            },
        )?;
        Ok(())
    }

    /// Messages that look like `id`, and a smart mailbox that would gather
    /// them.
    ///
    /// Three kinds of likeness, strongest first. The same mailing list is the
    /// same thing by definition. The same sender with a subject built the same
    /// way — "Your order #1234 has shipped", "Your order #5678 has shipped" —
    /// is a series. And the same text from different senders is what a spam
    /// run looks like, since rotating the sender is the cheapest evasion there
    /// is and rewriting the text is not.
    pub fn similar(
        &self,
        account: AccountId,
        id: MessageId,
        trash: Option<&str>,
    ) -> Result<SimilarReport> {
        // Mail only somewhere the person put it themselves — the Trash, Sent,
        // Drafts — is not offered: it is not arriving, so it is not a run.
        let mut skip: Vec<String> = trash.map(str::to_string).into_iter().collect();
        for folder in self.store().folder_summaries(account)? {
            if matches!(folder.rank(), 1 | 2 | 6) {
                skip.push(folder.name);
            }
        }
        let target = self
            .store()
            .similarity_row(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;

        // A conversation is not a run. Deleting one message of it says
        // nothing about the rest, and offering to delete other people's
        // replies because they share a subject line would be the one mistake
        // here that costs someone real mail.
        if is_conversation(&target) {
            return Ok(SimilarReport {
                matches: Vec::new(),
                suggestion: suggest(&target, &[]),
            });
        }
        let candidates = self
            .store()
            .similarity_candidates(account, &skip, SIMILARITY_WINDOW)?;

        let shape = Shape::of(&target);
        let mut matches = Vec::new();
        let mut matched_rows = Vec::new();
        for row in candidates
            .iter()
            .filter(|row| row.id != id && !is_conversation(row))
        {
            if let Some(reason) = shape.likeness(&Shape::of(row)) {
                matches.push(SimilarMessage {
                    id: row.id,
                    from: display_name(row.from_name.as_deref(), row.from_addr.as_deref()),
                    subject: row.subject.clone().unwrap_or_else(|| "(no subject)".into()),
                    date_utc: row.date_utc,
                    reason,
                });
                matched_rows.push(row);
            }
        }

        Ok(SimilarReport {
            suggestion: suggest(&target, &matched_rows),
            matches,
        })
    }
}

/// Mail a person wrote, or a reply in a thread: never part of a run.
fn is_conversation(row: &SimilarityRow) -> bool {
    row.category.as_deref() == Some("personal")
        || row.in_reply_to.is_some()
        || row.subject.as_deref().is_some_and(|subject| {
            let subject = subject.trim_start().to_lowercase();
            ["re:", "aw:", "sv:", "antw:"]
                .iter()
                .any(|prefix| subject.starts_with(prefix))
        })
}

/// Domains shared by millions of unrelated people. Two senders there have
/// nothing in common but their provider.
const SHARED_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "yahoo.de",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "gmx.de",
    "gmx.net",
    "gmx.at",
    "gmx.ch",
    "web.de",
    "t-online.de",
    "freenet.de",
    "posteo.de",
    "mailbox.org",
    "proton.me",
    "protonmail.com",
    "tutanota.com",
    "tuta.io",
    "fastmail.com",
    "zoho.com",
    "yandex.com",
    "mail.ru",
    "arcor.de",
    "online.de",
    "1und1.de",
];

fn shared_domain(domain: &str) -> bool {
    SHARED_DOMAINS.contains(&domain)
}

/// A message reduced to what likeness is judged on.
struct Shape<'a> {
    address: Option<String>,
    domain: Option<String>,
    list: Option<&'a str>,
    subject: HashSet<String>,
    subject_line: String,
    text: HashSet<String>,
}

impl<'a> Shape<'a> {
    fn of(row: &'a SimilarityRow) -> Self {
        let address = row.from_addr.as_deref().map(str::to_lowercase);
        let domain = address
            .as_deref()
            .and_then(|a| a.rsplit_once('@'))
            .map(|(_, domain)| registrable(domain))
            .filter(|domain| !shared_domain(domain));
        let subject_line = normalise_subject(row.subject.as_deref().unwrap_or_default());
        Self {
            subject: words(&subject_line),
            subject_line,
            text: words(row.snippet.as_deref().unwrap_or_default()),
            list: row.list_id.as_deref().filter(|list| !list.is_empty()),
            address,
            domain,
        }
    }

    fn likeness(&self, other: &Shape<'_>) -> Option<String> {
        if let (Some(a), Some(b)) = (self.list, other.list) {
            if a == b {
                return Some("same mailing list".into());
            }
        }
        let subject = jaccard(&self.subject, &other.subject);
        let text = jaccard(&self.text, &other.text);
        let same_sender = self.address.is_some() && self.address == other.address;
        let same_domain = self.domain.is_some() && self.domain == other.domain;

        if same_sender && (subject >= 0.5 || text >= 0.6) {
            return Some("same sender, similar subject".into());
        }
        // From here on the senders differ, so neither a shared subject line
        // nor a shared domain is enough — "Rückfrage zum Angebot" is written
        // by many people, some of them colleagues. The text has to be alike.
        let same_subject = !self.subject_line.is_empty()
            && self.subject.len() >= 3
            && self.subject_line == other.subject_line;
        if same_subject && self.text.len() >= 5 && text >= 0.5 {
            return Some("same subject and text".into());
        }
        // Different senders, near-identical text: the spam run.
        if self.text.len() >= 8 && text >= 0.6 && (subject >= 0.3 || same_domain) {
            return Some("nearly the same text".into());
        }
        None
    }
}

/// `mail.news.example.co.uk` → `example.co.uk`, near enough: the last two
/// labels, or three under a short second-level label like `co.` or `com.`.
fn registrable(domain: &str) -> String {
    let labels: Vec<&str> = domain.trim_end_matches('.').split('.').collect();
    let keep = if labels.len() >= 3 && labels[labels.len() - 2].len() <= 3 {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

/// Lowercased, prefixes and numbers taken out: what stays the same across a
/// series whose every issue has a different order number or date.
fn normalise_subject(subject: &str) -> String {
    let mut text = subject.trim().to_lowercase();
    loop {
        let before = text.len();
        for prefix in ["re:", "fwd:", "fw:", "aw:", "wg:", "tr:", "sv:"] {
            if let Some(rest) = text.strip_prefix(prefix) {
                text = rest.trim_start().to_string();
            }
        }
        if text.len() == before {
            break;
        }
    }
    words(&text)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words of a text worth comparing: letters only, two or more of them.
/// Numbers go, because they are what changes between one issue and the next.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_lowercase)
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count() as f64;
    let either = a.union(b).count() as f64;
    shared / either
}

/// A smart mailbox that gathers the target and what matched it: the list if
/// they share one, else the sender or its domain, narrowed by the subject
/// words they all have.
fn suggest(target: &SimilarityRow, matched: &[&SimilarityRow]) -> SmartSuggestion {
    let everyone: Vec<&SimilarityRow> = std::iter::once(target)
        .chain(matched.iter().copied())
        .collect();
    let mut rules = Vec::new();
    let name;

    let list = target.list_id.as_deref().filter(|list| {
        !list.is_empty()
            && everyone
                .iter()
                .all(|row| row.list_id.as_deref() == Some(list))
    });
    let address = target.from_addr.as_deref().map(str::to_lowercase);
    let same_address = address.is_some()
        && everyone
            .iter()
            .all(|row| row.from_addr.as_deref().map(str::to_lowercase) == address);
    let domain = address
        .as_deref()
        .and_then(|a| a.rsplit_once('@'))
        .map(|(_, d)| registrable(d))
        .filter(|d| !shared_domain(d));
    let same_domain = domain.is_some()
        && everyone.iter().all(|row| {
            row.from_addr
                .as_deref()
                .and_then(|a| a.rsplit_once('@'))
                .map(|(_, d)| registrable(&d.to_lowercase()))
                == domain
        });

    if let Some(list) = list {
        rules.push(SmartRule {
            field: SmartField::ListId,
            op: SmartOp::Is,
            value: list.to_string(),
        });
        name = list.to_string();
    } else if same_address {
        let address = address.clone().unwrap_or_default();
        rules.push(SmartRule {
            field: SmartField::From,
            op: SmartOp::Is,
            value: address.clone(),
        });
        name = display_name(target.from_name.as_deref(), Some(&address));
    } else if same_domain {
        let domain = domain.clone().unwrap_or_default();
        rules.push(SmartRule {
            field: SmartField::From,
            op: SmartOp::EndsWith,
            value: domain.clone(),
        });
        name = domain;
    } else {
        name = target
            .subject
            .clone()
            .unwrap_or_else(|| "Similar mail".into());
    }

    // The subject words every one of them has, longest first, as far as
    // three. Enough to tell this series from the sender's other mail.
    if list.is_none() {
        let mut shared: Option<HashSet<String>> = None;
        for row in &everyone {
            let these = words(&normalise_subject(
                row.subject.as_deref().unwrap_or_default(),
            ));
            shared = Some(match shared {
                None => these,
                Some(so_far) => so_far.intersection(&these).cloned().collect(),
            });
        }
        let mut shared: Vec<String> = shared.unwrap_or_default().into_iter().collect();
        shared.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
        for word in shared
            .into_iter()
            .filter(|w| w.chars().count() >= 4 && !STOP_WORDS.contains(&w.as_str()))
            .take(3)
        {
            rules.push(SmartRule {
                field: SmartField::Subject,
                op: SmartOp::Contains,
                value: word,
            });
        }
    }

    SmartSuggestion {
        name,
        query: SmartQuery {
            match_all: true,
            rules,
        },
    }
}

/// Words too common in subjects to tell one series from another, in the two
/// languages the classifier reads.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "you", "your", "with", "from", "this", "that", "have", "are", "was",
    "about", "more", "our", "has", "is", "to", "of", "in", "on", "at", "an", "a", "it", "be", "or",
    "by", "as", "we", "my", "me", "re", "fw", "fwd", "der", "die", "das", "und", "ihre", "ihr",
    "ein", "eine", "einen", "mit", "für", "fuer", "über", "ueber", "sich", "sind", "wird", "nicht",
    "oder", "zum", "zur", "vom", "von", "im", "am", "auf", "den", "dem", "des", "ist", "es", "wir",
    "sie", "aw", "wg",
];

fn display_name(name: Option<&str>, address: Option<&str>) -> String {
    name.filter(|n| !n.trim().is_empty())
        .or(address)
        .unwrap_or("(unknown)")
        .to_string()
}

impl Session {
    /// Reads the headers the store came to keep, back out of mail synced
    /// before it kept them. Returns how many messages are still waiting —
    /// zero once the account is done.
    ///
    /// Its own connection, like a sync, so the window's store stays free
    /// while it reads.
    pub fn backfill_headers(&self, email: &str, batch: usize) -> Result<usize> {
        let (store, blobs) = self.open_store()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;
        let missing = store.messages_missing_headers(account.id, batch)?;
        let updates: Vec<_> = missing
            .iter()
            .map(|(id, path)| {
                let headers = path
                    .as_deref()
                    .and_then(|path| blobs.get(path).ok())
                    .and_then(|raw| core_proto::parse::backfill_headers(&raw));
                (*id, headers)
            })
            .collect();
        store.set_backfilled_headers(&updates)?;
        if missing.len() < batch {
            return Ok(0);
        }
        // More may be waiting; say roughly how many without counting them all
        // on every call.
        Ok(store.messages_missing_headers(account.id, batch)?.len())
    }

    /// Unsubscribes from each sender named by `keys`, the way each allows.
    ///
    /// One-click and mail unsubscribes happen here. A sender that offers only
    /// a web page comes back as `open` with its address, for the window to
    /// open in the browser — the person finishes those, since this cannot see
    /// what the page asks for.
    pub async fn unsubscribe(
        &self,
        email: &str,
        keys: &[String],
    ) -> Result<Vec<UnsubscribeResult>> {
        let senders = {
            let (store, blobs) = self.open_store()?;
            let account = store
                .account_by_email(email)?
                .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;
            let core = Core::new(store, blobs);
            let all = core.unsubscribe_senders(account.id)?;
            let wanted: HashSet<&String> = keys.iter().collect();
            (
                account.id,
                all.into_iter()
                    .filter(|sender| wanted.contains(&sender.key))
                    .collect::<Vec<_>>(),
            )
        };
        let (account_id, senders) = senders;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            // A one-click endpoint answers the POST itself. A redirect is it
            // sending the person somewhere, and following that with the POST
            // body is not what they agreed to.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("kuverta/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| RpcError::Network(err.to_string()))?;

        let mut results = Vec::new();
        for sender in senders {
            tracing::info!(sender = %sender.key, method = sender.method.name(), "unsubscribing");
            let outcome = match &sender.method {
                UnsubscribeMethod::OneClick { url } => one_click(&http, url).await,
                UnsubscribeMethod::Mailto {
                    address,
                    subject,
                    body,
                } => {
                    let draft = DraftInput {
                        to: vec![address.clone()],
                        subject: subject.clone(),
                        body: body.clone(),
                        ..Default::default()
                    };
                    // Filed in Sent like anything else this account sends: a
                    // mail went out in the person's name, and they should be
                    // able to see exactly what it said.
                    self.send(email, &draft, true)
                        .await
                        .map(|_| format!("mailed {address}"))
                        .map_err(|err| err.to_string())
                }
                UnsubscribeMethod::Browser { .. } => Err(String::new()),
            };

            let (state, detail, url) = match (&sender.method, outcome) {
                (UnsubscribeMethod::Browser { url }, _) => ("open", None, Some(url.clone())),
                (_, Ok(detail)) => ("done", Some(detail), None),
                (_, Err(err)) => {
                    tracing::warn!(sender = %sender.key, %err, "unsubscribing failed");
                    ("failed", Some(err), None)
                }
            };

            if state != "open" {
                let (store, _) = self.open_store()?;
                store.record_unsubscription(
                    account_id,
                    &Unsubscription {
                        sender: sender.key.clone(),
                        method: sender.method.name().into(),
                        target: sender.method.target(),
                        state: state.into(),
                        detail: detail.clone(),
                        created_at: crate::now_utc(),
                    },
                )?;
            }
            results.push(UnsubscribeResult {
                key: sender.key,
                name: sender.name,
                state: state.into(),
                detail,
                url,
            });
        }
        Ok(results)
    }
}

/// RFC 8058 §3.2: a POST whose body is exactly `List-Unsubscribe=One-Click`.
async fn one_click(http: &reqwest::Client, url: &str) -> std::result::Result<String, String> {
    let response = http
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|err| format!("could not reach the sender: {err}"))?;
    let status = response.status();
    if status.is_success() || status.is_redirection() {
        Ok(format!("the sender answered {}", status.as_u16()))
    } else {
        Err(format!("the sender answered {}", status.as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_click_needs_both_the_header_and_https() {
        let header = "<mailto:leave@list.example>, <https://list.example/u?x=1>";
        assert_eq!(
            unsubscribe_method(header, Some("List-Unsubscribe=One-Click")),
            Some(UnsubscribeMethod::OneClick {
                url: "https://list.example/u?x=1".into()
            })
        );
        // Without the POST header, the mail is the automatic choice — when
        // its address is the sender's own.
        assert!(matches!(
            unsubscribe_method_from(header, None, Some(&["news@list.example"])),
            Some(UnsubscribeMethod::Mailto { .. })
        ));
        // Plain HTTP is never posted to, only opened.
        assert_eq!(
            unsubscribe_method(
                "<http://list.example/u>",
                Some("List-Unsubscribe=One-Click")
            ),
            Some(UnsubscribeMethod::Browser {
                url: "http://list.example/u".into()
            })
        );
        assert_eq!(unsubscribe_method("<ftp://x>", None), None);
    }

    #[test]
    fn a_mailto_carries_its_subject_and_body() {
        assert_eq!(
            unsubscribe_method_from(
                "<mailto:leave%40list.example?subject=Stop%20please&body=bye>",
                None,
                Some(&["weekly.list.example"]),
            ),
            Some(UnsubscribeMethod::Mailto {
                address: "leave@list.example".into(),
                subject: "Stop please".into(),
                body: "bye".into(),
            })
        );
    }

    #[test]
    fn a_mail_unsubscribe_goes_only_to_the_sender_itself() {
        // A forged newsletter naming someone else's address is not mailed.
        let forged = "<mailto:boss@company.example?subject=hello&body=anything>";
        assert_eq!(
            unsubscribe_method_from(forged, None, Some(&["news@shop.example"])),
            None
        );
        // Nor is any address when there is no sender to check it against.
        assert_eq!(
            unsubscribe_method("<mailto:leave@shop.example>", None),
            None
        );
        // A list's own domain, or the From address's, will do.
        assert!(unsubscribe_method_from(
            "<mailto:leave@lists.shop.example>",
            None,
            Some(&["news@shop.example"])
        )
        .is_some());
    }

    #[test]
    fn nothing_is_posted_to_this_computer_or_its_network() {
        let local = [
            "https://localhost/u",
            "https://127.0.0.1/u",
            "https://192.168.1.1/admin",
            "https://[::1]/u",
            "https://router.local/u",
            "https://intranet/u",
            "https://user@10.0.0.1:8443/u",
        ];
        for url in local {
            assert_eq!(
                unsubscribe_method(&format!("<{url}>"), Some("List-Unsubscribe=One-Click")),
                None,
                "{url}"
            );
        }
        assert!(public_host("https://list.example.com/u?x=1"));
    }

    fn row(id: i64, from: &str, subject: &str, snippet: &str, list: Option<&str>) -> SimilarityRow {
        SimilarityRow {
            id,
            from_name: None,
            from_addr: Some(from.into()),
            subject: Some(subject.into()),
            snippet: Some(snippet.into()),
            list_id: list.map(Into::into),
            date_utc: None,
            category: None,
            in_reply_to: None,
        }
    }

    #[test]
    fn a_series_from_one_sender_is_alike_and_a_different_mail_is_not() {
        let a = row(
            1,
            "shop@shop.example",
            "Your order #1234 has shipped",
            "",
            None,
        );
        let b = row(
            2,
            "shop@shop.example",
            "Your order #5678 has shipped",
            "",
            None,
        );
        let c = row(3, "shop@shop.example", "Password reset", "", None);
        assert!(Shape::of(&a).likeness(&Shape::of(&b)).is_some());
        assert!(Shape::of(&a).likeness(&Shape::of(&c)).is_none());
    }

    #[test]
    fn the_same_text_from_rotating_senders_is_a_spam_run() {
        let text = "Congratulations you have been selected to receive an exclusive reward claim it now before it expires today";
        let a = row(1, "promo1@spam-a.example", "You won a reward", text, None);
        let b = row(
            2,
            "promo2@spam-b.example",
            "You won the reward!",
            text,
            None,
        );
        assert_eq!(
            Shape::of(&a).likeness(&Shape::of(&b)).as_deref(),
            Some("nearly the same text")
        );
    }

    #[test]
    fn the_suggested_mailbox_names_the_sender_and_the_shared_words() {
        let a = row(
            1,
            "shop@shop.example",
            "Your order #1234 has shipped",
            "",
            None,
        );
        let b = row(
            2,
            "Shop@shop.example",
            "Re: Your order #5678 has shipped",
            "",
            None,
        );
        let suggestion = suggest(&a, &[&b]);
        assert_eq!(suggestion.query.rules[0].field, SmartField::From);
        assert_eq!(suggestion.query.rules[0].value, "shop@shop.example");
        let words: Vec<&str> = suggestion.query.rules[1..]
            .iter()
            .map(|rule| rule.value.as_str())
            .collect();
        assert_eq!(words, ["shipped", "order"]);
    }

    #[test]
    fn a_conversation_is_never_a_run() {
        // Many people write "Rückfrage zum Angebot"; none of their replies is
        // like the others in the way that matters here.
        let mut a = row(
            1,
            "anna@example.de",
            "Rückfrage zum Angebot",
            "Können wir den Termin verschieben",
            None,
        );
        let b = row(
            2,
            "jan@gmx.de",
            "Rückfrage zum Angebot",
            "Anbei die überarbeitete Fassung",
            None,
        );
        assert!(
            Shape::of(&a).likeness(&Shape::of(&b)).is_none(),
            "different people, same subject"
        );

        a.category = Some("personal".into());
        assert!(is_conversation(&a));
        let reply = row(
            3,
            "shop@shop.example",
            "Re: Your order #1 has shipped",
            "",
            None,
        );
        assert!(is_conversation(&reply));
    }

    #[test]
    fn a_shared_provider_is_no_likeness() {
        let a = row(
            1,
            "someone@gmail.com",
            "Quarterly report ready now",
            "",
            None,
        );
        let b = row(
            2,
            "else@gmail.com",
            "Quarterly report ready today",
            "",
            None,
        );
        assert!(Shape::of(&a).likeness(&Shape::of(&b)).is_none());
        assert!(suggest(&a, &[&b])
            .query
            .rules
            .iter()
            .all(|rule| rule.value != "gmail.com"));
    }

    #[test]
    fn a_domain_under_a_short_second_level_keeps_three_labels() {
        assert_eq!(registrable("mail.news.example.co.uk"), "example.co.uk");
        assert_eq!(registrable("mail.example.de"), "example.de");
    }
}
