//! Words the letters already filed suggest for the folders they went in.
//!
//! A folder's words are typed by hand, once, before any letter has arrived —
//! and they are guesses. `Taxes` was given `Finanzamt` and `Steuerbescheid`
//! and not `Kassenzeichen`, because nobody thinks of `Kassenzeichen` sitting
//! at a keyboard. It is on every letter the tax office sends.
//!
//! The letters filed already know. Each one is a page of text with a folder
//! written on it by somebody holding the paper, which is as good a label as
//! this house will ever produce. A word that stands on several letters in one
//! folder and on none in any other is a word that would have filed them.
//!
//! **This proposes and never writes.** Nothing here touches Paperless, and a
//! word only becomes a folder's word when somebody says so. The evidence
//! travels with the proposal — how many letters here, how many elsewhere — so
//! the answer to "why is it suggesting that" is on the screen rather than in
//! this file.
//!
//! **What it will not say** matters more than what it will. A word is only
//! proposed when it stands on [`AT_LEAST`] letters in its folder and on none
//! at all anywhere else. Both halves are load-bearing: the first is what
//! keeps OCR wreckage out, because misread characters do not repeat, and the
//! second is what keeps `Sehr geehrte` and the household's own street out
//! without a list of German filler words that would have to be maintained and
//! would still be wrong. The cost is that a folder nobody has filed anything
//! into learns nothing, which is correct — there is nothing to learn from.
//!
//! **People are not learned.** A letter is for somebody because their name
//! stands on it, and no other word means "this is for Nicolas". Asked for a
//! person's words, the first run of this against a real archive proposed
//! `Hambura` — which is not a word, and would have addressed post to whoever
//! lives in Hamburg.
//!
//! And **nothing at all is proposed until letters have been filed in
//! [`ENOUGH_FOLDERS`] different folders**, because until then the second half
//! has nothing to work with. On a shelf where only `Taxes` has letters,
//! `Kassenzeichen` and `Sehr geehrter Herr` are the same word: both stand on
//! every letter there is and on no letter anywhere else. The first tests
//! written here duly proposed filing tax letters on `freundlichen` and on the
//! household's own street. That is not a threshold to tune — it is the point
//! at which this can tell one thing from another at all, and before it the
//! honest answer is that there is nothing to say yet.
//!
//! `Hambura` is worth keeping in mind for the other reason too: it stood on
//! four letters, so counting letters did not catch it, and it reads like a
//! word, so looking at its letters did not either. What catches it is that
//! `Hamburg` is on more letters than it is and differs from it by one
//! character — a misreading is a rare neighbour of a common word. See
//! [`misread`].

use crate::ShelfFolder;
use serde::{Deserialize, Serialize};

/// A letter that has already been filed.
pub struct Filed<'a> {
    /// The folder it went in, as the folder is named.
    pub folder: &'a str,
    /// What is written on it: Paperless's OCR text.
    pub text: &'a str,
}

/// A word worth adding to a folder, with what says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    /// As it stands on the letters — German capitalises its nouns, and the
    /// commonest spelling of the word is the one shown.
    pub word: String,
    /// Letters in this folder it stands on.
    pub letters: usize,
}

/// What one folder's own letters suggest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Learned {
    pub folder: String,
    /// Best first: the word on most of the folder's letters. Never more than
    /// would still fit in the tag Paperless keeps them in.
    pub words: Vec<Word>,
    /// How many filed letters there were to learn from. Zero words out of a
    /// handful of letters is a real answer and this is how it is told from
    /// having had nothing to read.
    pub from: usize,
}

/// A word must stand on this many of a folder's letters. Two, not one: one is
/// a coincidence, and it is also what a misread word looks like.
pub const AT_LEAST: usize = 2;
/// Folders that must have letters in them before anything is proposed for
/// any of them. With one, every word on the shelf is in exactly one folder
/// and in no other, boilerplate included — see the note at the top.
pub const ENOUGH_FOLDERS: usize = 2;
/// Shorter than this is not a word that files anything. German runs long and
/// its short words are all filler.
const SHORTEST: usize = 6;
/// Longer than this is two words run together by the OCR.
const LONGEST: usize = 28;
/// The most proposed for one folder at a time. A screenful; the rest keep.
const MOST: usize = 10;
/// The most Paperless keeps of a tag's match, in characters.
pub const MATCH_LIMIT: usize = 256;

/// What the filed letters suggest for each of `folders`, in the order the
/// folders are given.
///
/// `filed` is every letter that has a folder on it. A letter filed in two
/// folders counts for both — it was put in both by somebody, and this is not
/// the place to second-guess that — but a word on it is then not "in no other
/// folder", so in practice such a letter teaches little. That is the right
/// outcome: a letter that fits two folders is not evidence about either.
pub fn learn(filed: &[Filed<'_>], folders: &[ShelfFolder]) -> Vec<Learned> {
    // Which folders each word stands in a letter of, counted once per letter.
    let mut counts: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    // And how each word is most often spelled.
    let mut spellings: std::collections::HashMap<String, std::collections::HashMap<String, usize>> =
        std::collections::HashMap::new();

    let mut letters = vec![0usize; folders.len()];
    for letter in filed {
        let Some(which) = folders
            .iter()
            .position(|folder| folder.name.eq_ignore_ascii_case(letter.folder.trim()))
        else {
            continue;
        };
        letters[which] += 1;
        let mut seen = std::collections::HashSet::new();
        for word in words_in(letter.text) {
            let key = word.to_lowercase();
            *spellings
                .entry(key.clone())
                .or_default()
                .entry(word.to_string())
                .or_default() += 1;
            if seen.insert(key.clone()) {
                counts.entry(key).or_insert_with(|| vec![0; folders.len()])[which] += 1;
            }
        }
    }

    // Words anybody has already typed, in any folder: they are doing their
    // job, or they were deliberately given to a different folder.
    let taken: std::collections::HashSet<String> = folders
        .iter()
        .flat_map(|folder| folder.words.split(','))
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();

    // Until more than one folder has letters there is nothing to contrast
    // against, and a proposal would be boilerplate as often as not.
    let telling = letters.iter().filter(|count| **count > 0).count() >= ENOUGH_FOLDERS;

    // How many letters each word stands on anywhere, which is what says
    // whether a word is a misreading of a commoner one.
    let overall: std::collections::HashMap<&str, usize> = counts
        .iter()
        .map(|(word, here)| (word.as_str(), here.iter().sum()))
        .collect();

    folders
        .iter()
        .enumerate()
        .map(|(which, folder)| {
            let mut found: Vec<Word> = counts
                .iter()
                // A person is looked for by their name. See the note at the
                // top: no other word means "this letter is for them".
                .filter(|_| telling && !folder.person)
                .filter(|(word, _)| !taken.contains(*word))
                .filter(|(_, here)| {
                    here[which] >= AT_LEAST
                        && here
                            .iter()
                            .enumerate()
                            .all(|(other, count)| other == which || *count == 0)
                })
                .filter(|(word, _)| !misread(word, &overall))
                .map(|(word, here)| Word {
                    word: spelled(&spellings, word),
                    letters: here[which],
                })
                .collect();
            // Most letters first, then the longer word — the longer of two
            // words on the same letters is the more particular one — then by
            // spelling, so the same shelf gives the same answer twice.
            found.sort_by(|a, b| {
                b.letters
                    .cmp(&a.letters)
                    .then(b.word.chars().count().cmp(&a.word.chars().count()))
                    .then(a.word.cmp(&b.word))
            });
            found.truncate(MOST);
            fit(&mut found, folder);
            Learned {
                folder: folder.name.clone(),
                words: found,
                from: letters[which],
            }
        })
        .collect()
}

/// Whether a word is a misreading of a commoner one: it differs from some
/// other word on the shelf by a single character, and that other word stands
/// on more letters than it does.
///
/// This is the one thing counting letters cannot do. `Hambura` stood on four
/// letters of a real archive — the same smudge on the same letterhead four
/// times over — and it reads like a word, so nothing about its own spelling
/// gives it away. What gives it away is `Hamburg` standing on more letters
/// than it does, one character apart. A real word that happens to be one
/// character from a commoner one is lost with it, which is a small price:
/// there is always another word on the page.
fn misread(word: &str, overall: &std::collections::HashMap<&str, usize>) -> bool {
    let mine = overall.get(word).copied().unwrap_or(0);
    overall.iter().any(|(other, count)| {
        *count > mine && other.len().abs_diff(word.len()) <= 1 && one_apart(word, other)
    })
}

/// Whether two words differ by one character: one changed, one added, or one
/// taken away. Not a general edit distance — it stops at the first difference
/// and checks the rest lines up, which is all "one apart" needs.
fn one_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b {
        return false;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if long.len() - short.len() > 1 {
        return false;
    }
    let same = if long.len() == short.len() {
        // One character changed: the rest must match.
        short.iter().zip(long).filter(|(x, y)| x != y).count() <= 1
    } else {
        // One character added: skip it once and the rest must match.
        let at = short
            .iter()
            .zip(long.iter())
            .position(|(x, y)| x != y)
            .unwrap_or(short.len());
        short[at..] == long[at + 1..]
    };
    same
}

/// Drops whatever would not fit in the tag Paperless keeps the words in. A
/// word that cannot be stored is not a suggestion, and finding that out after
/// somebody has accepted it is how the rig came to refuse three folders at
/// once.
fn fit(words: &mut Vec<Word>, folder: &ShelfFolder) {
    let mut room = MATCH_LIMIT.saturating_sub(folder.paperless_match().chars().count());
    words.retain(|word| {
        // A space before it, and quotes around it if it has a space in it.
        let costs = word.word.chars().count() + 1 + if word.word.contains(' ') { 2 } else { 0 };
        if costs > room {
            return false;
        }
        room -= costs;
        true
    });
}

/// The commonest spelling of a word, ties going to the one that sorts first
/// so that the same letters always give the same answer.
fn spelled(
    spellings: &std::collections::HashMap<String, std::collections::HashMap<String, usize>>,
    key: &str,
) -> String {
    spellings
        .get(key)
        .and_then(|seen| {
            seen.iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(spelling, _)| spelling.clone())
        })
        .unwrap_or_else(|| key.to_string())
}

/// The words on a page: runs of letters, of a length that could file
/// something, that look like a word rather than like a misreading.
fn words_in(text: &str) -> impl Iterator<Item = &str> {
    text.split(|letter: char| !letter.is_alphabetic())
        .filter(|word| {
            let length = word.chars().count();
            (SHORTEST..=LONGEST).contains(&length) && is_a_word(word)
        })
}

/// Whether a run of letters reads like a word. Cheap and deliberately loose:
/// the real filter is that a word has to stand on two letters, and OCR
/// wreckage does not repeat. This only catches the wreckage that does — the
/// same smudge in the same place on a form sent twice.
fn is_a_word(word: &str) -> bool {
    let mut vowels = 0usize;
    let mut same_in_a_row = 1usize;
    let mut worst = 1usize;
    let mut last = None;
    for letter in word.chars().flat_map(char::to_lowercase) {
        if "aeiouyäöü".contains(letter) {
            vowels += 1;
        }
        match last {
            Some(previous) if previous == letter => same_in_a_row += 1,
            _ => same_in_a_row = 1,
        }
        worst = worst.max(same_in_a_row);
        last = Some(letter);
    }
    let length = word.chars().count();
    // A word with no vowel in it was not read off a page, and no German word
    // has the same letter three times running.
    vowels > 0 && vowels * 5 <= length * 4 && worst < 3
}
