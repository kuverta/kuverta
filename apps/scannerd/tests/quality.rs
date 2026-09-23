//! The photograph check against synthetic pages: a sheet of "text" — dark bars
//! with sharp edges — on bright paper, and the ways it goes wrong.

use scannerd::quality::{assess, Problem};

const W: usize = 600;
const H: usize = 800;

/// Paper of `paper` with lines of short bars of `ink`, the way a letter's
/// words look from far enough away.
fn page(paper: u8, ink: u8) -> Vec<u8> {
    let mut luma = vec![paper; W * H];
    for y in 0..H {
        for x in 0..W {
            let in_line = (y % 30) < 10;
            let in_word = (x % 40) < 30 && (x % 12) < 6;
            if in_line && in_word {
                luma[y * W + x] = ink;
            }
        }
    }
    luma
}

/// A box blur of `radius`, horizontally and vertically.
fn blurred(luma: &[u8], radius: usize) -> Vec<u8> {
    let pass = |source: &[u8], horizontal: bool| {
        let mut out = source.to_vec();
        for y in 0..H {
            for x in 0..W {
                let (mut sum, mut n) = (0u32, 0u32);
                for d in 0..=2 * radius {
                    let (sx, sy) = if horizontal {
                        ((x + d).checked_sub(radius), Some(y))
                    } else {
                        (Some(x), (y + d).checked_sub(radius))
                    };
                    if let (Some(sx), Some(sy)) = (sx, sy) {
                        if sx < W && sy < H {
                            sum += source[sy * W + sx] as u32;
                            n += 1;
                        }
                    }
                }
                out[y * W + x] = (sum / n) as u8;
            }
        }
        out
    };
    pass(&pass(luma, true), false)
}

#[test]
fn a_well_exposed_sharp_page_is_good() {
    let quality = assess(&page(215, 30), W, H);
    assert!(quality.ok(), "{quality:?}");
    assert_eq!(quality.summary(), "good");
}

#[test]
fn clipped_paper_with_pale_text_is_washed_out() {
    // What the Pi's first pages looked like: paper at 255, text at 220.
    let quality = assess(&page(255, 220), W, H);
    assert_eq!(quality.problems, vec![Problem::WashedOut]);
}

#[test]
fn clipped_paper_with_dark_text_is_fine() {
    // White paper at full white is only a problem when it takes the text
    // with it.
    assert!(assess(&page(255, 40), W, H).ok());
}

#[test]
fn a_dark_middle_is_no_page() {
    let quality = assess(&page(60, 10), W, H);
    assert_eq!(quality.problems, vec![Problem::NoPage]);
}

#[test]
fn a_blurred_page_is_blurry() {
    let quality = assess(&blurred(&page(215, 30), 3), W, H);
    assert_eq!(quality.problems, vec![Problem::Blurry], "{quality:?}");
}

#[test]
fn a_page_with_nothing_on_it_has_no_text() {
    let quality = assess(&vec![205u8; W * H], W, H);
    assert_eq!(quality.problems, vec![Problem::NoText]);
    // With nothing to measure, sharpness is not guessed at.
    assert_eq!(quality.sharpness, None);
}

#[test]
fn a_pale_ceiling_with_a_shadow_is_not_a_page() {
    // What the camera saw pointing upwards: a surface bright enough to pass
    // for paper, with a soft dark patch that passes for ink.
    let luma: Vec<u8> = (0..W * H)
        .map(|i| {
            let (x, y) = ((i % W) as f32, (i / W) as f32);
            let d = ((x - 400.0).powi(2) + (y - 450.0).powi(2)).sqrt();
            (165.0 - 70.0 * (-d * d / 8000.0).exp()) as u8
        })
        .collect();
    let quality = assess(&luma, W, H);
    assert!(quality.ink < 110, "{quality:?}");
    assert_eq!(quality.problems, vec![Problem::NoText]);
}

#[test]
fn only_the_middle_is_judged() {
    // A dark table all round a page that fills the middle.
    let mut luma = page(215, 30);
    for y in 0..H {
        for x in 0..W {
            if !(W / 6..W * 5 / 6).contains(&x) || !(H / 6..H * 5 / 6).contains(&y) {
                luma[y * W + x] = 20;
            }
        }
    }
    assert!(assess(&luma, W, H).ok());
}

#[test]
fn a_short_letter_is_still_text() {
    // Three lines in the middle of an otherwise empty page.
    let full = page(215, 30);
    let mut luma = vec![215u8; W * H];
    for y in 390..480 {
        luma[y * W..(y + 1) * W].copy_from_slice(&full[y * W..(y + 1) * W]);
    }
    let quality = assess(&luma, W, H);
    assert!(quality.ok(), "{quality:?}");
}

#[test]
fn a_ceiling_with_a_lamp_in_it_is_not_text() {
    // Grey, with one bright bulb and its dark fitting: a few sharp edges in a
    // photograph that is otherwise empty.
    let mut luma = vec![170u8; W * H];
    // About 1% of the middle's blocks, as the real one was.
    for y in 310..330 {
        for x in 290..310 {
            luma[y * W + x] = if y < 320 { 30 } else { 255 };
        }
    }
    let quality = assess(&luma, W, H);
    assert_eq!(quality.problems, vec![Problem::NoText], "{quality:?}");
}
