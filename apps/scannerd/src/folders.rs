//! Which of the folders on the shelf a letter goes in, or whether it can go in
//! the bin.
//!
//! The Pi cannot read a letter — OCR on a Zero W takes minutes a page — but
//! Paperless reads every one, and it already has the machinery for deciding
//! what a document is: tags that match themselves, by words or by learning
//! from documents tagged by hand. So a folder is a Paperless tag of the same
//! name. scannerd makes sure each exists and says how it matches — by the
//! folder's words when it has some, which works from the first letter, or by
//! Paperless's own "auto" matching when it has none, which learns from the
//! documents tagged with it. Then, after each upload, it waits for Paperless to
//! finish with the letter and reads which folder tags it gave it.
//!
//! A folder marked `discard` is the bin: a letter that gets that tag is kept in
//! Paperless and needs no paper copy.

use serde::{Deserialize, Serialize};

/// A folder on the shelf, and the Paperless tag that stands for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    /// Words that put a letter in this folder, separated by commas — any one
    /// is enough. None: Paperless learns from what is tagged with it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub words: String,
    /// The bin: a letter here can be thrown away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub discard: bool,
    /// Somebody in the household rather than somewhere on the shelf: a letter
    /// that matches is *for* this person. The display says both — the folder
    /// it goes in, and who it is for.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub person: bool,
}

impl Folder {
    pub fn named(name: &str) -> Self {
        Self {
            name: name.trim().to_string(),
            words: String::new(),
            discard: false,
            person: false,
        }
    }

    /// Somebody in the household. With no words of their own, their name is
    /// what is looked for — which is what stands on the letter.
    pub fn person(name: &str) -> Self {
        Self {
            person: true,
            ..Self::named(name)
        }
    }

    /// The words to look for, which for a person falls back to their name.
    pub fn looked_for(&self) -> String {
        if self.person && self.words.trim().is_empty() {
            return self.name.clone();
        }
        self.words.clone()
    }

    /// What the tag is called in Paperless. People are marked, so that a
    /// shelf read back out of Paperless still knows who is a person and what
    /// is a folder — and so that the two do not collide when somebody files
    /// their post under their own name.
    pub fn tag_name(&self) -> String {
        if self.person {
            format!("{PERSON_TAG_PREFIX}{}", self.name)
        } else {
            self.name.clone()
        }
    }

    /// The words, one each, as Paperless's "any word" matching takes them: a
    /// space between words, and a phrase with a space in it quoted.
    pub fn paperless_match(&self) -> String {
        self.looked_for()
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(|word| {
                if word.contains(' ') {
                    format!("\"{}\"", word.replace('"', ""))
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The other way round: a tag's match as Paperless keeps it, back into the
/// comma-separated words the setup page shows. A phrase is one word in
/// quotes, so this cannot simply split on spaces — `"Grüne Karte"` is one
/// thing to look for, not two.
pub fn words_from_match(text: &str) -> String {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for letter in text.chars() {
        match letter {
            '"' => quoted = !quoted,
            letter if letter.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            letter => word.push(letter),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words.join(", ")
}

/// What a person's tag is called in Paperless: their name behind this. A tag
/// named this way is read back as a person, and one that is not, as a folder.
pub const PERSON_TAG_PREFIX: &str = "Person: ";

/// The most Paperless keeps of a tag's match: its words, a space between them.
pub const MATCH_LIMIT: usize = 256;

/// The people the env file names: `Erika Mustermann,Max Mustermann`. Words
/// are not given here — a person is looked for by their name — and can be set
/// on the setup page for someone who is written to in more than one way.
pub fn parse_env_people(text: &str) -> Vec<Folder> {
    text.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(Folder::person)
        .collect()
}

/// Folders as the env file names them: `Car,House,Work`. A name starting with
/// `-` is the bin — `-Throw away`.
pub fn parse_env(text: &str) -> Vec<Folder> {
    text.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| match name.strip_prefix('-') {
            Some(bin) => Folder {
                discard: true,
                ..Folder::named(bin)
            },
            None => Folder::named(name),
        })
        .filter(|folder| !folder.name.is_empty())
        .collect()
}

/// Filing a letter by hand, from the display, instead of taking what
/// Paperless made of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refiling {
    /// In no folder: the letter stays in Paperless, and its folder tags come
    /// off. For post that is worth keeping but belongs on no shelf.
    Nowhere,
    /// In the bin: the paper can be thrown away, and Paperless keeps the
    /// letter. The bin's tag goes on, the other folders' come off.
    Bin,
}

/// What Paperless made of a letter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not finished yet.
    Reading,
    /// The folders whose tags it got, in the order they are set up. Empty: it
    /// matched none of them.
    Folders(Vec<Folder>),
    /// Paperless had it already and refused it.
    Duplicate,
    Failed(String),
    /// Paperless did not finish in time to be worth saying.
    TimedOut,
    /// Taken back with "Undo last letter": deleted from Paperless.
    Deleted,
    /// Filed by hand in no folder: kept in Paperless, on no shelf.
    Nowhere,
}

/// A letter handed to Paperless, followed until it is filed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filing {
    pub name: String,
    pub task: String,
    pub sent_at: u64,
    pub outcome: Outcome,
    /// When the outcome became known.
    pub decided_at: Option<u64>,
    /// The document Paperless made of it, once it has.
    pub document: Option<u64>,
    /// To be deleted from Paperless as soon as it is filed: taken back while
    /// it was still being read.
    pub undo: bool,
    /// To be filed by hand as soon as Paperless has read it: the display was
    /// told where it goes before Paperless had an answer.
    pub refile: Option<Refiling>,
    checked_at: Option<u64>,
}

/// How often Paperless is asked whether a letter is done.
pub const CHECK_EVERY_SECS: u64 = 2;
/// Longer than this, and the letter is not waited for: its folder is in
/// Paperless when it gets there, but not on the display.
pub const GIVE_UP_SECS: u64 = 15 * 60;
/// Letters followed at once. More than this in flight means a queue of old
/// post being sent, whose folders nobody is standing by to hear.
const KEEP: usize = 5;

/// The letters sent recently, newest last.
#[derive(Debug, Default)]
pub struct Filings {
    filings: Vec<Filing>,
}

impl Filings {
    pub fn sent(&mut self, name: &str, task: &str, now: u64) {
        self.filings.push(Filing {
            name: name.to_string(),
            task: task.to_string(),
            sent_at: now,
            outcome: Outcome::Reading,
            decided_at: None,
            document: None,
            undo: false,
            refile: None,
            checked_at: None,
        });
        if self.filings.len() > KEEP {
            self.filings.remove(0);
        }
    }

    /// The oldest letter still being read that is due another question, and
    /// marks it asked. Letters Paperless has taken too long over are given up
    /// on here.
    pub fn next_to_check(&mut self, now: u64) -> Option<Filing> {
        for filing in &mut self.filings {
            if filing.outcome != Outcome::Reading {
                continue;
            }
            if now.saturating_sub(filing.sent_at) >= GIVE_UP_SECS {
                filing.outcome = Outcome::TimedOut;
                filing.decided_at = Some(now);
                continue;
            }
            let due = filing
                .checked_at
                .is_none_or(|at| now < at || now - at >= CHECK_EVERY_SECS);
            if due {
                filing.checked_at = Some(now);
                return Some(filing.clone());
            }
        }
        None
    }

    pub fn decide(&mut self, task: &str, outcome: Outcome, now: u64) {
        if let Some(filing) = self.filings.iter_mut().find(|f| f.task == task) {
            filing.outcome = outcome;
            filing.decided_at = Some(now);
        }
    }

    /// The letter sent last.
    pub fn latest(&self) -> Option<&Filing> {
        self.filings.last()
    }

    /// The letter sent as `name`, to change.
    /// The same, to look at rather than to change.
    pub fn named_ref(&self, name: &str) -> Option<&Filing> {
        self.filings.iter().rev().find(|filing| filing.name == name)
    }

    pub fn named(&mut self, name: &str) -> Option<&mut Filing> {
        self.filings
            .iter_mut()
            .rev()
            .find(|filing| filing.name == name)
    }
}

/// The folders among a document's tags, given each folder's tag id.
pub fn folders_among(tags: &[u64], folder_tags: &[(u64, Folder)]) -> Vec<Folder> {
    folder_tags
        .iter()
        .filter(|(id, _)| tags.contains(id))
        .map(|(_, folder)| folder.clone())
        .collect()
}
