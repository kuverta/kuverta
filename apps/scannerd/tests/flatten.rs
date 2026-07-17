//! Flattening the light off a photographed page.
//!
//! Every page here is built rather than photographed, because what is being
//! asked is not "does this look nicer" — it is what the arithmetic does to a
//! page with a lamp on one side of it, to a page with nothing on it, and to a
//! page whose darkest thing is not ink.

use scannerd::flatten::{flatten, Tone};

/// A page `width`×`height` lit by `light(x, y)` — a paper value 0–255 — with
/// `ink(x, y)` true where the page is written on. Ink is a fifth of whatever
/// light is falling there, which is how ink behaves.
fn page(
    width: u32,
    height: u32,
    light: impl Fn(u32, u32) -> f64,
    ink: impl Fn(u32, u32) -> bool,
) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let paper = light(x, y);
            let value = if ink(x, y) { paper * 0.2 } else { paper };
            let value = value.clamp(0.0, 255.0) as u8;
            rgb.extend_from_slice(&[value, value, value]);
        }
    }
    rgb
}

/// Lines of text down a page: every fourth row, a third of the way in.
fn lines(x: u32, y: u32) -> bool {
    y % 8 < 2 && x % 5 < 3 && y > 40
}

fn luma(rgb: &[u8], width: u32, x: u32, y: u32) -> u8 {
    let at = ((y * width + x) * 3) as usize;
    ((77 * rgb[at] as u32 + 150 * rgb[at + 1] as u32 + 29 * rgb[at + 2] as u32) >> 8) as u8
}

/// The median and the half-percentile of a picture's luma: its paper and its
/// ink, the same two numbers the rig judges a photograph by.
fn levels(rgb: &[u8]) -> (u8, u8) {
    let mut counts = [0u32; 256];
    for pixel in rgb.chunks_exact(3) {
        let value = (77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8;
        counts[value as usize] += 1;
    }
    let total = (rgb.len() / 3) as u32;
    let at = |fraction: f64| {
        let wanted = (total as f64 * fraction) as u32;
        let mut seen = 0;
        for (value, &count) in counts.iter().enumerate() {
            seen += count;
            if seen > wanted {
                return value as u8;
            }
        }
        255
    };
    (at(0.005), at(0.5))
}

#[test]
fn a_lamp_on_one_side_of_the_page_stops_being_on_one_side_of_the_page() {
    let (width, height) = (400u32, 560u32);
    // Bright at the left edge, half as bright at the right: about what a desk
    // lamp does to an A4 sheet.
    let light = |x: u32, _| 210.0 - 100.0 * (x as f64 / width as f64);
    let mut rgb = page(width, height, light, lines);

    let across = |rgb: &[u8]| {
        let paper_at = |x: u32| luma(rgb, width, x, 20) as i32;
        (paper_at(10), paper_at(width - 10))
    };
    let (left, right) = across(&rgb);
    assert!(
        left - right > 80,
        "the page should start badly lit: {left} against {right}"
    );

    flatten(&mut rgb, width, height).expect("a page this size is flattened");

    let (left, right) = across(&rgb);
    assert!(
        (left - right).abs() <= 8,
        "the lamp is still on one side: {left} against {right}"
    );
    assert!(left > 240, "the paper did not come out white: {left}");
}

#[test]
fn the_ink_ends_up_further_from_the_paper_than_it_started() {
    let (width, height) = (400u32, 560u32);
    let mut rgb = page(width, height, |_, _| 170.0, lines);

    let (ink, paper) = levels(&rgb);
    let before = (paper as f64 - ink as f64) / paper as f64;

    flatten(&mut rgb, width, height).expect("flattened");

    let (ink, paper) = levels(&rgb);
    let after = (paper as f64 - ink as f64) / paper as f64;
    assert!(
        after > before + 0.1,
        "the text is no darker against the paper: {before:.2} to {after:.2}"
    );
    assert!(paper > 245, "the paper is still grey: {paper}");
}

#[test]
fn the_colour_of_the_light_comes_off_the_paper_and_the_colour_of_a_pen_stays_on() {
    let (width, height) = (400u32, 560u32);
    // Tungsten: the paper photographs warm, red over blue.
    let mut rgb = Vec::new();
    for y in 0..height {
        for x in 0..width {
            // A blue signature across the middle of the page.
            let signed = (240..260).contains(&y) && (40..360).contains(&x);
            let pixel = if signed {
                [40u8, 45, 150]
            } else if lines(x, y) {
                [38, 32, 26]
            } else {
                [196, 172, 138]
            };
            rgb.extend_from_slice(&pixel);
        }
    }

    flatten(&mut rgb, width, height).expect("flattened");

    let paper = ((10 * width + 10) * 3) as usize;
    let (r, g, b) = (rgb[paper], rgb[paper + 1], rgb[paper + 2]);
    assert!(
        r.abs_diff(b) <= 10 && r.abs_diff(g) <= 10,
        "the paper is still the colour of the lamp: {r},{g},{b}"
    );

    let pen = ((250 * width + 200) * 3) as usize;
    let (r, b) = (rgb[pen] as i32, rgb[pen + 2] as i32);
    assert!(
        b - r > 40,
        "the signature stopped being blue: {r},{},{b}",
        rgb[pen + 1]
    );
}

#[test]
fn a_blank_page_is_not_stretched_into_its_own_grain() {
    let (width, height) = (400u32, 560u32);
    // Paper and nothing else, with the grain a real sheet has.
    let mut rgb = page(
        width,
        height,
        |x, y| 186.0 + ((x * 7 + y * 13) % 11) as f64 - 5.0,
        |_, _| false,
    );

    flatten(&mut rgb, width, height).expect("flattened");

    let (ink, paper) = levels(&rgb);
    assert!(
        paper as i32 - ink as i32 <= 30,
        "the grain was stretched into text: ink {ink} against paper {paper}"
    );
}

#[test]
fn a_shadow_with_no_paper_in_it_is_not_scaled_up_into_a_grey_cloud() {
    let (width, height) = (400u32, 560u32);
    // A well-lit page with one corner in deep shadow — a curled corner, or a
    // hand still on the sheet. Nothing in that corner is paper.
    let mut rgb = page(
        width,
        height,
        |x, y| if x < 60 && y < 60 { 18.0 } else { 200.0 },
        lines,
    );

    flatten(&mut rgb, width, height).expect("flattened");

    let corner = luma(&rgb, width, 20, 20);
    assert!(corner < 90, "the shadow was lifted into the page: {corner}");
    let elsewhere = luma(&rgb, width, 300, 20);
    assert!(
        elsewhere > 240,
        "the rest of the page is not white: {elsewhere}"
    );
}

#[test]
fn what_the_page_was_before_is_carried_out_and_is_not_what_it_became() {
    let (width, height) = (400u32, 560u32);
    let mut rgb = page(width, height, |_, _| 170.0, lines);
    let (ink_before, paper_before) = levels(&rgb);

    let tone = flatten(&mut rgb, width, height).expect("flattened");

    // The exposure is steered by these, and they have to be the camera's
    // numbers rather than this module's — see `Quality::exposed_as`.
    assert_eq!(
        tone,
        Tone {
            paper: paper_before,
            ink: ink_before
        }
    );
    let (_, paper_after) = levels(&rgb);
    assert!(
        paper_after > tone.paper,
        "flattening did nothing: {paper_after} against {}",
        tone.paper
    );
}

#[test]
fn a_picture_too_small_to_tell_the_light_from_the_page_is_left_exactly_as_it_was() {
    for (width, height) in [(1u32, 1u32), (8, 8), (40, 40), (95, 60)] {
        let before = page(width, height, |_, _| 150.0, lines);
        let mut rgb = before.clone();
        assert_eq!(
            flatten(&mut rgb, width, height),
            None,
            "{width}×{height} should be left alone"
        );
        assert_eq!(rgb, before, "{width}×{height} was changed anyway");
    }
}

#[test]
fn a_size_that_does_not_match_the_pixels_changes_nothing() {
    let mut rgb = page(400, 560, |_, _| 150.0, lines);
    let before = rgb.clone();
    // Claiming more pixels than there are is the shape of a decoding bug, and
    // the answer to it is to keep one's hands off the picture.
    assert_eq!(flatten(&mut rgb, 400, 600), None);
    assert_eq!(rgb, before);
    assert_eq!(flatten(&mut rgb, 4000, 560), None);
    assert_eq!(rgb, before);
}

#[test]
fn an_odd_shaped_page_is_flattened_without_running_off_the_end_of_it() {
    // Sizes that do not divide by the tile, including one tile wide.
    for (width, height) in [(97u32, 563u32), (563, 97), (101, 101), (2, 4000)] {
        let mut rgb = page(width, height, |x, _| 120.0 + x as f64 % 60.0, lines);
        let expected = (width * height * 3) as usize;
        flatten(&mut rgb, width, height);
        assert_eq!(rgb.len(), expected, "{width}×{height} lost pixels");
    }
}
