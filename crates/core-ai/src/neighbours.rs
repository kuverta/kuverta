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

        if total <= 0.0 {
            return Some((scored[0].1, 0.0));
        }
        Some((winner, weight / total))
    }
}
