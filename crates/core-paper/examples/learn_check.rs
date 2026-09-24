//! What a real archive suggests for its folders:
//! `learn_check <paperless-url> <token>`.
//!
//! Reads and prints. The words this proposes are only ever worth as much as
//! the letters behind them, so the count of letters each folder had to learn
//! from is printed beside them — a folder with three letters in it saying
//! nothing is the right answer, not a broken one.

use core_paper::{learn, Paperless};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(url), Some(token)) = (args.next(), args.next()) else {
        eprintln!("usage: learn_check <paperless-url> <token>");
        std::process::exit(2);
    };

    let client = Paperless::new(&url, &token)?;
    let folders = client.folders().await?;
    let filed = client.filed(&folders).await?;
    println!(
        "{} folders on the shelf, {} filed letters",
        folders.len(),
        filed.len()
    );
    let filed: Vec<learn::Filed<'_>> = filed
        .iter()
        .map(|(folder, text)| learn::Filed { folder, text })
        .collect();

    for one in learn::learn(&filed, &folders) {
        let words = one
            .words
            .iter()
            .map(|word| format!("{} (on {})", word.word, word.letters))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:24} from {:3} letters: {}",
            one.folder,
            one.from,
            if words.is_empty() { "—" } else { &words }
        );
    }
    Ok(())
}
