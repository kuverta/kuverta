//! Words the letters already filed suggest for the folders they went in.

use core_paper::learn::{learn, Learned, Word, AT_LEAST, MATCH_LIMIT};
use core_paper::ShelfFolder;

fn folder(name: &str, words: &str) -> ShelfFolder {
    ShelfFolder {
        name: name.into(),
        words: words.into(),
        person: false,
    }
}

fn filed<'a>(letters: &'a [(&'a str, String)]) -> Vec<core_paper::learn::Filed<'a>> {
    letters
        .iter()
        .map(|(folder, text)| core_paper::learn::Filed { folder, text })
        .collect()
}

fn words_for<'a>(learned: &'a [Learned], folder: &str) -> Vec<&'a str> {
    learned
        .iter()
        .find(|one| one.folder == folder)
        .expect("the folder is in the answer")
        .words
        .iter()
        .map(|word| word.word.as_str())
        .collect()
}

/// A letter with a greeting, a sign-off and the household's address on it,
/// which is what every letter has, plus whatever this one is about.
fn letter(about: &str) -> String {
    format!(
        "Sehr geehrter Herr Mustermann,\n\
         Vogelhüttendeich 28, 21107 Hamburg\n\
         {about}\n\
         Mit freundlichen Grüßen, Ihre Sachbearbeiterin"
    )
}

#[test]
fn a_word_on_several_letters_in_one_folder_and_nowhere_else_is_proposed() {
    let letters = [
        (
            "Taxes",
            letter("Finanzamt Hamburg-Mitte. Kassenzeichen 4711."),
        ),
        (
            "Taxes",
            letter("Ihr Kassenzeichen lautet 4712, Finanzkasse."),
        ),
        ("House", letter("Die Nebenkostenabrechnung für das Jahr.")),
        (
            "House",
            letter("Nebenkostenabrechnung, Nachzahlung fällig."),
        ),
    ];
    let folders = [folder("Taxes", "Finanzamt"), folder("House", "Miete")];

    let learned = learn(&filed(&letters), &folders);

    assert!(
        words_for(&learned, "Taxes").contains(&"Kassenzeichen"),
        "the word on both tax letters and neither house one: {:?}",
        words_for(&learned, "Taxes")
    );
    assert!(
        words_for(&learned, "House").contains(&"Nebenkostenabrechnung"),
        "{:?}",
        words_for(&learned, "House")
    );
}

#[test]
fn what_stands_on_every_letter_stands_on_every_letter_and_is_not_proposed() {
    let letters = [
        ("Taxes", letter("Kassenzeichen 4711.")),
        ("Taxes", letter("Kassenzeichen 4712.")),
        ("House", letter("Nebenkostenabrechnung für das Jahr.")),
        ("House", letter("Nebenkostenabrechnung, Nachzahlung.")),
    ];
    let folders = [folder("Taxes", ""), folder("House", "")];

    let learned = learn(&filed(&letters), &folders);

    for name in ["Taxes", "House"] {
        let words = words_for(&learned, name);
        // The greeting, the sign-off and the street are on all four letters.
        for common in [
            "geehrter",
            "Mustermann",
            "Vogelhüttendeich",
            "Hamburg",
            "freundlichen",
            "Sachbearbeiterin",
        ] {
            assert!(
                !words.contains(&common),
                "{name} was told to file on `{common}`: {words:?}"
            );
        }
    }
}

#[test]
fn one_letter_is_a_coincidence_and_is_not_enough() {
    let folders = [folder("Taxes", ""), folder("House", "")];
    let letters = [
        ("Taxes", letter("Grundsteuermessbescheid liegt bei.")),
        ("Taxes", letter("Nichts weiter dazu.")),
        ("House", letter("Die Nebenkostenabrechnung für das Jahr.")),
        (
            "House",
            letter("Nebenkostenabrechnung, Nachzahlung fällig."),
        ),
    ];

    let learned = learn(&filed(&letters), &folders);

    assert_eq!(
        words_for(&learned, "Taxes"),
        Vec::<&str>::new(),
        "one letter should teach nothing"
    );
    assert_eq!(learned[0].from, 2, "but it did read the letters");

    // The same word on a second letter, and now it is evidence.
    let letters = [
        ("Taxes", letter("Grundsteuermessbescheid liegt bei.")),
        ("Taxes", letter("Grundsteuermessbescheid vom Vorjahr.")),
        ("House", letter("Die Nebenkostenabrechnung für das Jahr.")),
        (
            "House",
            letter("Nebenkostenabrechnung, Nachzahlung fällig."),
        ),
    ];
    let learned = learn(&filed(&letters), &folders);
    assert_eq!(
        words_for(&learned, "Taxes"),
        vec!["Grundsteuermessbescheid"]
    );
    assert_eq!(learned[0].words[0].letters, AT_LEAST);
}

#[test]
fn a_folder_nobody_has_filed_anything_into_says_so_rather_than_guessing() {
    let letters = [
        ("Taxes", letter("Kassenzeichen 4711.")),
        ("Taxes", letter("Kassenzeichen 4712.")),
        ("House", letter("Nebenkostenabrechnung liegt bei.")),
        ("House", letter("Nebenkostenabrechnung, Nachzahlung.")),
    ];
    let folders = [
        folder("Taxes", ""),
        folder("House", ""),
        folder("Car", "Kfz"),
    ];

    let learned = learn(&filed(&letters), &folders);

    let car = learned.iter().find(|one| one.folder == "Car").unwrap();
    assert_eq!(car.from, 0, "nothing has been filed in Car");
    assert!(car.words.is_empty());
}

#[test]
fn a_word_somebody_already_typed_is_not_proposed_back_to_them() {
    let letters = [
        ("Taxes", letter("Finanzamt Hamburg-Mitte, Steuerbescheid.")),
        ("Taxes", letter("Finanzamt Hamburg-Nord, Steuerbescheid.")),
    ];
    // Both words are already on the folder — and `Steuerbescheid` is on
    // another folder on purpose, which is a decision, not an oversight.
    let folders = [
        folder("Taxes", "Finanzamt"),
        folder("Rechnungen", "Steuerbescheid"),
    ];
    let letters = [
        letters[0].clone(),
        letters[1].clone(),
        ("Rechnungen", letter("Zahlungserinnerung, offener Betrag.")),
        ("Rechnungen", letter("Zahlungserinnerung, zweite Mahnung.")),
    ];

    let learned = learn(&filed(&letters), &folders);
    let words = words_for(&learned, "Taxes");
    assert!(!words.contains(&"Finanzamt"), "{words:?}");
    assert!(!words.contains(&"Steuerbescheid"), "{words:?}");
}

#[test]
fn ocr_wreckage_does_not_read_like_a_word() {
    let letters = [
        (
            "Taxes",
            letter("Kassenzeichen xqzrtkw aaaXbbb 4711 —— |||| Kassenzeichen"),
        ),
        (
            "Taxes",
            letter("Kassenzeichen xqzrtkw aaaXbbb 4712 —— |||| Kassenzeichen"),
        ),
        ("House", letter("Die Nebenkostenabrechnung liegt bei.")),
        ("House", letter("Nebenkostenabrechnung, Nachzahlung.")),
    ];
    let folders = [folder("Taxes", ""), folder("House", "")];

    let learned = learn(&filed(&letters), &folders);
    let words = words_for(&learned, "Taxes");

    // The same smudge twice is the one thing counting letters cannot catch,
    // so it is caught here: no vowels, and a letter three times over.
    assert!(!words.contains(&"xqzrtkw"), "{words:?}");
    assert!(!words.contains(&"aaaXbbb"), "{words:?}");
    assert!(words.contains(&"Kassenzeichen"), "{words:?}");
}

#[test]
fn the_word_on_most_of_the_letters_is_proposed_first() {
    let mut letters = Vec::new();
    for _ in 0..5 {
        letters.push(("Taxes", letter("Kassenzeichen und Säumniszuschlag.")));
    }
    letters.push(("Taxes", letter("Nur Säumniszuschlag hier.")));
    letters.push(("Taxes", letter("Und hier auch Säumniszuschlag.")));
    // Another folder with letters in it, so the boilerplate every letter
    // carries has somewhere else to stand — see `ENOUGH_FOLDERS`.
    letters.push(("House", letter("Die Nebenkostenabrechnung liegt bei.")));
    letters.push(("House", letter("Nebenkostenabrechnung, Nachzahlung.")));
    let folders = [folder("Taxes", ""), folder("House", "")];

    let learned = learn(&filed(&letters), &folders);
    let words = &learned[0].words;
    assert_eq!(words[0].word, "Säumniszuschlag");
    assert_eq!(words[0].letters, 7);
    assert_eq!(words[1].word, "Kassenzeichen");
    assert_eq!(words[1].letters, 5);
}

#[test]
fn nothing_is_proposed_that_paperless_would_refuse_to_keep() {
    // A folder whose words already nearly fill the tag.
    let long = std::iter::repeat_n("Zahlungsaufforderung", 11)
        .enumerate()
        .map(|(n, word)| format!("{word}{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let folders = [folder("Rechnungen", &long), folder("Taxes", "")];
    assert!(
        folders[0].paperless_match().chars().count() > MATCH_LIMIT - 60,
        "the folder should start nearly full: {}",
        folders[0].paperless_match().chars().count()
    );

    let mut letters: Vec<(&str, String)> = (0..2)
        .map(|n| {
            (
                "Rechnungen",
                letter(&format!("Vollstreckungsbescheid Mahngebühren {n}")),
            )
        })
        .collect();
    letters.extend((0..2).map(|n| ("Taxes", letter(&format!("Kassenzeichen {n}")))));

    let learned = learn(&filed(&letters), &folders);
    let added: usize = learned[0]
        .words
        .iter()
        .map(|word| word.word.chars().count() + 1)
        .sum();
    assert!(
        folders[0].paperless_match().chars().count() + added <= MATCH_LIMIT,
        "the suggestions would not fit: {added} on top of {}",
        folders[0].paperless_match().chars().count()
    );
}

#[test]
fn the_same_shelf_read_twice_gives_the_same_answer() {
    let mut letters: Vec<(&str, String)> = (0..4)
        .map(|n| ("Taxes", letter(&format!("Kassenzeichen Finanzkasse {n}"))))
        .collect();
    letters.extend((0..2).map(|n| ("House", letter(&format!("Nebenkostenabrechnung {n}")))));
    let folders = [folder("Taxes", ""), folder("House", "")];

    let once = learn(&filed(&letters), &folders);
    let twice = learn(&filed(&letters), &folders);
    assert_eq!(once, twice);
    assert_eq!(
        once[0].words,
        vec![
            Word {
                word: "Kassenzeichen".into(),
                letters: 4
            },
            Word {
                word: "Finanzkasse".into(),
                letters: 4
            },
        ]
    );
}

#[test]
fn a_letter_filed_under_a_folder_that_is_gone_is_read_past() {
    let letters = [
        ("Ferien", letter("Kassenzeichen hier.")),
        ("Taxes", letter("Kassenzeichen dort.")),
        ("Taxes", letter("Kassenzeichen auch hier.")),
        ("House", letter("Nebenkostenabrechnung liegt bei.")),
        ("House", letter("Nebenkostenabrechnung, Nachzahlung.")),
    ];
    let folders = [folder("Taxes", ""), folder("House", "")];

    // The folder `Ferien` is not on the shelf any more. Its letter is not
    // evidence for anything, and it is not evidence against `Taxes` either.
    let learned = learn(&filed(&letters), &folders);
    assert_eq!(learned.len(), 2);
    assert_eq!(learned[0].from, 2);
    assert!(words_for(&learned, "Taxes").contains(&"Kassenzeichen"));
}

#[test]
fn with_letters_in_only_one_folder_there_is_nothing_to_tell_apart_and_nothing_is_said() {
    // Every one of these words stands on every letter there is and on no
    // letter anywhere else: `Kassenzeichen`, which would file a tax letter,
    // and `freundlichen`, which would file everything. Nothing here can tell
    // them apart, so nothing here says anything.
    let letters: Vec<(&str, String)> = (0..6)
        .map(|n| ("Taxes", letter(&format!("Kassenzeichen Finanzkasse {n}"))))
        .collect();
    let folders = [folder("Taxes", ""), folder("House", "")];

    let learned = learn(&filed(&letters), &folders);

    assert_eq!(learned[0].from, 6, "the letters were read");
    assert_eq!(learned[0].words, Vec::new(), "and nothing was made of them");

    // Two letters in a second folder, and the boilerplate has somewhere else
    // to stand.
    let letters: Vec<(&str, String)> = letters
        .into_iter()
        .chain((0..2).map(|n| ("House", letter(&format!("Nebenkostenabrechnung {n}")))))
        .collect();
    let learned = learn(&filed(&letters), &folders);
    assert_eq!(
        words_for(&learned, "Taxes"),
        vec!["Kassenzeichen", "Finanzkasse"]
    );
    assert_eq!(words_for(&learned, "House"), vec!["Nebenkostenabrechnung"]);
}
