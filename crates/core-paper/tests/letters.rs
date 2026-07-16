//! Reading who a letter is from and what it is about, when Paperless has not
//! been told.
//!
//! Found on the dev stack: freshly scanned post has no correspondent and a title
//! of `Post <timestamp>`, so triage listed a dash and a number, and filing a
//! letter taught nothing. These are pure — a `Document` in, strings out.

use core_paper::Document;
use core_rules::{Category, Classifier, Learned};

fn letter(title: &str, correspondent: Option<&str>, content: Option<&str>) -> Document {
    Document {
        id: 1,
        title: title.into(),
        correspondent: correspondent.map(Into::into),
        created_utc: None,
        added_utc: None,
        tags: vec![],
        content: content.map(Into::into),
        page_count: None,
        download_path: String::new(),
    }
}

/// The OCR text Paperless produced for the two dev-stack letters, verbatim.
const STADTWERKE: &str = "Stadtwerke Musterstadt GmbH\n\nErika Mustermann, Hauptstrasse 12, 80331 Muenchen\n\nRechnung Nr. 2026-48211\n\nRechnungsdatum: 01.09.2026\n\nStromverbrauch August 2026: 312 kWh\nRechnungsbetrag: 98,40 EUR\n\nBitte ueberweisen Sie den Betrag bis zum 15.09.2026.";
const FINANZAMT: &str = "Finanzamt Muenchen\n\nErika Mustermann, Hauptstrasse 12, 80331 Muenchen\n\nBescheid fuer 2025 ueber Einkommensteuer\n\nSteuernummer 143/222/33333\nDie festgesetzte Steuer betraegt 1.204,00 EUR.\n\nGegen diesen Bescheid ist der Einspruch zulaessig.";

#[test]
fn the_scanned_dev_letters_say_who_and_what() {
    let stadtwerke = letter("Post 1789400000-1", None, Some(STADTWERKE));
    assert_eq!(stadtwerke.sender(), Some("Stadtwerke Musterstadt GmbH"));
    assert!(stadtwerke.sender_is_inferred());
    assert_eq!(stadtwerke.subject(), "Rechnung Nr. 2026-48211");

    let finanzamt = letter("Post 1789400060-2", None, Some(FINANZAMT));
    assert_eq!(finanzamt.sender(), Some("Finanzamt Muenchen"));
    // "2025 ueber" is a year, not a postcode and a place.
    assert_eq!(
        finanzamt.subject(),
        "Bescheid fuer 2025 ueber Einkommensteuer"
    );
}

#[test]
fn a_din_letter_skips_the_return_line_the_address_block_and_the_date() {
    let text = "Stadtwerke Musterstadt GmbH · Postfach 1234 · 80000 München\n\
                Erika Mustermann\n\
                Hauptstraße 12\n\
                80331 München\n\
                München, 01.09.2026\n\
                Ihre Stromrechnung für August\n\
                Sehr geehrte Frau Mustermann,\n";
    let document = letter("scan-0003", None, Some(text));
    assert_eq!(document.sender(), Some("Stadtwerke Musterstadt GmbH"));
    assert_eq!(document.subject(), "Ihre Stromrechnung für August");
}

#[test]
fn a_postal_address_is_only_claimed_where_it_cannot_be_the_recipients() {
    // The return line on the letterhead, which is what DIN 5008 puts there.
    let return_line = letter(
        "scan-0003",
        None,
        Some(
            "Stadtwerke Musterstadt GmbH · Postfach 1234 · 80000 München\n\
             Erika Mustermann\n\
             Hauptstraße 12\n\
             80331 München\n\
             München, 01.09.2026\n\
             Ihre Stromrechnung für August\n",
        ),
    );
    assert_eq!(
        return_line.postal_address().as_deref(),
        Some("Postfach 1234, 80000 München")
    );

    // Two blocks: the sender's under the letterhead, the recipient's below.
    let two_blocks = letter(
        "scan-0004",
        None,
        Some(
            "Stadtwerke Musterstadt GmbH\n\
             Musterstraße 1\n\
             80331 München\n\
             \n\
             Erika Mustermann\n\
             Hauptstraße 12\n\
             80331 München\n\
             \n\
             Ihre Stromrechnung für August\n",
        ),
    );
    assert_eq!(
        two_blocks.postal_address().as_deref(),
        Some("Musterstraße 1\n80331 München")
    );

    // The common scan: one name, then an address block that is the reader's
    // own. Saying that is the sender's would be worse than saying nothing.
    assert_eq!(
        letter("Post 1789400000-1", None, Some(STADTWERKE)).postal_address(),
        None
    );
    assert_eq!(
        letter("Post 1789400060-2", None, Some(FINANZAMT)).postal_address(),
        None
    );
    assert_eq!(letter("scan-0005", None, None).postal_address(), None);
}

#[test]
fn what_paperless_was_told_always_wins() {
    let document = letter(
        "Abschlag März",
        Some("Stadtwerke München"),
        Some(STADTWERKE),
    );
    assert_eq!(document.sender(), Some("Stadtwerke München"));
    assert!(!document.sender_is_inferred());
    assert_eq!(document.subject(), "Abschlag März");
}

#[test]
fn only_titles_that_are_a_scans_name_give_way() {
    for name in [
        "Post 1789400000-1",
        "IMG_2044",
        "scan-0003",
        "Dokument 7",
        "(no title)",
        "  ",
    ] {
        assert_eq!(
            letter(name, None, Some(STADTWERKE)).subject(),
            "Rechnung Nr. 2026-48211",
            "{name:?} is machine-made"
        );
    }
    for name in ["Rechnung 2026", "Postbank Kontoauszug", "Scanner-Garantie"] {
        assert_eq!(
            letter(name, None, Some(STADTWERKE)).subject(),
            name,
            "{name:?} was written by a person"
        );
    }
}

#[test]
fn with_no_text_there_is_no_guess() {
    let document = letter("Post 1789400000-1", None, None);
    assert_eq!(document.sender(), None);
    assert!(!document.sender_is_inferred());
    assert_eq!(document.subject(), "Post 1789400000-1");

    let noise = letter("Post 1789400000-1", None, Some("12\n--\n 3 4"));
    assert_eq!(noise.sender(), None);
    assert_eq!(noise.subject(), "Post 1789400000-1");
}

#[test]
fn filing_one_letter_files_the_next_from_the_same_letterhead() {
    // The point of reading the letterhead: the learned table is keyed on the
    // sender, and a letter with no sender could never be learned from.
    let september = letter("Post 1789400000-1", None, Some(STADTWERKE));
    let october = letter(
        "Post 1792000000-1",
        None,
        Some("Stadtwerke Musterstadt GmbH\n\nErika Mustermann, Hauptstrasse 12, 80331 Muenchen\n\nIhr neuer Abschlag ab November"),
    );

    let learned = Learned::new().with_sender(september.sender().unwrap(), Category::Notification);
    let verdict = Classifier::new(learned).classify(&october.facts());

    assert_eq!(verdict.category, Category::Notification);
    assert_eq!(verdict.reasons[0].rule, "learned.sender");
}
