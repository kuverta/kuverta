//! A colour LCD: the 3.5″ SPI panel the rig moved to with the Pi 4.
//!
//! Identified on the rig: nothing on HDMI, and on SPI chip select 1 an
//! XPT2046 touch controller whose temperature channels read the room. That is
//! the common 3.5″ Pi display — an ILI9486, 480×320, data/command on GPIO 24,
//! reset on 25, touch interrupt on 17 — which the kernel's `piscreen` overlay
//! drives as `/dev/fb0`:
//!
//! ```text
//! dtoverlay=piscreen,speed=24000000,rotate=90
//! ```
//!
//! So there is no driver here. The kernel has the panel; this paints a screen
//! ([`crate::paint`]: smooth shapes, Inter for the type) and writes it to the
//! framebuffer, which is a file, in the panel's 16-bit colour.
//!
//! The words are the e-paper's ([`Screen`]); the room is not. A card holds the
//! headline at four times the size, with an icon and a bar in the colour of
//! the news — green ready, amber working, teal done, red look, blue a folder —
//! so it can be read from across the room.
//!
//! It is also touched. Between letters it offers **Start scanning** and
//! **Learn empty table**; scanning, it shows what the camera sees beside the
//! page it photographed last, with **Finish letter** and **Stop scanning**
//! under them. Every button is a band the width of the screen, because the
//! touch film on the rig's panel reports only how far down a finger is — its
//! other axis reads the same wherever it is pressed — and a band needs nothing
//! more. See [`crate::touch`].

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use tiny_skia::{Color, Pixmap};

use crate::display::{Action, Panes, Screen, Surface, Tone};
use crate::paint::{self, Icon, Weight};

/// The panel's own colour: five bits of red, six of green, five of blue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

impl Canvas {
    /// The colour at a point, back in eight bits a channel.
    pub fn at(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let raw = self.pixels[(y * self.width + x) as usize];
        let (r, g, b) = (
            (raw >> 11) as u8,
            ((raw >> 5) & 0x3F) as u8,
            (raw & 0x1F) as u8,
        );
        (r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2)
    }

    /// As the framebuffer takes it: two bytes a pixel, little-endian.
    pub fn bytes(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect()
    }

    /// A painted picture in the panel's colour. Five and six bits a channel
    /// would band a gradient in stripes you can count, so the rounding is
    /// spread about with an ordered dither.
    pub fn from_pixmap(pixmap: &Pixmap) -> Self {
        // Bayer 4×4, the smallest dither that does not look like a pattern.
        const BAYER: [[u16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        let (width, height) = (pixmap.width(), pixmap.height());
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for (i, pixel) in pixmap.pixels().iter().enumerate() {
            let (x, y) = (i as u32 % width, i as u32 / width);
            let noise = BAYER[(y % 4) as usize][(x % 4) as usize];
            // What each channel loses to rounding: three bits, two, three.
            let quantise = |value: u8, keep: u32| -> u16 {
                let step = 1u16 << (8 - keep);
                let spread = noise * step / 16;
                let raised = (value as u16 + spread).min(255);
                raised >> (8 - keep)
            };
            let (r, g, b) = (
                quantise(pixel.demultiply().red(), 5),
                quantise(pixel.demultiply().green(), 6),
                quantise(pixel.demultiply().blue(), 5),
            );
            pixels.push((r << 11) | (g << 5) | b);
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

// -- the look ---------------------------------------------------------------------------

// kuverta's own colours, in its dark shades (`apps/desktop/ui/styles.css`):
// the screen on the rig and the window on the desk are one thing to look at,
// so petrol is the accent here too and amber means the same as it does there.

/// The page: kuverta's dark background, a shade lighter at the bottom.
fn background_top() -> Color {
    paint::colour(22, 26, 27)
}
fn background_bottom() -> Color {
    paint::colour(27, 31, 32)
}
fn card_top() -> Color {
    paint::colour(34, 40, 42)
}
fn card_bottom() -> Color {
    paint::colour(27, 31, 32)
}
/// What is read.
fn ink() -> Color {
    paint::colour(231, 234, 234)
}
/// What is read second.
fn muted() -> Color {
    paint::colour(140, 149, 150)
}
/// What is only there when looked for.
fn quiet() -> Color {
    paint::colour(109, 118, 119)
}

/// The colour of a kind of news.
pub fn accent(tone: Tone) -> Color {
    match tone {
        // Petrol, kuverta's own: waiting for a page is the resting state.
        Tone::Ready => paint::colour(42, 127, 124),
        // Amber, which in the window also means "this wants dealing with".
        Tone::Busy => paint::colour(234, 168, 67),
        Tone::Done => paint::colour(92, 201, 138),
        Tone::Problem => paint::colour(255, 107, 97),
        // The answer to the question the rig is there to answer: petrol,
        // bright enough to read across a room.
        Tone::Folder => paint::colour(127, 196, 193),
        Tone::Bin => paint::colour(140, 149, 150),
    }
}

/// The same as the panel keeps it, for a test to look for.
pub fn band(tone: Tone) -> (u8, u8, u8) {
    let colour = accent(tone);
    (
        (colour.red() * 255.0) as u8,
        (colour.green() * 255.0) as u8,
        (colour.blue() * 255.0) as u8,
    )
}

/// Whether a colour is light enough to want dark letters on it.
fn is_light(colour: Color) -> bool {
    colour.red() * 0.3 + colour.green() * 0.6 + colour.blue() * 0.1 > 0.6
}

/// The drawing beside a headline of this tone.
fn tone_icon(tone: Tone) -> Icon {
    match tone {
        Tone::Ready => Icon::Camera,
        Tone::Busy => Icon::Clock,
        Tone::Done => Icon::Check,
        Tone::Problem => Icon::Warning,
        Tone::Folder => Icon::Folder,
        Tone::Bin => Icon::Bin,
    }
}

/// The drawing on a button.
fn action_icon(action: Action) -> Icon {
    match action {
        Action::StartScanning => Icon::Play,
        Action::StopScanning => Icon::Stop,
        Action::FinishLetter(_) => Icon::Check,
        Action::LearnEmpty => Icon::Table,
        Action::UndoPage | Action::UndoLetter => Icon::Undo,
        Action::CancelLetter(_) | Action::ConfirmCancel(_) => Icon::Bin,
        Action::ConfirmUndoLetter => Icon::Undo,
        Action::FileNowhere => Icon::Folder,
        Action::FileInBin => Icon::Bin,
        Action::OpenQueue(_) => Icon::Clock,
        Action::QueuedLetter(_) => Icon::Envelope,
        Action::ConfirmQueued(_) => Icon::Bin,
        Action::Back => Icon::Back,
    }
}

const MARGIN: f32 = 14.0;
const GAP: f32 = 8.0;

/// Where each button of a screen is, top to bottom, as rows of the panel:
/// `(top, bottom, action)`. What [`render`] draws them at, and what a touch is
/// looked up in. Stacked up from the bottom; shorter when there are three, so
/// the pictures above keep their room.
pub fn rows(screen: &Screen, height: u32) -> Vec<(i32, i32, Action)> {
    let h = height as f32;
    let count = screen.buttons.len();
    if count == 0 {
        return Vec::new();
    }
    let each = match count {
        0..=2 => h * 0.15,
        3..=4 => h * 0.135,
        // Five and more only happen with a letter to file by hand on top of
        // everything else; they still have to fit above the card.
        _ => h * 0.115,
    };
    let top = h - MARGIN - count as f32 * each - (count - 1) as f32 * GAP;
    screen
        .buttons
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let row_top = top + i as f32 * (each + GAP);
            (row_top as i32, (row_top + each) as i32, *action)
        })
        .collect()
}

/// Paints a screen for a panel `width`×`height`, laid out for 480×320.
pub fn render(screen: &Screen, width: u32, height: u32) -> Canvas {
    let mut pixmap = paint::picture(width, height, background_top());
    let whole = (0.0, 0.0, width as f32, height as f32);
    paint::fill_rect(
        &mut pixmap,
        whole,
        paint::down(whole, background_top(), background_bottom()),
    );
    let top_of_buttons = rows(screen, height)
        .first()
        .map(|(top, _, _)| *top as f32 - GAP)
        .unwrap_or(height as f32 - MARGIN);
    match &screen.scanning {
        Some(panes) => paint_scanning(&mut pixmap, screen, panes, top_of_buttons),
        // A list needs its rows: a heading rather than a card.
        None if !screen.queue.is_empty() => paint_heading(&mut pixmap, screen),
        None => paint_card(&mut pixmap, screen, top_of_buttons),
    }
    paint_buttons(&mut pixmap, screen);
    Canvas::from_pixmap(&pixmap)
}

/// Two lines at the top, for a screen that is a list.
fn paint_heading(pixmap: &mut Pixmap, screen: &Screen) {
    let width = pixmap.width() as f32;
    let room = width - 2.0 * MARGIN - 34.0;
    let (headline, size) = paint::fitted(&screen.headline, Weight::Semibold, &[22.0, 20.0], room);
    paint::icon(
        pixmap,
        tone_icon(screen.tone),
        (MARGIN + 4.0, MARGIN),
        22.0,
        accent(screen.tone),
    );
    paint::text(
        pixmap,
        &headline,
        (MARGIN + 34.0, MARGIN),
        Weight::Semibold,
        size,
        ink(),
    );
    let (detail, size) = paint::fitted(&screen.detail, Weight::Regular, &[15.0, 13.0], room + 30.0);
    paint::text(
        pixmap,
        &detail,
        (MARGIN + 34.0, MARGIN + 26.0),
        Weight::Regular,
        size,
        muted(),
    );
}

/// The news on a card: an icon in the colour of it, the headline large, what
/// to do under it, and along the bottom what is waiting.
fn paint_card(pixmap: &mut Pixmap, screen: &Screen, bottom: f32) {
    let width = pixmap.width() as f32;
    let card = (MARGIN, MARGIN, width - 2.0 * MARGIN, bottom - MARGIN);
    paint::rounded(
        pixmap,
        card,
        22.0,
        paint::down(card, card_top(), card_bottom()),
    );
    paint::rounded_edge(pixmap, card, 22.0, paint::faded(ink(), 0.07));
    let tone = accent(screen.tone);

    // The icon in a soft square of its own colour.
    let badge = (card.0 + 22.0, card.1 + 22.0, 58.0, 58.0);
    paint::rounded(pixmap, badge, 18.0, paint::flat(paint::faded(tone, 0.16)));
    paint::icon(
        pixmap,
        tone_icon(screen.tone),
        (badge.0 + 15.0, badge.1 + 15.0),
        28.0,
        tone,
    );

    let left = badge.0 + badge.2 + 20.0;
    let room = card.0 + card.2 - left - 22.0;
    let roomy = card.3 > 190.0;
    let (headline, size) = paint::fitted(
        &screen.headline,
        Weight::Semibold,
        if roomy {
            &[44.0, 38.0, 32.0, 26.0]
        } else {
            &[32.0, 28.0, 24.0, 20.0]
        },
        room,
    );
    let mut y = card.1 + if roomy { 28.0 } else { 24.0 };
    paint::text(pixmap, &headline, (left, y), Weight::Semibold, size, ink());
    y += size * 1.25;

    if !screen.detail.is_empty() {
        let (detail, size) =
            paint::fitted(&screen.detail, Weight::Regular, &[21.0, 18.0, 16.0], room);
        paint::text(pixmap, &detail, (left, y), Weight::Regular, size, muted());
        y += size * 1.7;
    }

    if let Some((ok, words)) = &screen.verdict {
        if y + 30.0 < card.1 + card.3 - 36.0 {
            let colour = accent(if *ok { Tone::Ready } else { Tone::Problem });
            paint::icon(
                pixmap,
                if *ok { Icon::Check } else { Icon::Cross },
                (left, y + 2.0),
                22.0,
                colour,
            );
            let (words, size) =
                paint::fitted(words, Weight::Semibold, &[20.0, 18.0, 16.0], room - 34.0);
            paint::text(
                pixmap,
                &words,
                (left + 34.0, y),
                Weight::Semibold,
                size,
                colour,
            );
        }
    }

    if !screen.footer.is_empty() {
        let (footer, size) =
            paint::fitted(&screen.footer, Weight::Regular, &[15.0, 13.0], room + 30.0);
        paint::text(
            pixmap,
            &footer,
            (card.0 + 24.0, card.1 + card.3 - 28.0),
            Weight::Regular,
            size,
            quiet(),
        );
    }
}

/// Scanning: what to do along the top, where the letter before goes beside
/// it, and the camera next to the page last taken.
fn paint_scanning(pixmap: &mut Pixmap, screen: &Screen, panes: &Panes, bottom: f32) {
    let width = pixmap.width() as f32;
    let tone = accent(screen.tone);
    let bar_height = 36.0;

    // Where a letter goes: a pill, top right. The letter in your hand comes
    // first — that is the live question, and it is answered from the first
    // page read rather than after Paperless has the letter — and it is drawn
    // as an outline, because it is still only a guess. Where the letter
    // before went is filled in, and gives up the corner while there is a
    // guess to show.
    let mut room = width - 2.0 * MARGIN;
    let pill = match (&screen.guess, &screen.last_letter) {
        (Some(guess), _) => Some((accent(Tone::Folder), guess.as_str(), true)),
        (None, Some((folder_tone, words))) => Some((
            accent(*folder_tone),
            words
                .split_once(": ")
                .map(|(_, folder)| folder)
                .unwrap_or(words.as_str()),
            false,
        )),
        (None, None) => None,
    };
    if let Some((colour, folder, guessed)) = pill {
        let (label, size) = paint::fitted(folder, Weight::Semibold, &[16.0, 14.0], width * 0.34);
        let pill_width = paint::measure(&label, Weight::Semibold, size).0 + 52.0;
        let pill = (width - MARGIN - pill_width, MARGIN, pill_width, bar_height);
        let ink = if guessed {
            paint::rounded_edge(pixmap, pill, bar_height / 2.0, colour);
            colour
        } else {
            paint::rounded(pixmap, pill, bar_height / 2.0, paint::flat(colour));
            if is_light(colour) {
                card_bottom()
            } else {
                ink()
            }
        };
        paint::icon(
            pixmap,
            Icon::Folder,
            (pill.0 + 12.0, pill.1 + 9.0),
            19.0,
            ink,
        );
        paint::text(
            pixmap,
            &label,
            (pill.0 + 38.0, pill.1 + (bar_height - size) / 2.0 - 1.0),
            Weight::Semibold,
            size,
            ink,
        );
        room -= pill_width + GAP;
    }

    // What to do now: a dot in the colour of the news, and the words.
    let dot = (MARGIN + 8.0, MARGIN + bar_height / 2.0);
    paint::rounded(
        pixmap,
        (dot.0 - 5.0, dot.1 - 5.0, 10.0, 10.0),
        5.0,
        paint::flat(tone),
    );
    let (headline, size) = paint::fitted(
        &screen.headline,
        Weight::Semibold,
        &[21.0, 19.0, 17.0],
        room - 30.0,
    );
    paint::text(
        pixmap,
        &headline,
        (MARGIN + 24.0, MARGIN + (bar_height - size) / 2.0 - 1.0),
        Weight::Semibold,
        size,
        ink(),
    );

    let top = MARGIN + bar_height + GAP;
    let pane_width = (width - 2.0 * MARGIN - GAP) / 2.0;
    let pane_height = (bottom - top).max(60.0);
    let caption = match &screen.verdict {
        Some((true, words)) => (words.clone(), accent(Tone::Ready)),
        Some((false, words)) => (words.clone(), accent(Tone::Problem)),
        None => ("Last page".to_string(), muted()),
    };
    paint_pane(
        pixmap,
        (MARGIN, top, pane_width, pane_height),
        panes.live.as_ref(),
        "No picture from the camera",
        ("Camera".to_string(), muted()),
    );
    paint_pane(
        pixmap,
        (MARGIN + pane_width + GAP, top, pane_width, pane_height),
        panes.last.as_ref(),
        "No page yet",
        caption,
    );
}

/// A card with a picture in it and a caption under it.
fn paint_pane(
    pixmap: &mut Pixmap,
    rect: (f32, f32, f32, f32),
    picture: Option<&crate::picture::Picture>,
    missing: &str,
    caption: (String, Color),
) {
    paint::rounded(pixmap, rect, 16.0, paint::flat(card_top()));
    paint::rounded_edge(pixmap, rect, 16.0, paint::faded(ink(), 0.06));
    let caption_height = 26.0;
    let inner = (
        rect.0 + 8.0,
        rect.1 + 8.0,
        rect.2 - 16.0,
        rect.3 - 16.0 - caption_height,
    );
    let drawn = picture
        .filter(|picture| picture.width > 0 && picture.height > 0)
        .and_then(|picture| {
            paint::grey_picture(
                pixmap,
                &picture.luma,
                picture.width,
                picture.height,
                inner,
                10.0,
            )
        });
    if drawn.is_none() {
        let (words, size) = paint::fitted(missing, Weight::Regular, &[14.0, 12.0], inner.2 - 8.0);
        let width = paint::measure(&words, Weight::Regular, size).0;
        paint::text(
            pixmap,
            &words,
            (
                inner.0 + (inner.2 - width) / 2.0,
                inner.1 + inner.3 / 2.0 - size / 2.0,
            ),
            Weight::Regular,
            size,
            muted(),
        );
    }
    let (words, size) = paint::fitted(
        &caption.0,
        Weight::Semibold,
        &[15.0, 13.0, 12.0],
        rect.2 - 16.0,
    );
    let width = paint::measure(&words, Weight::Semibold, size).0;
    paint::text(
        pixmap,
        &words,
        (
            rect.0 + (rect.2 - width) / 2.0,
            rect.1 + rect.3 - caption_height + 4.0,
        ),
        Weight::Semibold,
        size,
        caption.1,
    );
}

/// The buttons: the one thing to do in colour, the rest quiet, and what
/// cannot be undone in red.
fn paint_buttons(pixmap: &mut Pixmap, screen: &Screen) {
    let width = pixmap.width() as f32;
    for (top, bottom, action) in rows(screen, pixmap.height()) {
        let rect = (
            MARGIN,
            top as f32,
            width - 2.0 * MARGIN,
            (bottom - top) as f32,
        );
        let radius = rect.3 / 2.0;
        let (ink, icon_colour) = match action {
            Action::StartScanning => {
                let colour = accent(Tone::Ready);
                paint::rounded(
                    pixmap,
                    rect,
                    radius,
                    paint::down(rect, paint::lighter(colour, 0.18), colour),
                );
                (card_bottom(), card_bottom())
            }
            Action::FinishLetter(_) => {
                let colour = accent(Tone::Folder);
                paint::rounded(
                    pixmap,
                    rect,
                    radius,
                    paint::down(rect, paint::lighter(colour, 0.18), colour),
                );
                (ink(), ink())
            }
            Action::ConfirmCancel(_) | Action::ConfirmUndoLetter | Action::ConfirmQueued(_) => {
                let colour = accent(Tone::Problem);
                paint::rounded(
                    pixmap,
                    rect,
                    radius,
                    paint::down(rect, paint::lighter(colour, 0.18), colour),
                );
                (ink(), ink())
            }
            Action::CancelLetter(_) => {
                let colour = accent(Tone::Problem);
                paint::rounded(
                    pixmap,
                    rect,
                    radius,
                    paint::flat(paint::faded(colour, 0.14)),
                );
                paint::rounded_edge(pixmap, rect, radius, paint::faded(colour, 0.5));
                (colour, colour)
            }
            _ => {
                paint::rounded(pixmap, rect, radius, paint::flat(paint::faded(ink(), 0.08)));
                paint::rounded_edge(pixmap, rect, radius, paint::faded(ink(), 0.12));
                (ink(), muted())
            }
        };
        // A letter waiting to be sent says when it came and how often it has
        // been tried; the rest say what they do.
        let label = match action {
            Action::QueuedLetter(index) => screen
                .queue
                .get(index)
                .cloned()
                .unwrap_or_else(|| action.label()),
            _ => action.label(),
        };
        let (label, size) =
            paint::fitted(&label, Weight::Semibold, &[21.0, 19.0, 17.0], rect.2 - 96.0);
        let text_width = paint::measure(&label, Weight::Semibold, size).0;
        paint::text(
            pixmap,
            &label,
            (
                rect.0 + (rect.2 - text_width) / 2.0,
                rect.1 + (rect.3 - size) / 2.0 - 1.0,
            ),
            Weight::Semibold,
            size,
            ink,
        );
        paint::icon(
            pixmap,
            action_icon(action),
            (rect.0 + 22.0, rect.1 + rect.3 / 2.0 - 11.0),
            22.0,
            icon_colour,
        );
    }
}

/// Where the buttons on the screen are, as [`rows`] gives them: written by
/// the display after each picture, read by the touchscreen.
pub type Bands = Arc<Mutex<Vec<(i32, i32, Action)>>>;

/// A Linux framebuffer, `/dev/fbN`, of 16 bits a pixel.
pub struct Framebuffer {
    path: PathBuf,
    width: u32,
    height: u32,
    bands: Bands,
}

impl Framebuffer {
    /// Its size and depth from sysfs, where the kernel says what it made.
    pub fn open(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("{} is not a framebuffer", path.display()))?;
        let sysfs = Path::new("/sys/class/graphics").join(name);
        let read = |file: &str| {
            std::fs::read_to_string(sysfs.join(file)).with_context(|| {
                format!(
                    "{} has no {file}: is the LCD's overlay loaded?",
                    path.display()
                )
            })
        };
        let size = read("virtual_size")?;
        let (width, height) = size
            .trim()
            .split_once(',')
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
            .with_context(|| format!("{} has a size of {size:?}", path.display()))?;
        let depth = read("bits_per_pixel")?;
        if depth.trim() != "16" {
            bail!(
                "{} has {} bits a pixel; scannerd draws 16",
                path.display(),
                depth.trim()
            );
        }
        // Writable now, rather than at the first picture.
        OpenOptions::new().write(true).open(path).with_context(|| {
            format!(
                "could not open {} (is scannerd in the `video` group?)",
                path.display()
            )
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            width,
            height,
            bands: Bands::default(),
        })
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Where the buttons of the picture on the screen are.
    pub fn bands(&self) -> Bands {
        self.bands.clone()
    }
}

impl Surface for Framebuffer {
    fn draw(&mut self, screen: &Screen) -> Result<()> {
        let started = std::time::Instant::now();
        let canvas = render(screen, self.width, self.height);
        let painting = started.elapsed();
        if painting > std::time::Duration::from_millis(300) {
            tracing::warn!(
                ms = painting.as_millis() as u64,
                "the screen took a while to paint"
            );
        }
        let mut file = OpenOptions::new().write(true).open(&self.path)?;
        file.write_all(&canvas.bytes())?;
        // Only once it is on the screen: a tap is on what can be seen.
        *self.bands.lock().unwrap() = rows(screen, self.height);
        Ok(())
    }
}
