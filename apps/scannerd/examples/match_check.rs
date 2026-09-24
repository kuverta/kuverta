//! Which folders a letter's text lands in, and on which words.
//!
//! ```text
//! cargo run --example match_check -- folders.tsv text.txt
//! ```
//!
//! `folders.tsv` is a name, a tab, and the folder's words. For tuning the
//! word lists against what the rig's own OCR actually produced: a letter from
//! the Wasserschutzpolizei came out with "Strafprozessordnung" spelled three
//! different wrong ways, and the only way to know which words still land is
//! to run them against the real text.
use anyhow::{bail, Result};
use scannerd::folders::{guess, Folder};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [folders, text] = &args[..2.min(args.len())] else {
        bail!("usage: match_check <folders.tsv> <text.txt>");
    };
    let text = std::fs::read_to_string(text)?;
    let folders: Vec<Folder> = std::fs::read_to_string(folders)?
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, words)| Folder {
            name: name.trim().to_string(),
            words: words.trim().to_string(),
            discard: false,
            person: false,
        })
        .collect();

    for folder in guess(&text, &folders) {
        // Which of its words did it, by the same rule: a word of its own,
        // not one inside a longer one.
        let landed: Vec<&str> = folder
            .words
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty() && !guess(&text, &[one(word)]).is_empty())
            .collect();
        println!("{} <- {}", folder.name, landed.join(", "));
    }
    Ok(())
}

fn one(word: &str) -> Folder {
    Folder {
        name: String::new(),
        words: word.to_string(),
        discard: false,
        person: false,
    }
}
