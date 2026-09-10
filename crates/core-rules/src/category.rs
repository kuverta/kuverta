//! The triage taxonomy.
//!
//! Deliberately small. Every category has to earn its place by changing what
//! you would *do* with a message — a taxonomy with twenty buckets is one
//! nobody keeps tidy, and the point of this tool is to reduce decisions.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A human wrote this to you and expects an answer.
    Personal,
    /// Subscribed bulk mail: lists, digests, newsletters.
    Newsletter,
    /// Unsubscribed or promotional bulk mail.
    Marketing,
    /// Invoices, receipts, orders, bank statements — money and records.
    Transactional,
    /// Machine-generated status: CI, cron, alerts, delivery notices.
    Notification,
    /// No signal strong enough to guess. Better than a confident wrong answer.
    Unknown,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Personal,
        Category::Newsletter,
        Category::Marketing,
        Category::Transactional,
        Category::Notification,
        Category::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Newsletter => "newsletter",
            Self::Marketing => "marketing",
            Self::Transactional => "transactional",
            Self::Notification => "notification",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
