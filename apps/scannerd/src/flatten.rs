//! Making a photograph of paper look like paper.
//!
//! A page on the rig's table is lit by whatever is in the room, and the camera
//! exposes for the whole frame — brown table included. What comes out is a
//! sheet that is grey where it should be white, warm on one side and cool on
//! the other, with the text a dull grey rather than black. It is readable, and
//! it looks like a photograph of a letter instead of a scan of one. Tesseract
//! reads it worse for the same reason a person does: there is less between the
//! ink and the paper than there should be.
//!
//! So the paper is found and told it is white. In tiles, because the light
//! across a page is never even — a lamp to one side leaves a gradient that a
//! single white point cannot take out — and per channel, because paper is
//! neutral and the difference between its channels is the colour of the light.
//! Correcting each channel by its own paper level is a white balance measured
//! off the one thing in the frame whose colour is known.
//!
//! **Nothing here decides anything.** It is a tone curve and no more: no pixel
//! moves, nothing is sharpened, nothing is denoised, and the page that comes
//! out is the page that went in with a different brightness. Thin strokes are
//! what the OCR reads and every filter that would smooth the noise would smooth
//! them too.
//!
//! **Every doubt leaves the picture alone.** A tile too dark to hold paper
//! borrows the page's own level rather than being scaled up by a wild factor;
//! a page with no ink in it is not stretched, because the only contrast to
//! stretch would be the paper's own grain; a picture too small to tile is
//! returned untouched. A photograph that is left as it was taken is a
//! photograph; one that has been stretched against a level that was not the
//! paper is scrap.

/// Tiles along the picture's long side. About eighty pixels a tile on a page
/// from the rig: fine enough to follow a lamp across a sheet, coarse enough
/// that a tile still has paper in it between the lines of a letter.
const TILES: usize = 24;
/// Where in a tile's own brightness the paper is taken to be. Not the
/// brightest pixel — that is a specular glint off a staple or a fold — and
/// well above the median, which on a dense tile is ink.
const PAPER_AT: f64 = 0.92;
/// A tile's paper is never believed darker than this share of the page's. A
/// tile that is all ink, or the shadow under a curled corner, has no paper in
/// it to measure, and scaling it up by whatever its own darkness suggests is
/// how a shadow becomes a grey cloud.
const DIMMEST: f64 = 0.55;
/// Passes of 3×3 averaging over the tile grid. Light across a table is smooth;
/// the measurements of it are not, and an unsmoothed field prints the tile
/// grid onto the page.
const SMOOTHINGS: usize = 4;
/// What paper is made. Not 255: a page that clips has no grain left to show
/// where the paper is, and [`crate::quality`] would have nothing to judge if
/// it ever looked at one of these.
const PAPER_BECOMES: f64 = 247.0;
/// Where the ink is taken to be, once the light is out of the picture. Half a
/// percent of a letter's pixels are the middles of its strokes.
const INK_AT: f64 = 0.005;
/// Ink must be at least this far below the paper for the contrast between them
/// to be stretched. A blank page has no ink, and its darkest half percent is
/// paper grain: stretching against that turns a clean sheet into noise.
const INK_BELOW: f64 = 45.0;
/// And the black point is never set above this share of the paper's level,
/// whatever the measurement says. A page that is mostly photograph has a
/// dark half percent that is not ink.
const BLACK_AT_MOST: f64 = 0.70;

/// What a page looked like before it was flattened: the two numbers the
/// exposure is steered by.
///
/// They are carried out of here because flattening destroys them on purpose.
/// Every page that comes out of it has paper at [`PAPER_BECOMES`], which is
/// exactly what [`crate::quality::exposure`] reads to decide whether the next
/// page should be taken darker — so a rig judging its own flattened pages
/// would see a perfect exposure every time, walk the camera down half a stop
/// a page, and never learn otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tone {
    /// The page's median, 0–255: the paper.
    pub paper: u8,
    /// Its half-percentile: the ink.
    pub ink: u8,
}

/// Flattens the light off a page and puts the paper at white, in place.
///
/// `rgb` is packed three bytes a pixel, row-major. Returns what the page
/// looked like before, or `None` when it was left alone — too small to tile,
/// or with no paper in it to measure.
pub fn flatten(rgb: &mut [u8], width: u32, height: u32) -> Option<Tone> {
    let (width, height) = (width as usize, height as usize);
    if rgb.len() < width * height * 3 {
        return None;
    }
    let long = width.max(height);
    if long < TILES * 4 {
        return None;
    }
    let tile = long.div_ceil(TILES).max(4);
    let (across, down) = (width.div_ceil(tile), height.div_ceil(tile));
    if across < 2 || down < 2 {
        return None;
    }

    let tone = tone_of(rgb, width, height);

    // The paper's level in each tile, one field per channel.
    let mut fields: [Vec<f64>; 3] = std::array::from_fn(|_| vec![0.0; across * down]);
    for ty in 0..down {
        for tx in 0..across {
            let x1 = ((tx + 1) * tile).min(width);
            let y1 = ((ty + 1) * tile).min(height);
            let mut counts = [[0u32; 256]; 3];
            for y in ty * tile..y1 {
                let row = y * width * 3;
                for x in tx * tile..x1 {
                    let at = row + x * 3;
                    for channel in 0..3 {
                        counts[channel][rgb[at + channel] as usize] += 1;
                    }
                }
            }
            let pixels = ((x1 - tx * tile) * (y1 - ty * tile)) as u32;
            for channel in 0..3 {
                fields[channel][ty * across + tx] = high(&counts[channel], pixels, PAPER_AT);
            }
        }
    }

    // A tile with no paper in it is lifted to the page's own level rather than
    // trusted, and the whole field is then smoothed: the light is smooth even
    // where the measurements of it are not.
    for field in &mut fields {
        let floor = median(field) * DIMMEST;
        if floor <= 0.0 {
            return None;
        }
        for value in field.iter_mut() {
            *value = value.max(floor);
        }
        smooth(field, across, down, floor);
    }

    for y in 0..height {
        let row = y * width * 3;
        for x in 0..width {
            let at = row + x * 3;
            for channel in 0..3 {
                let light = sampled(&fields[channel], across, down, tile, x, y);
                let lit = rgb[at + channel] as f64 * PAPER_BECOMES / light;
                rgb[at + channel] = lit.clamp(0.0, 255.0) as u8;
            }
        }
    }

    // And now that the light is out of it, the ink is where it is everywhere.
    stretch(rgb);
    Some(tone)
}

/// The paper and the ink of a picture, as it is.
fn tone_of(rgb: &[u8], width: usize, height: usize) -> Tone {
    let mut counts = [0u32; 256];
    for at in (0..width * height * 3).step_by(3) {
        counts[luma(&rgb[at..at + 3]) as usize] += 1;
    }
    let pixels = (width * height) as u32;
    Tone {
        paper: high(&counts, pixels, 0.5).round().clamp(0.0, 255.0) as u8,
        ink: high(&counts, pixels, INK_AT).round().clamp(0.0, 255.0) as u8,
    }
}

/// Pulls the black point down to the ink, so what is left between ink and
/// paper is the whole of the range. Does nothing when the darkest thing on the
/// page is not dark enough to be ink: see [`INK_BELOW`].
fn stretch(rgb: &mut [u8]) {
    let mut counts = [0u32; 256];
    for at in (0..rgb.len() - rgb.len() % 3).step_by(3) {
        counts[luma(&rgb[at..at + 3]) as usize] += 1;
    }
    let pixels = (rgb.len() / 3) as u32;
    let black = high(&counts, pixels, INK_AT).min(PAPER_BECOMES * BLACK_AT_MOST);
    if PAPER_BECOMES - black < INK_BELOW {
        return;
    }
    // The paper goes to 255 here, and only here: the tiles put it at
    // PAPER_BECOMES so that this step has somewhere to move it from, and a
    // scan of a letter has white paper on it, not grey paper with grain.
    let scale = 255.0 / (PAPER_BECOMES - black);
    let mut curve = [0u8; 256];
    for (value, out) in curve.iter_mut().enumerate() {
        *out = (((value as f64) - black) * scale).clamp(0.0, 255.0) as u8;
    }
    for byte in rgb.iter_mut() {
        *byte = curve[*byte as usize];
    }
}

/// Rec. 601 luma of one pixel, which is what the JPEG is stored in anyway.
fn luma(pixel: &[u8]) -> u8 {
    ((77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8) as u8
}

/// The value `fraction` of the way up a histogram of `total` samples.
fn high(counts: &[u32; 256], total: u32, fraction: f64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let wanted = (total as f64 * fraction) as u32;
    let mut seen = 0u32;
    for (value, &count) in counts.iter().enumerate() {
        seen += count;
        if seen > wanted {
            return value as f64;
        }
    }
    255.0
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}

/// 3×3 averaging over the grid, a few times.
///
/// Past the edges the field is continued along its own slope rather than
/// repeated, which leaves a steady gradient exactly where it was. Repeating
/// the edge instead averages the dim end of the page with the brighter tiles
/// inside it, so the field there comes out brighter than the paper is: a lamp
/// at one side of an A4 sheet still left twenty levels between the two edges
/// after it had supposedly been taken out.
fn smooth(field: &mut [f64], across: usize, down: usize, floor: f64) {
    let mut next = field.to_vec();
    for _ in 0..SMOOTHINGS {
        for y in 0..down {
            for x in 0..across {
                let mut sum = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        sum += beyond(field, across, down, x as i32 + dx, y as i32 + dy);
                    }
                }
                next[y * across + x] = (sum / 9.0).max(floor);
            }
        }
        field.copy_from_slice(&next);
    }
}

/// The field at a tile, continuing its slope past the grid's edges.
fn beyond(field: &[f64], across: usize, down: usize, x: i32, y: i32) -> f64 {
    let along = |row: usize, x: i32| {
        let at = |x: usize| field[row * across + x];
        if x < 0 {
            2.0 * at(0) - at(1)
        } else if x as usize >= across {
            2.0 * at(across - 1) - at(across - 2)
        } else {
            at(x as usize)
        }
    };
    if y < 0 {
        2.0 * along(0, x) - along(1, x)
    } else if y as usize >= down {
        2.0 * along(down - 1, x) - along(down - 2, x)
    } else {
        along(y as usize, x)
    }
}

/// The field at a pixel: bilinear between the four tile centres around it,
/// so the correction has no steps in it where the tiles meet.
fn sampled(field: &[f64], across: usize, down: usize, tile: usize, x: usize, y: usize) -> f64 {
    let half = tile as f64 / 2.0;
    let fx = ((x as f64 - half) / tile as f64).clamp(0.0, across as f64 - 1.0);
    let fy = ((y as f64 - half) / tile as f64).clamp(0.0, down as f64 - 1.0);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(across - 1), (y0 + 1).min(down - 1));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    let top = field[y0 * across + x0] * (1.0 - tx) + field[y0 * across + x1] * tx;
    let bottom = field[y1 * across + x0] * (1.0 - tx) + field[y1 * across + x1] * tx;
    top * (1.0 - ty) + bottom * ty
}
