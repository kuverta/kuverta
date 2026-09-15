//! Cutting the loops a vision model writes when it loses its place.

use core_ai::{cut_repetition, LOOP_MARK};

#[test]
fn a_model_stuck_on_a_tables_headings_is_cut_after_writing_them_twice() {
    // The shape of what came back for a tax office statement: the letterhead,
    // then two column headings a hundred and sixteen times.
    let mut text = String::from("Finanzamt Beispielstadt\nZeitraum\nFälligkeitstag\n");
    for _ in 0..116 {
        text.push_str("EUR\nCt\n");
    }

    let (cut, looped) = cut_repetition(&text);
    assert!(looped);
    assert_eq!(
        cut,
        format!("Finanzamt Beispielstadt\nZeitraum\nFälligkeitstag\nEUR\nCt\nEUR\nCt\n{LOOP_MARK}")
    );
}

#[test]
fn a_table_with_a_few_identical_rows_is_left_alone() {
    let text = "Monat | Betrag\n0,00 EUR\n0,00 EUR\n0,00 EUR\n0,00 EUR\n0,00 EUR\nSumme | 0,00 EUR";
    assert_eq!(cut_repetition(text), (text.to_string(), false));
}

#[test]
fn one_line_over_and_over_is_a_loop_too_and_what_follows_it_is_kept() {
    let mut lines = vec!["Sehr geehrte Damen und Herren,"];
    lines.extend(std::iter::repeat_n("vielen Dank", 20));
    lines.push("Mit freundlichen Grüßen");

    let (cut, looped) = cut_repetition(&lines.join("\n"));
    assert!(looped);
    assert_eq!(
        cut,
        format!("Sehr geehrte Damen und Herren,\nvielen Dank\nvielen Dank\n{LOOP_MARK}\nMit freundlichen Grüßen")
    );
}

#[test]
fn repeated_blank_lines_are_not_a_loop() {
    let text = format!("Seite 1{}Seite 2", "\n".repeat(12));
    let (cut, looped) = cut_repetition(&text);
    assert!(!looped);
    assert_eq!(cut, text);
}

#[test]
fn a_letter_without_repetition_is_unchanged() {
    let text = "Stadtwerke Beispielstadt\nIhre Jahresabrechnung\nNachzahlung 87,40 EUR";
    assert_eq!(cut_repetition(text), (text.to_string(), false));
}
