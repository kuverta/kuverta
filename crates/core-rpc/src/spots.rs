//! Where a PDF asks to be signed.
//!
//! A contract says where to sign: a rule to write on, with the words that
//! name it beneath — „Unterschrift Darlehensnehmer (…)“ — or beside it. A
//! signature placed without reading those words lands at a measurement
//! guessed from nothing: 30 mm above the bottom edge, which on the last page
//! of a contract is under the line, under the names, in the white.
//!
//! So the page is read for them. The text is read as the text of an
//! attachment is, with pdf-extract, but with every character's place kept,
//! and the lines drawn on the page are read with it. From those come the
//! rules — a run of underscores, or a flat stroke, or a filled hairline —
//! and for each the nearest words, which are the words the person would read
//! to find their own line. Only a rule whose words say a signature belongs
//! there is a spot: a table's border is a flat stroke too, and nothing here
//! guesses.
//!
//! Everything is in the page as a reader shows it — x from the left edge, y
//! from the bottom, a rotated page turned upright — which is what
//! [`crate::pdf::stamp`] places by, and in millimetres, which is what the
//! assistant's tool speaks in.

use lopdf::Document;
use pdf_extract::{
    ColorSpace, MediaBox, OutputDev, OutputError, Path as PdfPath, PathOp, Transform,
};

use crate::pdf::{self, MM};

/// The shortest mark that reads as a line to sign on.
const LEAST_RULE: f32 = 20.0 * MM;
/// How far beneath a rule its words may stand.
const LABEL_BELOW: f32 = 30.0;
/// How far above a rule its words may stand, when none are beneath.
const LABEL_ABOVE: f32 = 18.0;
/// The band beneath a rule the place and the date would be written in. Words
/// standing there — the words that name the line — are what leaves no room.
const CAPTION_BAND: f32 = 24.0;
/// How far from a rule its own baseline is: a rule typed as underscores sits
/// a little under the line of text it is part of, and that line is beside it
/// rather than over or under it.
const OWN_BAND: f32 = 4.0;
/// Strokes flatter than this read as horizontal.
const FLAT: f32 = 2.0;

/// A place a signature belongs, in the page as a reader shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    /// 1-based.
    pub page: usize,
    /// The signature's left edge, from the page's left edge.
    pub x_mm: f32,
    /// Where its lower edge goes, from the page's lower edge.
    pub above_bottom_mm: f32,
    /// How wide the rule it sits on is.
    pub line_mm: f32,
    /// The words that name it, as the document writes them.
    pub label: String,
    /// Nothing is written beneath, so the place and the date can go there.
    pub room_beneath: bool,
}

impl Spot {
    /// How the place reads in a list the model or the person is shown.
    pub fn say(&self) -> String {
        format!("“{}” on page {}", self.label, self.page)
    }
}

/// What the words beside a rule say, for a rule to be one to sign on.
fn names_a_signature(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "unterschrift",
        "unterzeichn",
        "unterschreib",
        "signature",
        "signatory",
        "signed",
        "sign here",
    ]
    .iter()
    .any(|word| text.contains(word))
}

/// Every place the document names for a signature, first page first and
/// from the top of each page down. Empty when it names none, or when the
/// PDF cannot be read — the caller then places by measurement.
pub fn find(pdf: &[u8]) -> Vec<Spot> {
    let Ok(doc) = pdf::load(pdf) else {
        return Vec::new();
    };
    let mut reader = Reader::default();
    // pdf-extract panics on some files rather than failing, and reads page by
    // page: what it managed before it gave up is still worth looking at.
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::output_doc(&doc, &mut reader)
    }));
    if matches!(read, Err(_) | Ok(Err(_))) {
        tracing::debug!("this PDF could not be read for the lines it asks to be signed on");
    }
    reader.spots(&doc)
}

/// A character where it sits, in the page's own coordinates.
#[derive(Debug, Clone)]
struct Glyph {
    page: usize,
    x: f32,
    y: f32,
    size: f32,
    advance: f32,
    text: String,
}

/// A mark drawn on the page, from one end to the other.
#[derive(Debug, Clone, Copy)]
struct Mark {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

/// A line of text as it reads, with the characters it is made of.
#[derive(Debug, Clone)]
struct TextLine {
    y: f32,
    x0: f32,
    x1: f32,
    text: String,
    items: Vec<Glyph>,
}

/// A rule to sign on, in the page as a reader shows it.
#[derive(Debug, Clone, Copy)]
struct Rule {
    x0: f32,
    x1: f32,
    y: f32,
}

/// Reads a PDF's characters and marks with the places they sit.
#[derive(Default)]
struct Reader {
    page: usize,
    glyphs: Vec<Glyph>,
    marks: Vec<Mark>,
}

impl OutputDev for Reader {
    fn begin_page(
        &mut self,
        page_num: u32,
        _media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        self.page = page_num as usize;
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        char: &str,
    ) -> Result<(), OutputError> {
        // The size the character is drawn at, which is the font's size under
        // everything the page scales it by.
        let size = {
            let x = font_size * (trm.m11 + trm.m21);
            let y = font_size * (trm.m12 + trm.m22);
            (x * y).abs().sqrt() as f32
        };
        self.glyphs.push(Glyph {
            page: self.page,
            x: trm.m31 as f32,
            y: trm.m32 as f32,
            size,
            advance: width as f32 * size,
            text: char.to_string(),
        });
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn stroke(
        &mut self,
        ctm: &Transform,
        _colorspace: &ColorSpace,
        _color: &[f64],
        path: &PdfPath,
    ) -> Result<(), OutputError> {
        self.drawn(ctm, path);
        Ok(())
    }

    fn fill(
        &mut self,
        ctm: &Transform,
        _colorspace: &ColorSpace,
        _color: &[f64],
        path: &PdfPath,
    ) -> Result<(), OutputError> {
        self.drawn(ctm, path);
        Ok(())
    }
}

impl Reader {
    /// The marks a path draws, each from one end to the other. A rectangle —
    /// which is how a hairline is often filled — gives its top and bottom
    /// edges; a curve only moves the pen.
    fn drawn(&mut self, ctm: &Transform, path: &PdfPath) {
        let at = |x: f64, y: f64| {
            (
                (x * ctm.m11 + y * ctm.m21 + ctm.m31) as f32,
                (x * ctm.m12 + y * ctm.m22 + ctm.m32) as f32,
            )
        };
        let mut from: Option<(f32, f32)> = None;
        for op in &path.ops {
            match *op {
                PathOp::MoveTo(x, y) => from = Some(at(x, y)),
                PathOp::LineTo(x, y) => {
                    let to = at(x, y);
                    if let Some(start) = from {
                        self.mark(start, to);
                    }
                    from = Some(to);
                }
                PathOp::CurveTo(_, _, _, _, x, y) => from = Some(at(x, y)),
                PathOp::Rect(x, y, w, h) => {
                    let (x0, y0) = at(x, y);
                    let (x1, y1) = at(x + w, y + h);
                    self.mark((x0, y0), (x1, y0));
                    self.mark((x0, y1), (x1, y1));
                    from = None;
                }
                PathOp::Close => {}
            }
        }
    }

    /// Keeps a mark long enough to be a line to sign on. Whether it is flat
    /// is asked later, of the page the right way up.
    fn mark(&mut self, from: (f32, f32), to: (f32, f32)) {
        if (to.0 - from.0).hypot(to.1 - from.1) < LEAST_RULE {
            return;
        }
        self.marks.push(Mark {
            page: self.page,
            from,
            to,
        });
    }

    /// What was read, page by page, as places to sign.
    fn spots(self, doc: &Document) -> Vec<Spot> {
        let mut out = Vec::new();
        for (number, page_id) in doc.get_pages() {
            let number = number as usize;
            let (boxed, rotate) = pdf::page_geometry(doc, page_id);
            let (matrix, _) = pdf::display_to_page(boxed, rotate);
            let shown = |point: (f32, f32)| pdf::to_display(matrix, point);

            let lines = text_lines(
                self.glyphs
                    .iter()
                    .filter(|g| g.page == number)
                    .map(|g| {
                        let (x, y) = shown((g.x, g.y));
                        Glyph { x, y, ..g.clone() }
                    })
                    .collect(),
            );
            let mut rules: Vec<Rule> = self
                .marks
                .iter()
                .filter(|mark| mark.page == number)
                .filter_map(|mark| {
                    let (from, to) = (shown(mark.from), shown(mark.to));
                    ((from.1 - to.1).abs() <= FLAT && (from.0 - to.0).abs() >= LEAST_RULE).then(
                        || Rule {
                            x0: from.0.min(to.0),
                            x1: from.0.max(to.0),
                            y: (from.1 + to.1) / 2.0,
                        },
                    )
                })
                .collect();
            for line in &lines {
                rules.extend(underscore_rules(line));
            }
            for rule in dedupe(rules) {
                if let Some(spot) = spot(number, &rule, &lines) {
                    out.push(spot);
                }
            }
        }
        out
    }
}

/// The characters as lines of text: everything on one baseline, left to
/// right, with a space where the page leaves a gap.
fn text_lines(mut glyphs: Vec<Glyph>) -> Vec<TextLine> {
    glyphs.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<TextLine> = Vec::new();
    for glyph in glyphs {
        match lines.last_mut() {
            Some(line) if (line.y - glyph.y).abs() <= (glyph.size * 0.4).max(1.5) => {
                if glyph.x - line.x1 > glyph.size * 0.25 && !line.text.ends_with(' ') {
                    line.text.push(' ');
                }
                line.x1 = line.x1.max(glyph.x + glyph.advance);
                line.text.push_str(&glyph.text);
                line.items.push(glyph);
            }
            _ => lines.push(TextLine {
                y: glyph.y,
                x0: glyph.x,
                x1: glyph.x + glyph.advance,
                text: glyph.text.clone(),
                items: vec![glyph],
            }),
        }
    }
    lines
}

/// The rules a line of text draws itself: a run of underscores, which is how
/// a document written in a word processor rules a line to sign on. They are
/// drawn below the baseline, which is where the ink would go.
fn underscore_rules(line: &TextLine) -> Vec<Rule> {
    let mut out = Vec::new();
    let mut run: Vec<&Glyph> = Vec::new();
    let mut keep = |run: &mut Vec<&Glyph>| {
        if run.len() >= 4 {
            let (first, last) = (run[0], run[run.len() - 1]);
            out.push(Rule {
                x0: first.x,
                x1: last.x + last.advance,
                y: first.y - first.size * 0.12,
            });
        }
        run.clear();
    };
    for glyph in &line.items {
        if glyph.text == "_" {
            run.push(glyph);
        } else {
            keep(&mut run);
        }
    }
    keep(&mut run);
    out
}

/// One rule where several were drawn on top of each other.
fn dedupe(rules: Vec<Rule>) -> Vec<Rule> {
    let mut out: Vec<Rule> = Vec::new();
    for rule in rules {
        if out
            .iter()
            .any(|kept| (kept.y - rule.y).abs() < 4.0 && (kept.x0 - rule.x0).abs() < 10.0)
        {
            continue;
        }
        out.push(rule);
    }
    out
}

/// Whether a line of text stands over or under `rule` rather than beside it.
fn under(rule: &Rule, line: &TextLine) -> bool {
    line.x1 > rule.x0 - 10.0 && line.x0 < rule.x1 + 10.0
}

/// A rule as a place to sign, when the words around it say it is one.
fn spot(page: usize, rule: &Rule, lines: &[TextLine]) -> Option<Spot> {
    // The words on the rule's own baseline: "Unterschrift: ______" names
    // the line it precedes.
    let own: String = lines
        .iter()
        .filter(|line| (line.y - rule.y).abs() <= OWN_BAND)
        .map(|line| line.text.replace('_', " "))
        .collect::<Vec<_>>()
        .join(" ");
    let below = lines
        .iter()
        .filter(|line| under(rule, line))
        .filter(|line| rule.y - line.y > OWN_BAND && rule.y - line.y <= LABEL_BELOW)
        .max_by(|a, b| a.y.total_cmp(&b.y));
    let above = lines
        .iter()
        .filter(|line| under(rule, line))
        .filter(|line| line.y - rule.y > OWN_BAND && line.y - rule.y <= LABEL_ABOVE)
        .min_by(|a, b| a.y.total_cmp(&b.y));

    let words =
        |line: Option<&TextLine>| line.map(|l| l.text.trim().to_string()).unwrap_or_default();
    let label = [words(below), words(above), own.trim().to_string()]
        .into_iter()
        .find(|text| names_a_signature(text))?;

    let room_beneath = !lines
        .iter()
        .filter(|line| under(rule, line))
        .any(|line| rule.y - line.y > OWN_BAND && rule.y - line.y <= CAPTION_BAND);

    Some(Spot {
        page,
        // A hair in from the end of the rule, and sitting on it: a signature
        // written by hand crosses the line rather than floating over it.
        x_mm: ((rule.x0 + 4.0) / MM).max(0.0),
        above_bottom_mm: ((rule.y - 2.0) / MM).max(0.0),
        line_mm: (rule.x1 - rule.x0) / MM,
        label: label.chars().take(120).collect(),
        room_beneath,
    })
}

// -- choosing between them ----------------------------------------------------

/// Which of the places found is the one to sign on.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    /// This one.
    On(Spot),
    /// The document names none: the measurements stand.
    Nowhere,
    /// It names several and nothing says which.
    Many(Vec<Spot>),
}

/// `text` as words, lowercase, with everything that is not a letter or a
/// digit a gap — so „(Mustermann,“ and „Mustermann“ are the same word.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `label` says every word of `wanted` — „Erika Mustermann“ under a
/// line that names Erika Mustermann.
fn says_all(label: &[String], wanted: &[String]) -> bool {
    !wanted.is_empty()
        && wanted
            .iter()
            .all(|word| label.iter().any(|w| w.starts_with(word.as_str())))
}

/// Whether `label` says the weightiest word of `wanted`: a document that
/// writes a name shortened, „(E. Mustermann)“, still names the person.
fn says_the_most_of_it(label: &[String], wanted: &[String]) -> bool {
    wanted
        .iter()
        .max_by_key(|word| word.len())
        .is_some_and(|longest| {
            longest.len() >= 4 && label.iter().any(|w| w.starts_with(longest.as_str()))
        })
}

/// The lines `wanted` names: the ones that say all of it, or — where none
/// does — the ones that say the most of it.
fn named(spots: &[Spot], wanted: &str) -> Vec<Spot> {
    let wanted = words(wanted);
    let by = |fits: fn(&[String], &[String]) -> bool| -> Vec<Spot> {
        spots
            .iter()
            .filter(|spot| fits(&words(&spot.label), &wanted))
            .cloned()
            .collect()
    };
    let all = by(says_all);
    if all.is_empty() {
        by(says_the_most_of_it)
    } else {
        all
    }
}

/// The place to sign: the one `near` names, the one that names the person,
/// or the only one there is.
///
/// `near` are words from the right line, as the person or the model says
/// them; `who` is the person themselves — their name is often what tells
/// their line from the other party's.
pub fn choose(spots: &[Spot], near: Option<&str>, page: Option<usize>, who: &str) -> Choice {
    let found: Vec<Spot> = spots
        .iter()
        .filter(|spot| page.is_none_or(|page| spot.page == page))
        .cloned()
        .collect();
    let one = |mut found: Vec<Spot>| Choice::On(found.remove(0));
    match found.len() {
        0 => return Choice::Nowhere,
        1 => return one(found),
        _ => {}
    }
    if let Some(near) = near.map(str::trim).filter(|near| !near.is_empty()) {
        let asked = named(&found, near);
        return match asked.len() {
            1 => one(asked),
            // Words that fit none of them say nothing: all the lines are
            // the answer, so that the next try is asked about every one.
            0 => Choice::Many(found),
            _ => Choice::Many(asked),
        };
    }
    let theirs = named(&found, who);
    if theirs.len() == 1 {
        return one(theirs);
    }
    Choice::Many(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readable::tests_support::pdf_of;

    /// The last page of a contract both parties sign: a rule of underscores
    /// each, with the words that name it beneath, the way a word processor
    /// writes one.
    fn contract() -> Vec<u8> {
        let line = "_".repeat(45);
        pdf_of(&[
            "BT /F1 11 Tf 70 700 Td (Hamburg, den) Tj ET".to_string(),
            format!(
                "BT /F1 11 Tf 70 600 Td ({line}) Tj ET\n\
                 BT /F1 11 Tf 70 586 Td (Unterschrift Darlehensgeber \\(Max Mustermann\\)) Tj ET\n\
                 BT /F1 11 Tf 70 500 Td ({line}) Tj ET\n\
                 BT /F1 11 Tf 70 486 Td (Unterschrift Darlehensnehmer \\(Schmetti GmbH, vertreten durch Erika Mustermann\\)) Tj ET"
            ),
        ])
    }

    #[test]
    fn the_lines_a_contract_rules_are_found_with_the_words_that_name_them() {
        let found = find(&contract());
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found.iter().all(|spot| spot.page == 2), "{found:#?}");
        assert!(found[0].label.contains("Darlehensgeber"), "{found:#?}");
        assert!(found[1].label.contains("Darlehensnehmer"), "{found:#?}");
        // On the rule rather than above the bottom edge: 600 pt up an A4
        // page is 211 mm, and the signature sits a hair under that.
        assert!(
            (found[0].above_bottom_mm - 210.0).abs() < 2.0,
            "{:?}",
            found[0]
        );
        assert!((found[0].x_mm - 26.0).abs() < 2.0, "{:?}", found[0]);
        assert!(found[0].line_mm > 80.0, "{:?}", found[0]);
        // The words stand where the place and the date would be written.
        assert!(!found[0].room_beneath);
    }

    #[test]
    fn a_rule_with_nothing_to_say_it_is_for_a_signature_is_not_one() {
        let line = "_".repeat(30);
        let found = find(&pdf_of(&[format!(
            "BT /F1 11 Tf 70 700 Td (Hamburg, den {line}) Tj ET\n\
             BT /F1 11 Tf 70 600 Td (Betrag: {line}) Tj ET"
        )]));
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn a_line_drawn_rather_than_typed_is_found_too_and_has_room_beneath_it() {
        let found = find(&pdf_of(&["0.5 w 70 400 m 300 400 l S\n\
             BT /F1 11 Tf 70 410 Td (Signature of the tenant) Tj ET"
            .to_string()]));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].label, "Signature of the tenant");
        assert!((found[0].above_bottom_mm - 140.4).abs() < 1.0, "{found:#?}");
        assert!(found[0].room_beneath, "{found:#?}");
    }

    #[test]
    fn a_filled_hairline_is_a_rule_as_much_as_a_stroked_one() {
        let found = find(&pdf_of(&["70 400 230 0.6 re f\n\
             BT /F1 11 Tf 70 384 Td (Unterschrift) Tj ET"
            .to_string()]));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].label, "Unterschrift");
    }

    #[test]
    fn nothing_is_found_in_what_is_not_a_pdf() {
        assert!(find(b"not a PDF at all").is_empty());
        assert!(find(b"%PDF-1.4 and then nothing").is_empty());
    }

    #[test]
    fn the_person_s_own_line_is_the_one_their_name_stands_under() {
        let found = find(&contract());
        let chosen = choose(&found, None, None, "Erika Mustermann");
        match chosen {
            Choice::On(spot) => assert!(spot.label.contains("Darlehensnehmer"), "{spot:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn words_from_the_right_line_pick_it_out() {
        let found = find(&contract());
        match choose(&found, Some("Darlehensgeber"), None, "") {
            Choice::On(spot) => assert!(spot.label.contains("Darlehensgeber"), "{spot:?}"),
            other => panic!("{other:?}"),
        }
        // A page of its own leaves one line, whoever asks.
        assert!(matches!(choose(&found, None, Some(1), ""), Choice::Nowhere));
    }

    #[test]
    fn a_document_that_rules_a_line_for_each_party_is_asked_about_rather_than_guessed_at() {
        let found = find(&contract());
        match choose(&found, None, None, "Hans Beispiel") {
            Choice::Many(several) => assert_eq!(several.len(), 2),
            other => panic!("{other:?}"),
        }
        // Words that fit neither: the lines are still the answer, both named.
        match choose(&found, Some("Bürge"), None, "") {
            Choice::Many(several) => assert_eq!(several.len(), 2),
            other => panic!("{other:?}"),
        }
        assert!(matches!(choose(&[], None, None, ""), Choice::Nowhere));
    }
}
