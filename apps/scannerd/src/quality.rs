//! Is the photograph good enough to read?
//!
//! Paperless will OCR whatever it is given, and a page it cannot read is only
//! found out about later, when the letter does not turn up in a search. So the
//! photograph is looked at once, as it is taken, while the page is still in
//! someone's hand.
//!
//! Like detection, this is arithmetic over a luma plane. It looks at the middle
//! of the photograph, where the page lies, and asks four things:
//!
//! **Is there a page?** Paper is the brightest thing on a desk. If the middle is
//! not bright, the camera photographed the table, a hand, or the ceiling.
//!
//! **Is the text dark?** On the first pages from the Pi the paper came out at
//! 255 over most of the photograph and the darkest text at about 220: legible
//! to a person squinting, faint to OCR. Paper at full white *and* ink that is
//! not dark is an overexposed page.
//!
//! **Is there any text?** A letter has lines of it, and each line has edges:
//! on the first pages from the rig, 14–32% of the small blocks the middle is
//! cut into held one. A photograph with hardly any is a blank page — the back
//! of a letter, which is worth saying but not wrong — or not a page at all: the
//! first photographs with the display attached were of the ceiling, grey and
//! bright enough to pass for paper, with a lamp in it whose edges filled 1%.
//!
//! **Is it sharp?** Measured in small blocks, each against its own contrast: a
//! sharp edge between ink and paper crosses most of that contrast within a
//! couple of pixels, a blurred one spreads it out. Against its own contrast,
//! because a faint page and a dark one should not differ in sharpness, and in
//! blocks, because the edge of the table is one very sharp line that a measure
//! of the whole picture would be dominated by.
//!
//! The thresholds were set against those first photographs and blurred copies
//! of them: the real ones score 0.77–0.98, blurred by a pixel about 0.65, by two
//! pixels about 0.43.

use anyhow::{Context, Result};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// Something wrong with a photograph, worst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The middle is not bright enough to be paper.
    NoPage,
    /// Paper clipped to white and the text pale.
    WashedOut,
    Blurry,
    /// No text anywhere: a blank page, or no page.
    NoText,
}

impl Problem {
    /// A word or two: they have to fit a 2.13″ display after "Page 12: ".
    pub fn describe(self) -> &'static str {
        match self {
            Problem::NoPage => "no page / too dark",
            Problem::WashedOut => "too bright",
            Problem::Blurry => "blurry",
            Problem::NoText => "no text found",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    /// How dark the darkest text is: the 0.5th percentile, 0–255.
    pub ink: u8,
    /// How bright the paper is: the median, 0–255.
    pub paper: u8,
    /// Median over blocks with something in them, 0–1. `None` when too few
    /// blocks had anything in them to say.
    pub sharpness: Option<f32>,
    pub problems: Vec<Problem>,
}

impl Quality {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    /// "good" or what is wrong, for a sentence.
    pub fn summary(&self) -> String {
        if self.ok() {
            return "good".to_string();
        }
        self.problems
            .iter()
            .map(|problem| problem.describe())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Below this median, the middle of the photograph is not paper.
const PAPER: u8 = 140;
/// Paper at or above this is clipped: the exposure ran out.
const CLIPPED: u8 = 245;
/// Text lighter than this on clipped paper is washed out.
const PALE_INK: u8 = 150;
/// Blocks sharper than this, on average, are sharp enough.
const SHARP: f32 = 0.5;
/// The side of a sharpness block, in pixels.
const BLOCK: usize = 24;
/// A block with less contrast than this has no edge in it worth measuring.
const BLOCK_CONTRAST: i16 = 30;
/// A smaller share of blocks than this with something in them is a photograph
/// with no text in it, whose sharpness is not judged.
const TEXT: f32 = 0.03;

/// The paper in a greyscale picture, one byte per pixel, row-major — or its
/// middle three fifths, when no paper is made out.
fn where_the_paper_is(luma: &[u8], width: usize, height: usize) -> (usize, usize, usize, usize) {
    let middle = (width / 5, height / 5, width * 4 / 5, height * 4 / 5);
    // A small copy is enough to find a sheet of paper in.
    let step = (width.max(height) / 400).max(1);
    let (sw, sh) = (width / step, height / step);
    if sw < 8 || sh < 8 {
        return middle;
    }
    let small: Vec<u8> = (0..sh)
        .flat_map(|y| (0..sw).map(move |x| luma[(y * step) * width + x * step]))
        .collect();
    let Some((corners, lit)) = crate::locate::page_corners(&small, sw, sh) else {
        return middle;
    };
    // Not the paper, whatever it is: a reflection, a stamp, the window of an
    // envelope.
    if lit < 0.2 {
        return middle;
    }
    let xs = corners.0.map(|(x, _)| x);
    let ys = corners.0.map(|(_, y)| y);
    let least = |values: [f64; 4]| values.into_iter().fold(1.0, f64::min).clamp(0.0, 1.0);
    let most = |values: [f64; 4]| values.into_iter().fold(0.0, f64::max).clamp(0.0, 1.0);
    // A little in from its edges, where a page curls and the table shows.
    let inset = 0.025;
    let (left, right) = (least(xs) + inset, most(xs) - inset);
    let (top, bottom) = (least(ys) + inset, most(ys) - inset);
    if right - left < 0.2 || bottom - top < 0.2 {
        return middle;
    }
    (
        (left * width as f64) as usize,
        (top * height as f64) as usize,
        ((right * width as f64) as usize).min(width),
        ((bottom * height as f64) as usize).min(height),
    )
}

/// Looks at the paper in a greyscale picture, one byte per pixel, row-major.
///
/// The paper rather than the middle of the picture: a photograph is trimmed to
/// the page it shows before it is judged, and the middle of a page whose
/// letter ends half way down is blank — which read as no text and as ink of
/// 203 against paper of 217, and called a perfectly good second page
/// unreadable.
pub fn assess(luma: &[u8], width: usize, height: usize) -> Quality {
    let (left, top, right, bottom) = where_the_paper_is(luma, width, height);
    let rows = || (top..bottom).map(move |y| &luma[y * width + left..y * width + right]);

    let mut histogram = [0u64; 256];
    for row in rows() {
        for &value in row {
            histogram[value as usize] += 1;
        }
    }
    let total: u64 = histogram.iter().sum();
    let ink = percentile(&histogram, total, 0.005);
    let paper = percentile(&histogram, total, 0.5);

    let sharpness = sharpness(luma, width, (left, top, right, bottom));

    let mut problems = Vec::new();
    if paper < PAPER {
        problems.push(Problem::NoPage);
    } else {
        let clipped = paper >= CLIPPED;
        if clipped && ink > PALE_INK {
            problems.push(Problem::WashedOut);
        }
        if sharpness.is_some_and(|sharpness| sharpness < SHARP) {
            problems.push(Problem::Blurry);
        }
        if sharpness.is_none() {
            problems.push(Problem::NoText);
        }
    }

    Quality {
        ink,
        paper,
        sharpness,
        problems,
    }
}

/// Decodes the greyscale of a JPEG and looks at it. Greyscale only: the
/// colour is neither needed nor, on a Pi Zero, cheap.
pub fn assess_jpeg(jpeg: &[u8]) -> Result<Quality> {
    let (luma, width, height) = read_luma(jpeg)?;
    Ok(assess(&luma, width, height))
}

/// The greyscale of a JPEG, and its width and height.
pub fn read_luma(jpeg: &[u8]) -> Result<(Vec<u8>, usize, usize)> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let luma = decoder
        .decode()
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .context("could not read the photograph")?;
    let (width, height) = decoder.dimensions().context("the photograph has no size")?;
    if luma.len() < width * height {
        anyhow::bail!("the photograph decoded to fewer pixels than its size");
    }
    Ok((luma, width, height))
}

/// The exposure scannerd will try, in stops from the camera's own choice.
pub const EV_DARKEST: f32 = -4.0;
pub const EV_BRIGHTEST: f32 = 1.0;
/// Paper this bright or brighter is at the edge of clipping: a page that is
/// still readable, but the next is taken half a stop darker.
const BRIGHT_PAPER: u8 = 245;
/// Paper darker than this on a readable page: the next half a stop brighter.
const DIM_PAPER: u8 = 180;

/// What to do about the exposure after a photograph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Exposure {
    /// Take the same page again at this exposure: this one is not readable
    /// for being too bright or too dark.
    Retake(f32),
    /// Keep the photograph, and take the next page at this exposure.
    Keep(f32),
}

/// Whether a photograph taken at `ev` should be taken again, lighter or
/// darker, and what the next one should be taken at.
///
/// Too bright is the common case: white paper fools the camera, which exposes
/// for a grey world. A stop darker at a time, while the page is still lying
/// there. Too dark is only believed when there is text in the photograph —
/// a dark middle with nothing in it is the table, and brightening the camera
/// for the table would wash out the next page.
pub fn exposure(ev: f32, quality: &Quality) -> Exposure {
    let clamp = |ev: f32| ev.clamp(EV_DARKEST, EV_BRIGHTEST);
    if quality.problems.contains(&Problem::WashedOut) && ev - 1.0 >= EV_DARKEST {
        return Exposure::Retake(ev - 1.0);
    }
    if quality.problems.contains(&Problem::NoPage)
        && quality.sharpness.is_some()
        && ev + 1.0 <= EV_BRIGHTEST
    {
        return Exposure::Retake(ev + 1.0);
    }
    if quality.ok() && quality.paper >= BRIGHT_PAPER {
        return Exposure::Keep(clamp(ev - 0.5));
    }
    if quality.ok() && quality.paper < DIM_PAPER {
        return Exposure::Keep(clamp(ev + 0.5));
    }
    Exposure::Keep(clamp(ev))
}

/// Whether `a` is a better photograph of the same page than `b`: readable
/// over not, fewer problems over more, and paper nearer a comfortable white.
pub fn better(a: &Quality, b: &Quality) -> bool {
    let rank = |q: &Quality| {
        (
            !q.ok(),
            q.problems.len(),
            (q.paper as i32 - 215).unsigned_abs(),
        )
    };
    rank(a) < rank(b)
}

fn percentile(histogram: &[u64; 256], total: u64, fraction: f64) -> u8 {
    let wanted = (total as f64 * fraction) as u64;
    let mut seen = 0;
    for (value, &count) in histogram.iter().enumerate() {
        seen += count;
        if seen > wanted {
            return value as u8;
        }
    }
    255
}

/// For each block with an edge in it, the steepest step across two pixels
/// as a share of the block's range; the median of those. `None` when too few
/// blocks have an edge in them to be text.
fn sharpness(
    luma: &[u8],
    width: usize,
    (left, top, right, bottom): (usize, usize, usize, usize),
) -> Option<f32> {
    let mut blocks = 0usize;
    let at = |x: usize, y: usize| luma[y * width + x] as i16;
    let mut ratios = Vec::new();
    let mut y = top;
    while y + BLOCK <= bottom {
        let mut x = left;
        while x + BLOCK <= right {
            let (mut low, mut high, mut steepest) = (255i16, 0i16, 0i16);
            for by in y..y + BLOCK {
                for bx in x..x + BLOCK {
                    let value = at(bx, by);
                    low = low.min(value);
                    high = high.max(value);
                    if bx + 2 < x + BLOCK {
                        steepest = steepest.max((at(bx + 2, by) - value).abs());
                    }
                    if by + 2 < y + BLOCK {
                        steepest = steepest.max((at(bx, by + 2) - value).abs());
                    }
                }
            }
            let range = high - low;
            blocks += 1;
            if range >= BLOCK_CONTRAST {
                ratios.push(steepest as f32 / range as f32);
            }
            x += BLOCK;
        }
        y += BLOCK;
    }
    if blocks == 0 || (ratios.len() as f32) < blocks as f32 * TEXT {
        return None;
    }
    ratios.sort_by(|a, b| a.total_cmp(b));
    Some(ratios[ratios.len() / 2])
}
