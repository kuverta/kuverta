//! Filing by resemblance to what has been filed before.
//!
//! Plan §4's alternative to prompting: embed the message, find the nearest of
//! the user's own past filings, and take their category. No prompt to tune, no
//! taxonomy to explain, and it improves every time the user files something —
//! which a prompt never does.

use std::cmp::Ordering;

use core_rules::{Category, MessageFacts};

/// Cosine similarity. Zero for vectors that cannot be compared, never NaN — a
/// NaN would sort unpredictably and quietly pick an arbitrary neighbour.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// The text a message is embedded as.
pub fn embedding_input(model: &str, facts: &MessageFacts<'_>) -> String {
    let mut parts = Vec::new();
    // nomic-embed-text is trained with task prefixes, and embeds noticeably
    // worse for classification without one.
    if model.contains("nomic") {
        parts.push("classification:".to_string());
    }
    if let Some(name) = facts.from_name {
        parts.push(name.to_string());
    }
    if let Some(addr) = facts.from_addr {
        parts.push(addr.to_string());
    }
    if let Some(list_id) = facts.list_id {
        parts.push(format!("list {list_id}"));
    }
    if let Some(subject) = facts.subject {
        parts.push(subject.to_string());
    }
    if let Some(snippet) = facts.snippet {
        parts.push(snippet.chars().take(600).collect());
    }
    parts.join("\n")
}

pub struct Neighbours {
    examples: Vec<(Vec<f32>, Category)>,
    k: usize,
}

impl Neighbours {
    pub fn new(k: usize) -> Self {
        Self {
            examples: Vec::new(),
            k: k.max(1),
        }
    }

    pub fn add(&mut self, vector: Vec<f32>, category: Category) {
        self.examples.push((vector, category));
    }

    pub fn len(&self) -> usize {
        self.examples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.examples.is_empty()
    }

    /// The category of the nearest filings, weighted by how near they are, and
    /// the share of that weight the winner holds.
    pub fn classify(&self, vector: &[f32]) -> Option<(Category, f32)> {
        self.nearest(vector)
            .map(|found| (found.category, found.share))
    }

    /// The same answer, with how close the closest filing was.
    ///
    /// The similarity is what makes a combined classifier possible: a message
    /// whose nearest filing is almost identical to it is a sender filed before,
    /// and the filings can be trusted; one whose nearest filing is merely the
    /// least unlike it is a stranger, and should go to the model. §15 found
    /// embeddings perfect on the first and poor on the second.
    pub fn nearest(&self, vector: &[f32]) -> Option<Nearest> {
        if self.examples.is_empty() {
            return None;
        }

        let mut scored: Vec<(f32, Category)> = self
            .examples
            .iter()
            .map(|(example, category)| (cosine(vector, example), *category))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));

        let mut votes: Vec<(Category, f32)> = Vec::new();
        for (similarity, category) in scored.iter().take(self.k) {
            // A neighbour pointing the opposite way is not evidence for its
            // category, so it adds nothing rather than subtracting.
            let weight = similarity.max(0.0);
            match votes.iter_mut().find(|(voted, _)| voted == category) {
                Some((_, total)) => *total += weight,
                None => votes.push((*category, weight)),
            }
        }

        let total: f32 = votes.iter().map(|(_, weight)| weight).sum();
        // Ties go to the category named first, so the same input always gets
        // the same answer.
        let (winner, weight) = votes.into_iter().max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.0.as_str().cmp(a.0.as_str()))
        })?;

        Some(if total <= 0.0 {
            Nearest {
                category: scored[0].1,
                share: 0.0,
                similarity: scored[0].0,
            }
        } else {
            Nearest {
                category: winner,
                share: weight / total,
                similarity: scored[0].0,
            }
        })
    }
}

/// What the nearest filings said, and how sure they were.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearest {
    pub category: Category,
    /// The winning category's share of the neighbours' weight: 1.0 when all of
    /// them agree.
    pub share: f32,
    /// Cosine similarity to the single closest filing.
    pub similarity: f32,
}

/// When the nearest filings are enough, and the model need not be asked.
///
/// Both conditions, not either. A close filing that its neighbours contradict
/// is a sender filed two ways; a unanimous vote among distant filings is a
/// stranger who happens to resemble one kind of mail. Neither is the cheap case.
#[derive(Debug, Clone, Copy)]
pub struct Hybrid {
    pub min_similarity: f32,
    pub min_share: f32,
}

/// The measured default: a similarity of 0.85 and a four-in-five agreement.
///
/// From `fuckmail eval`'s sweep on the generated corpus (decisions §16). On
/// senders never filed, accuracy fell below the model's from 0.75 down and
/// matched it from 0.80 up; on senders filed before, every threshold to 0.90
/// answered everything from the filings. 0.80 is exactly where the curve
/// recovers, which is the reason not to choose it — a threshold on the knee of
/// one corpus is tuned to that corpus. 0.85 keeps a margin and still answers
/// every known sender without the model.
impl Default for Hybrid {
    fn default() -> Self {
        Self {
            min_similarity: 0.85,
            min_share: 0.8,
        }
    }
}

impl Hybrid {
    pub fn accepts(&self, found: &Nearest) -> bool {
        found.similarity >= self.min_similarity && found.share >= self.min_share
    }
}
