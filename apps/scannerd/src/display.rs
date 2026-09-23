//! The e-paper display on the rig: what to do next, whether the last
//! photograph can be read, and which folder the letter goes in.
//!
//! The setup page answers the same questions, but it needs a phone in one hand
//! while the other holds a letter. A 2.13″ Waveshare panel on the Pi's header
//! says it where the post is: *Ready*, *Hold still*, *Scanned* — and, under
//! that, whether the photograph is good enough for the OCR or should be taken
//! again. Once a letter is sent and Paperless has read it, the display says
//! which folder on the shelf it goes in, or that it can be thrown away (see
//! [`crate::folders`]) — until the next letter is started.
//!
//! Two kinds of display, one set of words. What the screen says is
//! [`Screen`], worked out here from the loop's state. The e-paper HAT the rig
//! started with draws it into a one-bit frame here, and [`crate::epaper`] gets
//! that onto the glass (Linux only); the 3.5″ LCD it moved to draws it in
//! colour, in [`crate::lcd`]. Both are tested on a laptop by what they draw.
//!
//! E-paper keeps its picture without power and takes a moment to change it, so
//! the panel is only touched when the picture changes, and mostly with a
//! partial refresh (about a third of a second, no flashing). Every
//! [`FULL_EVERY`] partial refreshes it gets a full one, which flashes black and
//! white for two seconds and clears the faint ghosts partial refreshes leave.

use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::Result;
use embedded_graphics::pixelcolor::{BinaryColor, PixelColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use u8g2_fonts::fonts;
use u8g2_fonts::types::{FontColor, HorizontalAlignment, VerticalPosition};
use u8g2_fonts::FontRenderer;

use crate::folders::{Filing, Folder, Outcome, GIVE_UP_SECS};
use crate::picture::Picture;
use crate::run::Photographed;

/// The panel as it is looked at: landscape, the header along the top.
pub const WIDTH: u32 = 250;
pub const HEIGHT: u32 = 122;

/// The panel's own rows, which run along its short side: 122 pixels in 16
/// bytes.
pub const ROW_BYTES: usize = (HEIGHT as usize).div_ceil(8);

/// Partial refreshes between full ones.
pub const FULL_EVERY: u32 = 20;

/// How long the verdict on the last photograph stays up.
const VERDICT_SECS: u64 = 600;

/// What is on the screen, in words. Compared, so the panel is only redrawn
/// when one of them changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screen {
    pub headline: String,
    pub detail: String,
    /// Whether the last photograph can be read, and what it says about it.
    pub verdict: Option<(bool, String)>,
    pub footer: String,
    /// What kind of news the headline is, for a display with colour.
    pub tone: Tone,
    /// Buttons, for a display you can touch, top to bottom.
    pub buttons: Vec<Action>,
    /// Scanning, on a display you can touch: what the camera sees and the
    /// page it photographed last, side by side.
    pub scanning: Option<Panes>,
    /// Where the letter before this one goes, while the next is scanned.
    pub last_letter: Option<(Tone, String)>,
    /// A line for each button that is a letter waiting to be sent.
    pub queue: Vec<String>,
}

/// What the camera sees, and the last photograph with its verdict.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Panes {
    pub live: Option<Picture>,
    pub last: Option<Picture>,
}

/// Something a button on the display does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    StartScanning,
    /// With how many pages the letter has.
    FinishLetter(usize),
    StopScanning,
    LearnEmpty,
    UndoPage,
    /// Asks first: with how many pages would go.
    CancelLetter(usize),
    /// The second tap: throw them away.
    ConfirmCancel(usize),
    UndoLetter,
    /// The second tap: take the last letter back.
    ConfirmUndoLetter,
    /// File the last letter in no folder at all; Paperless keeps it.
    FileNowhere,
    /// File the last letter in the bin: the paper can go, Paperless keeps it.
    FileInBin,
    /// Open the list of what is waiting to be sent, with how many there are.
    OpenQueue(usize),
    /// One of them, by its place in the list.
    QueuedLetter(usize),
    /// The second tap: throw that one away.
    ConfirmQueued(usize),
    /// Close the list.
    Back,
}

/// A button waiting for its second tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirming {
    Cancel,
    UndoLetter,
    /// One of the letters waiting to be sent.
    Queued(usize),
}

impl Action {
    pub fn label(self) -> String {
        match self {
            Action::StartScanning => "Start scanning".into(),
            Action::FinishLetter(1) => "Finish letter (1 page)".into(),
            Action::FinishLetter(pages) => format!("Finish letter ({pages} pages)"),
            Action::StopScanning => "Stop scanning".into(),
            Action::LearnEmpty => "Learn empty table".into(),
            Action::UndoPage => "Undo last page".into(),
            Action::CancelLetter(_) => "Cancel letter".into(),
            Action::ConfirmCancel(1) => "Tap again: throw 1 page away".into(),
            Action::ConfirmCancel(pages) => format!("Tap again: throw {pages} pages away"),
            Action::UndoLetter => "Undo last letter".into(),
            Action::FileNowhere => "No folder".into(),
            Action::FileInBin => "Throw away".into(),
            Action::ConfirmUndoLetter => "Tap again: take it back".into(),
            Action::OpenQueue(1) => "1 waiting to send".into(),
            Action::OpenQueue(waiting) => format!("{waiting} waiting to send"),
            // The list gives these their own words: when, and how often
            // sending has been tried.
            Action::QueuedLetter(_) => "A letter waiting".into(),
            Action::ConfirmQueued(_) => "Tap again: throw it away".into(),
            Action::Back => "Back".into(),
        }
    }
}

/// What kind of news a screen is: on the LCD, the colour behind the headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Waiting for a page.
    #[default]
    Ready,
    /// Something is happening; wait.
    Busy,
    /// Photographed, or already filed.
    Done,
    /// Something to look at: a bad photograph, no camera, no folder.
    Problem,
    /// A folder to put the letter in.
    Folder,
    /// The bin.
    Bin,
}

/// What the screen is made from: the loop's state as the setup page gets it.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    pub now: u64,
    /// `starting`, `waiting`, `settling`, `photographed`, `no-camera`, or
    /// `paused` while scanning is stopped.
    pub state: &'a str,
    pub finishing: bool,
    pub collecting: bool,
    pub open_pages: usize,
    pub waiting: usize,
    pub last: Option<&'a Photographed>,
    /// The letter sent last, when letters are followed into folders.
    pub filing: Option<&'a Filing>,
    /// A display you can touch: scanning is started and stopped on it.
    pub touch: bool,
    /// Whether the camera is being watched.
    pub scanning: bool,
    /// What the camera sees now, turned as photographs are.
    pub live: Option<Picture>,
    /// What just came of a button that does its work at once, such as
    /// learning the table: whether it worked, and in words.
    pub notice: Option<(bool, &'a str)>,
    /// Seconds until the letter is finished for the table being clear.
    pub finishing_in: Option<u64>,
    /// Whether the last letter can still be taken back.
    pub can_undo_letter: bool,
    /// Whether the last letter can still be filed by hand — in no folder, or
    /// in the bin.
    pub can_refile: bool,
    /// What is waiting to be sent, oldest first.
    pub queue: &'a [crate::hub::Queued],
    /// Whether the display is showing that list.
    pub showing_queue: bool,
    /// A button waiting for its second tap.
    pub confirming: Option<Confirming>,
}

/// How many bands fit under the card on the panel without squeezing it down
/// to nothing. See [`crate::lcd::rows`].
const MOST_BUTTONS: usize = 5;

impl Screen {
    pub fn for_facts(facts: &Facts<'_>) -> Self {
        let mut screen = Self::words_for(facts);
        if !facts.touch {
            return screen;
        }
        if facts.showing_queue && !facts.queue.is_empty() {
            return Self::queue_screen(facts);
        }
        if facts.scanning {
            let envelope = facts.last.is_some_and(|last| last.envelope);
            // One line at the top says what to do; the panes and the
            // buttons are the rest.
            screen.headline = match (facts.state, screen.headline.as_str()) {
                (_, "Sending letter") => "Sending the letter...".into(),
                ("waiting", _) if facts.open_pages > 0 => match facts.finishing_in {
                    Some(seconds) => format!("Next page? Finishing in {seconds}s"),
                    None => "Next page, or finish".into(),
                },
                ("waiting", _) => "Put a page down".into(),
                ("settling", _) => "Hold still...".into(),
                ("photographed", _) if facts.last.is_some_and(|last| !last.kept) => {
                    "That was the table - not kept".into()
                }
                ("photographed", _) if envelope => "Envelope - now the letter in it".into(),
                ("photographed", _) => format!("{} - next page?", screen.headline),
                (_, headline) => headline.to_string(),
            };
            screen.detail.clear();
            if let Some((ok, words)) = facts.notice {
                screen.headline = words.to_string();
                if !ok {
                    screen.tone = Tone::Problem;
                }
            }
            let mut buttons = Vec::new();
            if facts.open_pages > 0 {
                buttons.push(Action::FinishLetter(facts.open_pages));
                buttons.push(Action::UndoPage);
                buttons.push(match facts.confirming {
                    Some(Confirming::Cancel) => Action::ConfirmCancel(facts.open_pages),
                    _ => Action::CancelLetter(facts.open_pages),
                });
            } else {
                if !facts.queue.is_empty() {
                    buttons.push(Action::OpenQueue(facts.queue.len()));
                }
                // Where the last letter goes is the question in front of
                // whoever is standing there; taking it back is a rarer one,
                // and waits until scanning is stopped.
                if facts.can_refile {
                    buttons.push(Action::FileNowhere);
                    buttons.push(Action::FileInBin);
                } else if facts.can_undo_letter {
                    buttons.push(undo_letter(facts));
                }
                buttons.push(Action::StopScanning);
            }
            screen.buttons = buttons;
            screen.scanning = Some(Panes {
                live: facts.live.clone(),
                last: facts.last.and_then(|last| last.thumbnail.clone()),
            });
            // Where the letter before goes stays in sight while the next is
            // scanned: the paper is still in a hand, or on the pile.
            screen.last_letter = relevant_filing(facts).map(|filing| {
                let (words, _) = where_it_goes(&filing.outcome);
                let words = match &filing.outcome {
                    Outcome::Reading => "Last letter: sorting...".to_string(),
                    _ => format!("Last letter: {words}"),
                };
                (tone_of(&filing.outcome), words)
            });
        } else {
            if screen.headline == "Ready" {
                screen.headline = "Ready to scan".into();
                screen.detail = if facts.open_pages > 0 {
                    "A letter is open: start to add pages".into()
                } else {
                    "Tap Start, then put pages down".into()
                };
            }
            // A button pressed here says what came of it, or a finger goes on
            // pressing it.
            if let Some((ok, words)) = facts.notice {
                screen.detail = words.to_string();
                if !ok {
                    screen.tone = Tone::Problem;
                }
            }
            // Between scans the last verdict belongs to a letter already
            // finished.
            screen.verdict = None;
            screen.buttons = vec![Action::StartScanning];
            if facts.can_refile {
                screen.buttons.push(Action::FileNowhere);
                screen.buttons.push(Action::FileInBin);
            }
            if facts.can_undo_letter {
                screen.buttons.push(undo_letter(facts));
            }
            if !facts.queue.is_empty() {
                screen.buttons.push(Action::OpenQueue(facts.queue.len()));
            }
            // Learning the table is a setup job and goes last, but it is not
            // given up: the moment you most want it is just after a pile has
            // gone through, which is also when the last letter can still be
            // filed by hand and the screen is at its fullest. It only stands
            // down when there is no row left that the card above can spare.
            if screen.buttons.len() < MOST_BUTTONS {
                screen.buttons.push(Action::LearnEmpty);
            }
        }
        screen
    }

    /// The letters waiting to be sent, one a row, and a way back. As many as
    /// the panel has rows for; the rest come up as those go.
    fn queue_screen(facts: &Facts<'_>) -> Self {
        const ROWS: usize = 4;
        let shown = facts.queue.len().min(ROWS);
        let mut buttons: Vec<Action> = (0..shown)
            .map(|index| match facts.confirming {
                Some(Confirming::Queued(at)) if at == index => Action::ConfirmQueued(index),
                _ => Action::QueuedLetter(index),
            })
            .collect();
        buttons.push(Action::Back);
        let more = facts.queue.len() - shown;
        Self {
            headline: match facts.queue.len() {
                1 => "1 waiting to send".to_string(),
                waiting => format!("{waiting} waiting to send"),
            },
            detail: match more {
                0 => "Tap a letter to throw it away".to_string(),
                1 => "Tap a letter to throw it away; 1 more below".to_string(),
                more => format!("Tap a letter to throw it away; {more} more below"),
            },
            tone: Tone::Busy,
            queue: facts
                .queue
                .iter()
                .take(shown)
                .map(|waiting| {
                    let tries = match waiting.attempts {
                        0 => "not tried yet".to_string(),
                        1 => "tried once".to_string(),
                        tries => format!("tried {tries}×"),
                    };
                    format!("{} · {tries}", clock(waiting.at))
                })
                .collect(),
            buttons,
            ..Self::default()
        }
    }

    fn words_for(facts: &Facts<'_>) -> Self {
        let verdict = facts
            .last
            .filter(|last| facts.now.saturating_sub(last.at) < VERDICT_SECS)
            .and_then(|last| {
                let quality = last.quality.as_ref()?;
                let which = match last.page {
                    Some(page) => format!("Page {page}"),
                    None => "Scan".to_string(),
                };
                Some(if !last.kept {
                    (false, "Not kept: nothing on it".to_string())
                } else if quality.ok() {
                    (true, format!("{which}: good to read"))
                } else {
                    (false, format!("{which}: {}", quality.summary()))
                })
            });
        // An envelope is judged by nothing but being one.
        let envelope = facts
            .last
            .is_some_and(|last| last.envelope && facts.now.saturating_sub(last.at) < VERDICT_SECS);
        let verdict = if envelope {
            Some((true, "Envelope: a new letter".to_string()))
        } else {
            verdict
        };
        let bad = verdict.as_ref().is_some_and(|(ok, _)| !ok);

        let filing = relevant_filing(facts);
        // Where the letter goes is the whole screen between letters. Once the
        // next one is started it gives way to that one, and is a line at the
        // bottom.
        let between_letters = !facts.finishing
            && matches!(facts.state, "waiting" | "paused")
            && facts.open_pages == 0;
        if let Some(filing) = filing.filter(|_| between_letters) {
            let (headline, detail) = where_it_goes(&filing.outcome);
            return Self {
                headline,
                detail,
                verdict: None,
                footer: footer(facts, &["Ready for the next letter".to_string()]),
                tone: tone_of(&filing.outcome),
                ..Self::default()
            };
        }

        let (headline, detail, tone) = match facts.state {
            _ if facts.finishing => ("Sending letter", "Just a moment", Tone::Busy),
            "no-camera" => ("No camera", "Check the camera cable", Tone::Problem),
            "starting" => ("Starting", "", Tone::Busy),
            "settling" => ("Hold still", "Taking the photograph", Tone::Busy),
            "photographed" if bad => ("Check scan", "Next page, or take it away", Tone::Problem),
            "photographed" => ("Scanned", "Next page, or take it away", Tone::Done),
            _ if facts.open_pages > 0 => ("Ready", "Next page, or finish the letter", Tone::Ready),
            _ => ("Ready", "Put a page down", Tone::Ready),
        };

        let mut first = Vec::new();
        if facts.collecting && facts.open_pages > 0 {
            let plural = if facts.open_pages == 1 { "" } else { "s" };
            first.push(format!("Letter: {} page{plural}", facts.open_pages));
        }
        if let Some(filing) = filing.filter(|f| f.decided_at.is_some()) {
            first.push(format!("Last: {}", where_it_goes(&filing.outcome).0));
        }

        Self {
            headline: headline.to_string(),
            detail: detail.to_string(),
            verdict,
            footer: footer(facts, &first),
            tone,
            ..Self::default()
        }
    }
}

/// A letter's time of day, as the display says it — in the Pi's own time
/// zone. Seconds since the epoch are UTC, and dividing them by an hour said
/// 11:54 for a letter photographed at 13:54 in Hamburg.
fn clock(at: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(at as i64, 0).single() {
        Some(time) => time.format("%H:%M").to_string(),
        None => "--:--".to_string(),
    }
}

/// "Undo last letter", or its second tap.
fn undo_letter(facts: &Facts<'_>) -> Action {
    match facts.confirming {
        Some(Confirming::UndoLetter) => Action::ConfirmUndoLetter,
        _ => Action::UndoLetter,
    }
}

/// The letter sent last, while where it goes is worth saying: being read, or
/// known for [`VERDICT_SECS`].
fn relevant_filing<'a>(facts: &Facts<'a>) -> Option<&'a Filing> {
    facts.filing.filter(|filing| match filing.decided_at {
        None => facts.now.saturating_sub(filing.sent_at) < GIVE_UP_SECS,
        Some(at) => {
            filing.outcome != Outcome::TimedOut && facts.now.saturating_sub(at) < VERDICT_SECS
        }
    })
}

fn tone_of(outcome: &Outcome) -> Tone {
    match outcome {
        Outcome::Reading => Tone::Busy,
        Outcome::Folders(folders) if folders.iter().any(|f| f.discard) => Tone::Bin,
        Outcome::Folders(folders) if !folders.is_empty() => Tone::Folder,
        Outcome::Duplicate | Outcome::Nowhere => Tone::Done,
        Outcome::Folders(_) | Outcome::Failed(_) | Outcome::TimedOut => Tone::Problem,
        Outcome::Deleted => Tone::Bin,
    }
}

/// The folder in large letters, and a line saying what to do with it.
fn where_it_goes(outcome: &Outcome) -> (String, String) {
    // Who the letter is for is said as well as where it goes: in a household
    // where more than one person gets post — a spouse, someone whose papers
    // are being kept — the folder alone does not say whose pile it joins.
    let (folders, people): (Vec<&Folder>, Vec<&Folder>) = match outcome {
        Outcome::Folders(all) => all.iter().partition(|folder| !folder.person),
        _ => (Vec::new(), Vec::new()),
    };
    let names: Vec<&str> = people.iter().map(|who| who.name.as_str()).collect();
    let for_whom = (!names.is_empty()).then(|| format!("For {}", names.join(" and ")));
    let (headline, detail) = match outcome {
        Outcome::Reading => ("Sorting...".into(), "Paperless is reading the letter"),
        Outcome::Folders(_) if folders.iter().any(|f| f.discard) => {
            ("Throw away".into(), "Paperless keeps a copy")
        }
        Outcome::Folders(_) if folders.len() == 1 => {
            (folders[0].name.clone(), "Put the letter in this folder")
        }
        Outcome::Folders(_) if !folders.is_empty() => {
            let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
            (names.join(" / "), "It fits more than one folder")
        }
        Outcome::Folders(_) => ("Which folder?".into(), "Paperless could not tell"),
        Outcome::Duplicate => ("Already filed".into(), "Paperless had this letter"),
        Outcome::Failed(_) => ("Not filed".into(), "See the setup page"),
        Outcome::TimedOut => ("Which folder?".into(), "Paperless took too long"),
        Outcome::Deleted => ("Taken back".into(), "Deleted from Paperless"),
        Outcome::Nowhere => ("No folder".into(), "Kept in Paperless, on no shelf"),
    };
    // Whose it is takes the line: which folder is already the headline, and
    // "put the letter in this folder" is what the headline means anyway.
    (headline, for_whom.unwrap_or_else(|| detail.to_string()))
}

/// `first`, then what is waiting to be sent.
fn footer(facts: &Facts<'_>, first: &[String]) -> String {
    let mut parts = first.to_vec();
    parts.push(match facts.waiting {
        0 => "All sent".to_string(),
        n => format!("{n} waiting to send"),
    });
    parts.join("  ·  ")
}

/// One bit a pixel in the panel's own layout: rows of [`ROW_BYTES`] along the
/// short side, 250 of them, most significant bit first, 1 for white.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub bytes: Vec<u8>,
    /// Upside down, for a panel mounted the other way round.
    flip: bool,
}

impl Frame {
    pub fn white(flip: bool) -> Self {
        Self {
            bytes: vec![0xFF; ROW_BYTES * WIDTH as usize],
            flip,
        }
    }

    /// Whether the pixel at `x, y`, as the screen is looked at, is black.
    pub fn is_black(&self, x: u32, y: u32) -> bool {
        let (index, bit) = self.locate(x, y);
        self.bytes[index] & bit == 0
    }

    /// The byte and bit of a pixel. The panel is portrait, 122 wide and 250
    /// tall; the screen is that turned a quarter anticlockwise.
    fn locate(&self, x: u32, y: u32) -> (usize, u8) {
        let (column, row) = if self.flip {
            (HEIGHT - 1 - y, x)
        } else {
            (y, WIDTH - 1 - x)
        };
        (
            row as usize * ROW_BYTES + column as usize / 8,
            0x80 >> (column % 8),
        )
    }
}

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for Frame {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 || point.x >= WIDTH as i32 || point.y >= HEIGHT as i32 {
                continue;
            }
            let (index, bit) = self.locate(point.x as u32, point.y as u32);
            match color {
                BinaryColor::On => self.bytes[index] &= !bit,
                BinaryColor::Off => self.bytes[index] |= bit,
            }
        }
        Ok(())
    }
}

const MARGIN: i32 = 6;

/// Draws a screen. Text that does not fit is set in a smaller size, and cut
/// short only if even the smallest is too wide.
pub fn render(screen: &Screen, flip: bool) -> Frame {
    let mut frame = Frame::white(flip);
    let width = WIDTH as i32 - 2 * MARGIN;
    let black = BinaryColor::On;
    let white = BinaryColor::Off;

    let headline = [
        FontRenderer::new::<fonts::u8g2_font_helvB24_tf>(),
        FontRenderer::new::<fonts::u8g2_font_helvB18_tf>(),
    ];
    text(
        &mut frame,
        &headline,
        &screen.headline,
        Point::new(MARGIN, 4),
        width,
        black,
    );

    let detail = [
        FontRenderer::new::<fonts::u8g2_font_helvR14_tf>(),
        FontRenderer::new::<fonts::u8g2_font_helvR12_tf>(),
    ];
    text(
        &mut frame,
        &detail,
        &screen.detail,
        Point::new(MARGIN, 36),
        width,
        black,
    );

    if let Some((ok, words)) = &screen.verdict {
        let bar = Rectangle::new(Point::new(0, 60), Size::new(WIDTH, 30));
        let verdict = [
            FontRenderer::new::<fonts::u8g2_font_helvB14_tf>(),
            FontRenderer::new::<fonts::u8g2_font_helvB12_tf>(),
            FontRenderer::new::<fonts::u8g2_font_helvB10_tf>(),
        ];
        // A tick beside good news; bad news the other way round, white on
        // black, so it is seen from across the room.
        let (ink, left) = if *ok {
            tick(&mut frame, Point::new(MARGIN, 67));
            (black, MARGIN + 24)
        } else {
            let _ = bar
                .into_styled(PrimitiveStyle::with_fill(black))
                .draw(&mut frame);
            (white, MARGIN)
        };
        text(
            &mut frame,
            &verdict,
            words,
            Point::new(left, 67),
            WIDTH as i32 - MARGIN - left,
            ink,
        );
    }

    let _ = Line::new(Point::new(0, 97), Point::new(WIDTH as i32, 97))
        .into_styled(PrimitiveStyle::with_stroke(black, 1))
        .draw(&mut frame);
    let footer = [
        FontRenderer::new::<fonts::u8g2_font_helvR10_tf>(),
        FontRenderer::new::<fonts::u8g2_font_helvR08_tf>(),
    ];
    text(
        &mut frame,
        &footer,
        &screen.footer,
        Point::new(MARGIN, 104),
        width,
        black,
    );

    frame
}

/// `words` in the first of `fonts` they fit in, their top left at `at`.
pub(crate) fn text<D, C>(
    frame: &mut D,
    fonts: &[FontRenderer],
    words: &str,
    at: Point,
    width: i32,
    color: C,
) where
    D: DrawTarget<Color = C>,
    C: PixelColor,
{
    if words.is_empty() {
        return;
    }
    let fonts: Vec<FontRenderer> = fonts
        .iter()
        .map(|font| font.clone().with_ignore_unknown_chars(true))
        .collect();
    let fits = |font: &FontRenderer, words: &str| {
        font.get_rendered_dimensions(words, Point::zero(), VerticalPosition::Top)
            .ok()
            .and_then(|dimensions| dimensions.bounding_box)
            .is_none_or(|bounds| bounds.size.width as i32 <= width)
    };
    let smallest = fonts.last().expect("at least one font");
    let (font, words) = match fonts.iter().find(|font| fits(font, words)) {
        Some(font) => (font, words.to_string()),
        None => {
            // Cut at a character boundary until it fits, with an ellipsis.
            let mut cut: Vec<char> = words.chars().collect();
            let mut shortened = words.to_string();
            while cut.pop().is_some() {
                shortened = format!("{}...", cut.iter().collect::<String>().trim_end());
                if fits(smallest, &shortened) {
                    break;
                }
            }
            (smallest, shortened)
        }
    };
    let _ = font.render_aligned(
        words.as_str(),
        at,
        VerticalPosition::Top,
        HorizontalAlignment::Left,
        FontColor::Transparent(color),
        frame,
    );
}

/// A tick, 18 pixels across, three thick.
fn tick(frame: &mut Frame, at: Point) {
    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 3);
    let _ = Line::new(at + Point::new(1, 9), at + Point::new(6, 15))
        .into_styled(style)
        .draw(frame);
    let _ = Line::new(at + Point::new(6, 15), at + Point::new(17, 1))
        .into_styled(style)
        .draw(frame);
}

/// Something that shows frames: the panel, or a test's record of them.
pub trait Panel: Send {
    /// `full`: the slow refresh that clears ghosts; otherwise a partial one.
    fn show(&mut self, frame: &Frame, full: bool) -> Result<()>;
}

/// Something a screen can be drawn on: the e-paper panel with its refresh
/// policy, or the LCD.
pub trait Surface: Send {
    fn draw(&mut self, screen: &Screen) -> Result<()>;
}

/// The e-paper panel, and when it gets a full refresh: the first picture,
/// the first after a failure, and every [`FULL_EVERY`] partial refreshes.
pub struct Epaper {
    panel: Box<dyn Panel>,
    flip: bool,
    partials: u32,
    fresh: bool,
}

impl Epaper {
    pub fn new(panel: Box<dyn Panel>, flip: bool) -> Self {
        Self {
            panel,
            flip,
            partials: 0,
            fresh: true,
        }
    }
}

impl Surface for Epaper {
    fn draw(&mut self, screen: &Screen) -> Result<()> {
        let full = self.fresh || self.partials >= FULL_EVERY;
        match self.panel.show(&render(screen, self.flip), full) {
            Ok(()) => {
                self.partials = if full { 0 } else { self.partials + 1 };
                self.fresh = false;
                Ok(())
            }
            Err(err) => {
                // Start again from a full refresh once it answers.
                self.fresh = true;
                Err(err)
            }
        }
    }
}

/// The display, driven from a thread of its own: a refresh takes up to two
/// seconds of waiting on the panel, which the capture loop should not spend.
pub struct Display {
    screens: Sender<Screen>,
}

impl Display {
    pub fn spawn(surface: Box<dyn Surface>) -> Self {
        let (screens, receiver) = mpsc::channel();
        std::thread::spawn(move || draw_all(surface, receiver));
        Self { screens }
    }

    /// Shows `screen` when the panel is next free. Screens that arrive while
    /// it is busy replace each other: only the latest is drawn.
    pub fn show(&self, screen: Screen) {
        let _ = self.screens.send(screen);
    }
}

/// Drives an e-paper panel with screens as they come until the sender is
/// dropped. Public so a test can run it on its own thread and see what
/// reached the panel.
pub fn drive(panel: Box<dyn Panel>, screens: Receiver<Screen>, flip: bool) {
    draw_all(Box::new(Epaper::new(panel, flip)), screens);
}

/// Draws screens as they come, the latest of any that queued up and only
/// when it differs from what is shown, until the sender is dropped.
pub fn draw_all(mut surface: Box<dyn Surface>, screens: Receiver<Screen>) {
    let mut shown: Option<Screen> = None;
    let mut failures = 0u32;
    while let Ok(mut screen) = screens.recv() {
        while let Ok(newer) = screens.try_recv() {
            screen = newer;
        }
        if shown.as_ref() == Some(&screen) {
            continue;
        }
        match surface.draw(&screen) {
            Ok(()) => {
                shown = Some(screen);
                failures = 0;
            }
            Err(err) => {
                failures += 1;
                if failures <= 3 || failures.is_multiple_of(50) {
                    tracing::warn!(%err, failures, "could not update the display");
                }
                shown = None;
            }
        }
    }
}
