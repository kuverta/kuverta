//! Deterministic classifier.
//!
//! This is the baseline the model has to beat. It is cheap, offline, produces
//! the same answer twice, and — the part that matters most — it can say *why*,
//! which is what makes a wrong answer correctable instead of infuriating.
//!
//! Signals are scored rather than matched first-to-win. Real mail carries
//! conflicting evidence (an invoice sent through a mailing list; a personal
//! reply from a `noreply` address), and a scoring model degrades sensibly where
//! an ordered rule list would need every conflict enumerated in the right
//! order. The matched signals are kept so the verdict stays explainable.
//!
//! One thing overrides the score: what you have told it before. A sender you
//! have corrected is filed the way you corrected it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub mod category;
mod signals;

pub use category::Category;

/// The headers and shape a message presents to the classifier.
///
/// Free of any parser or store type so the rules can be tested by writing a
/// literal, and so re-classifying from a stored row and classifying during sync
/// take exactly the same path.
#[derive(Debug, Clone, Default)]
pub struct MessageFacts<'a> {
    pub from_addr: Option<&'a str>,
    pub from_name: Option<&'a str>,
    pub subject: Option<&'a str>,
    /// RFC 2919 `List-Id`, already reduced to the bracketed identifier.
    pub list_id: Option<&'a str>,
    /// RFC 2369 `List-Unsubscribe`.
    pub list_unsubscribe: Option<&'a str>,
    /// `Precedence: bulk | list | junk`.
    pub precedence: Option<&'a str>,
    /// RFC 3834 `Auto-Submitted`.
    pub auto_submitted: Option<&'a str>,
    /// Set when the message is a reply.
    pub in_reply_to: Option<&'a str>,
    pub has_attachments: bool,
    /// How many addresses were on `To` and `Cc`.
    pub recipient_count: usize,
    /// Leading plain text, used for body keywords.
    pub snippet: Option<&'a str>,
}

/// Why the classifier decided what it did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reason {
    /// Stable identifier for the rule, for grouping and debugging.
    pub rule: String,
    /// Human-readable, shown in the UI under "why".
    pub detail: String,
    pub category: Category,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub category: Category,
    /// 0.0–1.0. Derived from how far the winner led the runner-up, so a
    /// message with balanced evidence reports low confidence rather than
    /// pretending.
    pub confidence: f64,
    pub reasons: Vec<Reason>,
}

/// Categories the user has previously assigned, keyed by sender and by list.
///
/// Built from the corrections table. This is the whole learning mechanism at
/// this stage, and it is worth more than any heuristic: it is the one signal
/// that is definitely right.
#[derive(Debug, Clone, Default)]
pub struct Learned {
    by_sender: HashMap<String, Category>,
    by_list: HashMap<String, Category>,
}

impl Learned {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_sender(mut self, addr: impl Into<String>, category: Category) -> Self {
        self.by_sender.insert(normalize(&addr.into()), category);
        self
    }

    pub fn with_list(mut self, list_id: impl Into<String>, category: Category) -> Self {
        self.by_list.insert(normalize(&list_id.into()), category);
        self
    }

    pub fn insert_sender(&mut self, addr: &str, category: Category) {
        self.by_sender.insert(normalize(addr), category);
    }

    pub fn insert_list(&mut self, list_id: &str, category: Category) {
        self.by_list.insert(normalize(list_id), category);
    }

    pub fn is_empty(&self) -> bool {
        self.by_sender.is_empty() && self.by_list.is_empty()
    }

    fn lookup(&self, facts: &MessageFacts<'_>) -> Option<(Category, String, &'static str)> {
        // Sender first: it is more specific than the list it arrived through.
        if let Some(addr) = facts.from_addr {
            if let Some(category) = self.by_sender.get(&normalize(addr)) {
                return Some((
                    *category,
                    format!("you have filed mail from {addr} as {category}"),
                    "learned.sender",
                ));
            }
        }
        if let Some(list_id) = facts.list_id {
            if let Some(category) = self.by_list.get(&normalize(list_id)) {
                return Some((
                    *category,
                    format!("you have filed mail from the list {list_id} as {category}"),
                    "learned.list",
                ));
            }
        }
        None
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Which rules these are. Raised whenever a change to the signals would file
/// mail differently, so that verdicts from older rules are decided again.
///
/// 2: role addresses, senders named after their domain, and bulk-sending
/// subdomains are organisations, not people.
pub const RULES_VERSION: i64 = 2;

/// Score below which the classifier declines to guess.
///
/// A confident wrong answer costs more than an honest "unknown": the first
/// hides mail in the wrong bucket, the second leaves it where you will see it.
const MIN_SCORE: f64 = 1.0;

pub struct Classifier {
    learned: Learned,
}

impl Classifier {
    pub fn new(learned: Learned) -> Self {
        Self { learned }
    }

    pub fn without_history() -> Self {
        Self::new(Learned::default())
    }

    pub fn classify(&self, facts: &MessageFacts<'_>) -> Classification {
        // An explicit correction is not evidence to be weighed against
        // heuristics — it is the answer.
        if let Some((category, detail, rule)) = self.learned.lookup(facts) {
            return Classification {
                category,
                confidence: 1.0,
                reasons: vec![Reason {
                    rule: rule.to_string(),
                    detail,
                    category,
                    weight: f64::INFINITY,
                }],
            };
        }

        let reasons = signals::evaluate(facts);

        let mut scores: HashMap<Category, f64> = HashMap::new();
        for reason in &reasons {
            *scores.entry(reason.category).or_default() += reason.weight;
        }

        let mut ranked: Vec<(Category, f64)> = scores.into_iter().collect();
        // Sort by score, then by name so ties are stable across runs rather
        // than following HashMap iteration order.
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.as_str().cmp(b.0.as_str()))
        });

        let Some(&(winner, top)) = ranked.first() else {
            return Classification {
                category: Category::Unknown,
                confidence: 0.0,
                reasons,
            };
        };

        if top < MIN_SCORE {
            return Classification {
                category: Category::Unknown,
                confidence: 0.0,
                reasons,
            };
        }

        let runner_up = ranked.get(1).map(|&(_, score)| score).unwrap_or(0.0);
        // 1.0 when nothing competed, 0.5 on a dead tie.
        let confidence = (top / (top + runner_up)).clamp(0.0, 1.0);

        // Keep only the evidence that supported the verdict; the rest is noise
        // in an explanation.
        let reasons = reasons
            .into_iter()
            .filter(|reason| reason.category == winner)
            .collect();

        Classification {
            category: winner,
            confidence,
            reasons,
        }
    }
}
