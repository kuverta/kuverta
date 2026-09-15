//! Cleaning up after a vision model that lost its place.
//!
//! A model that loses its place on a page — its context full, or a table it
//! cannot follow — writes the same lines again and again. As the text of a
//! letter that is worse than a gap: it fills the reading pane and search with
//! nothing, and it looks like the letter says it.

/// How many times a block of lines must repeat, back to back, to be a loop
/// rather than a table. A statement can have a few identical rows; a model that
/// has lost its place writes the same two lines a hundred times.
const LOOP_REPEATS: usize = 8;

/// The longest block of lines looked for.
const LONGEST_BLOCK: usize = 8;

/// What stands where a loop was cut.
pub const LOOP_MARK: &str =
    "[the model repeated itself here; the rest of the page may not have been read]";

/// Cuts every place the same lines repeat over and over, keeping them twice and
/// marking the cut, and says whether anything was cut.
pub fn cut_repetition(text: &str) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let same = |a: &[&str], b: &[&str]| a.iter().zip(b).all(|(x, y)| x.trim() == y.trim());

    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    let mut cut = false;
    let mut i = 0;
    'lines: while i < lines.len() {
        for k in 1..=LONGEST_BLOCK {
            if i + k * LOOP_REPEATS > lines.len() {
                break;
            }
            let block = &lines[i..i + k];
            if block.iter().all(|line| line.trim().is_empty()) {
                continue;
            }
            let mut repeats = 1;
            while i + (repeats + 1) * k <= lines.len()
                && same(block, &lines[i + repeats * k..i + (repeats + 1) * k])
            {
                repeats += 1;
            }
            if repeats >= LOOP_REPEATS {
                kept.extend_from_slice(&lines[i..i + 2 * k]);
                kept.push(LOOP_MARK);
                cut = true;
                i += repeats * k;
                continue 'lines;
            }
        }
        kept.push(lines[i]);
        i += 1;
    }
    (kept.join("\n"), cut)
}
