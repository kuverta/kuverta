//! The 3.5″ LCD's screens, by what they paint.
//!
//! Anti-aliased edges and gradients mean no pixel is exactly the colour it was
//! given, so these ask what a patch of the screen is mostly like: dark, or
//! near enough the colour of the news.

use scannerd::display::{Action, Panes, Screen, Tone};
use scannerd::lcd::{band, render, rows, Canvas};
use scannerd::picture::Picture;

const W: u32 = 480;
const H: u32 = 320;

fn screen(headline: &str, tone: Tone) -> Screen {
    Screen {
        headline: headline.into(),
        detail: "Put the letter in this folder".into(),
        footer: "Ready for the next letter  ·  All sent".into(),
        tone,
        ..Screen::default()
    }
}

fn scanning(pages: usize) -> Screen {
    let mut buttons = Vec::new();
    if pages > 0 {
        buttons.extend([
            Action::FinishLetter(pages),
            Action::UndoPage,
            Action::CancelLetter(pages),
        ]);
    } else {
        buttons.push(Action::StopScanning);
    }
    Screen {
        buttons,
        scanning: Some(Panes {
            live: Some(Picture::scaled(&vec![128u8; 320 * 240], 320, 240, 320, 240)),
            last: None,
        }),
        last_letter: Some((Tone::Folder, "Last letter: Taxes".into())),
        ..screen("Put a page down", Tone::Ready)
    }
}

/// How near a colour is to another, 0 the same.
fn apart(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).unsigned_abs();
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// How many pixels of a patch are near `colour`.
fn near(
    canvas: &Canvas,
    colour: (u8, u8, u8),
    xs: std::ops::Range<u32>,
    ys: std::ops::Range<u32>,
) -> usize {
    ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| apart(canvas.at(x, y), colour) < 60)
        .count()
}

fn brightness(canvas: &Canvas, x: u32, y: u32) -> u32 {
    let (r, g, b) = canvas.at(x, y);
    r as u32 + g as u32 + b as u32
}

#[test]
fn the_screen_is_dark_with_a_card_on_it() {
    let canvas = render(&screen("Car", Tone::Folder), W, H);
    // The page behind everything.
    assert!(brightness(&canvas, 3, 3) < 90, "{:?}", canvas.at(3, 3));
    // The card is lighter than the page it lies on.
    assert!(brightness(&canvas, 240, 120) > brightness(&canvas, 3, 3));
}

#[test]
fn the_news_has_an_icon_in_its_own_colour() {
    for tone in [
        Tone::Ready,
        Tone::Busy,
        Tone::Done,
        Tone::Problem,
        Tone::Folder,
        Tone::Bin,
    ] {
        let canvas = render(&screen("Car", tone), W, H);
        // The badge at the card's top left, with the icon drawn in it.
        let coloured = near(&canvas, band(tone), 36..94, 36..94);
        // The icon is drawn in it: a warning is two strokes, a folder more.
        assert!(coloured > 25, "{tone:?}: {coloured}");
    }
    assert_ne!(band(Tone::Ready), band(Tone::Problem));
}

#[test]
fn the_headline_is_large_and_light() {
    let canvas = render(&screen("Taxes", Tone::Folder), W, H);
    let letters = (40..100)
        .flat_map(|y| (110..400).map(move |x| (x, y)))
        .filter(|&(x, y)| brightness(&canvas, x, y) > 600)
        .count();
    assert!(letters > 600, "{letters}");
}

#[test]
fn a_long_folder_name_is_made_to_fit() {
    let canvas = render(
        &screen("Versicherungen und Verträge der Familie", Tone::Folder),
        W,
        H,
    );
    let spill = (0..H)
        .flat_map(|y| (W - 14..W).map(move |x| (x, y)))
        .filter(|&(x, y)| brightness(&canvas, x, y) > 400)
        .count();
    assert_eq!(spill, 0, "nothing runs off the card");
}

#[test]
fn buttons_are_bands_that_do_not_overlap_and_stay_on_the_screen() {
    let home = Screen {
        buttons: vec![
            Action::StartScanning,
            Action::LearnEmpty,
            Action::UndoLetter,
        ],
        ..screen("Ready to scan", Tone::Ready)
    };
    for screen in [home, scanning(0), scanning(3)] {
        let bands = rows(&screen, H);
        assert_eq!(bands.len(), screen.buttons.len());
        for (top, bottom, _) in &bands {
            assert!(bottom - top >= 40, "big enough to hit: {bands:?}");
            assert!(*top >= 0 && *bottom <= H as i32, "{bands:?}");
        }
        for pair in bands.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{bands:?}");
        }
    }
}

#[test]
fn the_one_thing_to_do_is_the_coloured_button() {
    let screen = Screen {
        buttons: vec![
            Action::StartScanning,
            Action::LearnEmpty,
            Action::ConfirmCancel(2),
        ],
        ..screen("Ready to scan", Tone::Ready)
    };
    let canvas = render(&screen, W, H);
    for (top, bottom, action) in rows(&screen, H) {
        let (top, bottom) = (top as u32 + 6, bottom as u32 - 6);
        let ready = near(&canvas, band(Tone::Ready), 100..380, top..bottom);
        let danger = near(&canvas, band(Tone::Problem), 100..380, top..bottom);
        match action {
            Action::StartScanning => assert!(ready > 5_000, "{ready}"),
            Action::ConfirmCancel(_) => assert!(danger > 5_000, "{danger}"),
            // The quiet ones are neither.
            _ => assert!(ready + danger < 500, "{ready} {danger}"),
        }
    }
}

#[test]
fn scanning_shows_what_the_camera_sees_and_where_the_last_letter_goes() {
    let canvas = render(&scanning(1), W, H);
    let grey = near(&canvas, (128, 128, 128), 20..230, 70..180);
    assert!(grey > 3_000, "the camera's picture: {grey}");
    assert!(
        near(&canvas, (128, 128, 128), 250..460, 70..180) < 200,
        "nothing on the right yet"
    );
    // The last letter's folder, as a pill in its colour at the top right.
    assert!(near(&canvas, band(Tone::Folder), W - 130..W - 10, 16..46) > 900);
}

#[test]
fn the_framebuffer_gets_two_little_endian_bytes_a_pixel() {
    let canvas = render(&screen("Ready", Tone::Ready), W, H);
    let bytes = canvas.bytes();
    assert_eq!(bytes.len(), (W * H * 2) as usize);
    let first = u16::from_le_bytes([bytes[0], bytes[1]]);
    assert_eq!(first, canvas.pixels[0]);
}
