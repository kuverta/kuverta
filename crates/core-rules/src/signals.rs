//! The individual heuristics.
//!
//! Weights are on a rough scale where 3.0 is "this header exists for exactly
//! this purpose", 1.5 is "a good hint", and 0.75 is "worth a nudge". They are
//! guesses, and they are meant to be: the point of recording every verdict
//! alongside the model's is to replace guesses with measurements.
//!
//! Keywords are German and English because that is what the mail is.

use crate::category::Category;
use crate::{MessageFacts, Reason};

/// Money and records: things you may need to find again in two years.
const TRANSACTIONAL_KEYWORDS: &[&str] = &[
    "rechnung",
    "quittung",
    "beleg",
    "zahlung",
    "lastschrift",
    "mahnung",
    "bestellung",
    "bestätigung",
    "bestaetigung",
    "auftrag",
    "vertrag",
    "kontoauszug",
    "abschlag",
    "steuer",
    "bescheid",
    "gutschrift",
    "zahlungseingang",
    "invoice",
    "receipt",
    "order confirmation",
    "payment",
    "statement",
    "billing",
    "refund",
    "your order",
    "subscription renewal",
];

/// Someone wants you to buy something.
const MARKETING_KEYWORDS: &[&str] = &[
    "rabatt",
    "angebot",
    "gutschein",
    "aktion",
    "gewinnspiel",
    "exklusiv",
    "jetzt kaufen",
    "jetzt sichern",
    "kostenlos testen",
    "black friday",
    "% off",
    "% rabatt",
    "sale",
    "discount",
    "deal",
    "save big",
    "limited time",
    "free shipping",
    "shop now",
    "last chance",
    "unbeatable",
];

/// Local parts that mean "a machine sent this and nobody reads replies".
const AUTOMATED_LOCAL_PARTS: &[&str] = &[
    "noreply",
    "no-reply",
    "no_reply",
    "donotreply",
    "do-not-reply",
    "mailer-daemon",
    "postmaster",
    "bounce",
    "bounces",
    "notification",
    "notifications",
    "alert",
    "alerts",
    "automated",
    "cron",
    "jenkins",
    "ci",
    // Software that mails on its own.
    "cpanel",
    "nextcloud",
    "wordpress",
    "jira",
    "confluence",
];

/// The same, written as one word inside a longer address:
/// `cloudplatform-noreply`, `nichtantworten.jamobil`, `keine-antwort`.
const AUTOMATED_WORDS: &[&str] = &[
    "noreply",
    "donotreply",
    "nichtantworten",
    "keineantwort",
    "noanswer",
    "mailerdaemon",
];

/// Display names that are a department, not a person.
const ROLE_WORDS: &[&str] = &[
    "team",
    "support",
    "info",
    "service",
    "kundenservice",
    "sales",
    "marketing",
    "news",
    "billing",
    "buchhaltung",
    "admin",
    "office",
    "kontakt",
    "hilfe",
    "help",
    "noreply",
    "no-reply",
    "shop",
    "store",
];

/// Local parts of an address that belong to a role or a department: nobody
/// in particular writes from `info@` or `kundenbetreuung@`.
const ROLE_LOCAL_PARTS: &[&str] = &[
    "info",
    "service",
    "services",
    "kundenservice",
    "kundenbetreuung",
    "kunden",
    "kontakt",
    "contact",
    "hello",
    "hallo",
    "team",
    "support",
    "help",
    "hilfe",
    "news",
    "newsletter",
    "mailing",
    "marketing",
    "sales",
    "vertrieb",
    "shop",
    "store",
    "order",
    "orders",
    "bestellung",
    "bestellungen",
    "rechnung",
    "rechnungen",
    "invoice",
    "invoices",
    "billing",
    "buchhaltung",
    "account",
    "accounts",
    "konto",
    "security",
    "sicherheit",
    "update",
    "updates",
    "office",
    "online",
    "webmaster",
    "admin",
    "feedback",
    "community",
    "events",
    "presse",
    "press",
    "jobs",
    "karriere",
    "careers",
    "booking",
    "bookings",
    "reservierung",
    "reservations",
    "tickets",
    "customer",
    "customerservice",
    "care",
    "mitglieder",
    "members",
    "verwaltung",
    "zentrale",
    "empfang",
    "abo",
    "aboservice",
    "leserservice",
];

/// First labels of a sending domain that exist to send bulk mail:
/// `news.traderepublic.com`, `email.feverup.com`, `send.barneysfarm.com`.
/// Not `mail.` or `nachrichten.`: platforms relay what people write through
/// those (a buyer on Kleinanzeigen, a landlord on ImmoScout).
const BULK_SUBDOMAINS: &[&str] = &[
    "news",
    "newsletter",
    "newsletters",
    "email",
    "e",
    "em",
    "send",
    "mailing",
    "mailings",
    "mailer",
    "marketing",
    "promo",
    "promotions",
    "angebote",
    "campaign",
    "campaigns",
    "mkt",
    "crm",
    "info",
    "loyalty",
    "offers",
    "deals",
    "mc",
    "sg",
];

/// Reply prefixes, including the German "AW:" and the Scandinavian "SV:".
const REPLY_PREFIXES: &[&str] = &["re:", "aw:", "antw:", "sv:", "fwd:", "wg:"];

pub(crate) fn evaluate(facts: &MessageFacts<'_>) -> Vec<Reason> {
    let mut reasons = Vec::new();

    let subject = facts.subject.map(str::to_lowercase).unwrap_or_default();
    let snippet = facts.snippet.map(str::to_lowercase).unwrap_or_default();
    let precedence_is_bulk = facts
        .precedence
        .map(|p| {
            let p = p.trim().to_ascii_lowercase();
            p == "bulk" || p == "list" || p == "junk"
        })
        .unwrap_or(false);

    // Whether this arrived as bulk mail at all. Personal heuristics are
    // suppressed when it did: newsletters are routinely sent from a real
    // person's name to one recipient, and would otherwise read as personal.
    let bulk_domain = facts.from_addr.and_then(bulk_subdomain);
    let bulk = facts.list_id.is_some()
        || precedence_is_bulk
        || facts.list_unsubscribe.is_some()
        || bulk_domain.is_some();

    // -- bulk shape ---------------------------------------------------------

    if let Some(list_id) = facts.list_id {
        reasons.push(Reason {
            rule: "header.list_id".into(),
            detail: format!("sent through the mailing list {list_id}"),
            category: Category::Newsletter,
            weight: 2.5,
        });
    }

    if precedence_is_bulk {
        reasons.push(Reason {
            rule: "header.precedence".into(),
            detail: "marked Precedence: bulk".into(),
            category: Category::Newsletter,
            weight: 1.0,
        });
    }

    if facts.list_unsubscribe.is_some() {
        // A List-Unsubscribe without a List-Id is the shape of a marketing
        // blast: bulk enough to need an opt-out, not a real list.
        let (category, weight, detail) = if facts.list_id.is_some() {
            (Category::Newsletter, 1.0, "offers a List-Unsubscribe link")
        } else {
            (
                Category::Marketing,
                2.0,
                "offers a List-Unsubscribe link but is not a real mailing list",
            )
        };
        reasons.push(Reason {
            rule: "header.list_unsubscribe".into(),
            detail: detail.into(),
            category,
            weight,
        });
    }

    if let Some(domain) = bulk_domain {
        // Weaker than a List-Unsubscribe: the name of a domain is a habit,
        // not a header that exists for the purpose.
        if facts.list_id.is_none() && facts.list_unsubscribe.is_none() {
            reasons.push(Reason {
                rule: "sender.bulk_domain".into(),
                detail: format!("sent from {domain}, a domain for bulk mail"),
                category: Category::Marketing,
                weight: 1.5,
            });
        }
    }

    // -- automation ---------------------------------------------------------

    if let Some(auto) = facts.auto_submitted {
        if !auto.trim().eq_ignore_ascii_case("no") {
            reasons.push(Reason {
                rule: "header.auto_submitted".into(),
                detail: format!("declares Auto-Submitted: {}", auto.trim()),
                category: Category::Notification,
                weight: 3.0,
            });
        }
    }

    if let Some(local) = facts.from_addr.and_then(|addr| addr.split('@').next()) {
        let local = local.to_ascii_lowercase();
        if is_automated_address(&local) {
            reasons.push(Reason {
                rule: "sender.automated".into(),
                detail: format!("sent from the unattended address {local}@…"),
                category: Category::Notification,
                weight: 2.5,
            });
        }
    }

    // -- subject and body keywords -----------------------------------------

    if let Some(hit) = first_match(&subject, TRANSACTIONAL_KEYWORDS) {
        reasons.push(Reason {
            rule: "subject.transactional".into(),
            detail: format!("subject mentions “{hit}”"),
            category: Category::Transactional,
            // Above a List-Id and its List-Unsubscribe together: a bill sent
            // through a utility's mailing system is still the bill.
            weight: 4.0,
        });

        // A document attached to something that reads like a bill is usually
        // the bill.
        if facts.has_attachments {
            reasons.push(Reason {
                rule: "attachment.with_transactional_subject".into(),
                detail: "carries an attachment".into(),
                category: Category::Transactional,
                weight: 1.0,
            });
        }
    } else if let Some(hit) = first_match(&snippet, TRANSACTIONAL_KEYWORDS) {
        reasons.push(Reason {
            rule: "body.transactional".into(),
            detail: format!("body mentions “{hit}”"),
            category: Category::Transactional,
            weight: 1.0,
        });
    }

    if let Some(hit) = first_match(&subject, MARKETING_KEYWORDS) {
        reasons.push(Reason {
            rule: "subject.marketing".into(),
            detail: format!("subject mentions “{hit}”"),
            category: Category::Marketing,
            weight: 3.5,
        });
    } else if let Some(hit) = first_match(&snippet, MARKETING_KEYWORDS) {
        reasons.push(Reason {
            rule: "body.marketing".into(),
            detail: format!("body mentions “{hit}”"),
            category: Category::Marketing,
            weight: 1.0,
        });
    }

    // -- personal shape -----------------------------------------------------

    // A reply is strong evidence even on a list: someone typed it to you.
    if facts.in_reply_to.is_some() {
        reasons.push(Reason {
            rule: "thread.reply".into(),
            detail: "is a reply to an existing thread".into(),
            category: Category::Personal,
            weight: 2.0,
        });
    }

    if !bulk {
        if REPLY_PREFIXES
            .iter()
            .any(|prefix| subject.starts_with(prefix))
        {
            reasons.push(Reason {
                rule: "subject.reply_prefix".into(),
                detail: "subject is a reply".into(),
                category: Category::Personal,
                weight: 1.5,
            });
        }

        let organisation = facts.from_addr.is_some_and(is_role_address)
            || facts.from_addr.is_some_and(is_company_address)
            || facts
                .from_name
                .is_some_and(|name| names_the_domain(name, facts.from_addr));
        if let Some(name) = facts.from_name.filter(|_| !organisation) {
            if looks_like_a_person(name, facts.from_addr) {
                reasons.push(Reason {
                    rule: "sender.person".into(),
                    detail: format!("{name} looks like a person, not a department"),
                    category: Category::Personal,
                    weight: 1.5,
                });
            }
        }

        if facts.recipient_count == 1 && !organisation {
            reasons.push(Reason {
                rule: "recipients.single".into(),
                detail: "addressed only to you".into(),
                category: Category::Personal,
                weight: 0.75,
            });
        }
    }

    reasons
}

/// The bulk-sending label a sender's domain starts with, if any: the whole
/// domain, for the explanation.
fn bulk_subdomain(addr: &str) -> Option<String> {
    let domain = addr.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').collect();
    // `news.example.com`, not `example.com`: the label must be a subdomain.
    (labels.len() >= 3 && BULK_SUBDOMAINS.contains(&labels[0])).then_some(domain)
}

/// An address that is a role, not a person: `info@`, `service-center@`,
/// `newsletter2@`.
fn is_role_address(addr: &str) -> bool {
    let local = addr
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    for segment in segments(&local) {
        if ROLE_LOCAL_PARTS.contains(&segment) {
            return true;
        }
    }
    false
}

/// uber@uber.com: an address that is the company's own name.
fn is_company_address(addr: &str) -> bool {
    let Some((local, domain)) = addr.rsplit_once('@') else {
        return false;
    };
    let labels: Vec<&str> = domain.split('.').collect();
    labels[..labels.len().saturating_sub(1)]
        .iter()
        .any(|label| label.eq_ignore_ascii_case(local))
}

/// An address kept by a machine: `noreply@`, `alerts-eu@`,
/// `calendar-notification@`, `cloudplatform-noreply@`.
fn is_automated_address(local: &str) -> bool {
    let compact: String = local
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    segments(local).any(|segment| AUTOMATED_LOCAL_PARTS.contains(&segment))
        || AUTOMATED_WORDS.iter().any(|word| compact.contains(word))
}

/// The words of an address's local part, without trailing numbers:
/// `gkv-news` → gkv, news; `newsletter2` → newsletter.
fn segments(local: &str) -> impl Iterator<Item = &str> {
    local
        .split(['.', '-', '_', '+'])
        .map(|segment| segment.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|segment| !segment.is_empty())
}

/// A display name that is the sending domain's name — "Sparkasse
/// Aurich-Norden" from sparkasse-aurich-norden.de, "Vodafone Kundenservice"
/// from vodafone.com — is the organisation speaking. Unless the address is
/// the person's own: Anna Weber at anna@weber.de names her domain and still
/// writes herself.
fn names_the_domain(name: &str, addr: Option<&str>) -> bool {
    let Some((local, domain)) = addr.and_then(|a| a.rsplit_once('@')) else {
        return false;
    };
    let lowered = name.to_lowercase();
    // "Anna über Kleinanzeigen", "Max via LinkedIn": a person, relayed.
    if [" über ", " via ", "(via "]
        .iter()
        .any(|relay| lowered.contains(relay))
    {
        return false;
    }
    let domain = domain.to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').collect();

    // Everything but the top-level domain, without its separators.
    let body: String = labels[..labels.len().saturating_sub(1)]
        .concat()
        .replace('-', "");
    let local = local.to_lowercase();
    let tokens: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 4)
        .map(str::to_string)
        .collect();
    tokens.iter().any(|t| body.contains(t.as_str()))
        && !tokens.iter().any(|t| local.contains(t.as_str()))
}

fn first_match<'k>(haystack: &str, needles: &[&'k str]) -> Option<&'k str> {
    needles.iter().copied().find(|n| haystack.contains(n))
}

/// A display name with a given and family name, that is not a department and
/// not just the address repeated back.
fn looks_like_a_person(name: &str, addr: Option<&str>) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains('@') {
        return false;
    }

    // Many senders put the address in the display name; that says nothing.
    if let Some(addr) = addr {
        if trimmed.eq_ignore_ascii_case(addr) {
            return false;
        }
    }

    let lowered = trimmed.to_ascii_lowercase();
    if ROLE_WORDS
        .iter()
        .any(|role| lowered.split_whitespace().any(|word| word == *role))
    {
        return false;
    }

    // "Anna Weber" yes; "Rust Weekly" also matches this shape, which is why
    // this only applies when nothing marked the message as bulk.
    trimmed.split_whitespace().count() >= 2
}
