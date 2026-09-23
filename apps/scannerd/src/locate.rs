//! Finding the page in a picture of the whole view, to suggest the crop.
//!
//! Dragging a box over a small picture on a phone is fiddly, and the crop
//! matters: outside it, people and shadows are ignored, and inside it the page
//! gets the camera's pixels. So setup asks for a page to be put where letters
//! will lie, and this finds it — paper being the largest bright thing in view.
//!
//! Brightness rather than colour or edges: the frames are greyscale already,
//! and paper under a lamp or daylight is brighter than any table worth scanning
//! on. The threshold is chosen for each picture (Otsu's method) rather than
//! fixed, because a dim room's paper is a bright room's table.

use crate::straighten::Corners;

/// An area of the view as fractions: left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    /// As `--roi` and the settings take it.
    pub fn to_roi(&self) -> String {
        format!(
            "{:.3},{:.3},{:.3},{:.3}",
            self.x, self.y, self.width, self.height
        )
    }
}

/// The least share of the view a page can cover and count. Below it, a
/// bright thing is a reflection, a mug or a lamp.
const MIN_SHARE: f32 = 0.02;

/// Room left around the page, as a share of its size on each side. It was
/// 12%, so that a letter put down a little off would still be inside — and
/// every photograph then had a band of table round it, which is what setup
/// was meant to get rid of. Letters that land off the mark are trimmed after
/// straightening instead ([`page_bounds`]); this is only for the edge.
const MARGIN: f32 = 0.03;

/// The largest bright area in a greyscale picture, with room around it —
/// `None` when nothing in view looks like a page.
pub fn find_page(luma: &[u8], width: usize, height: usize) -> Option<Area> {
    let region = largest_page(luma, width, height)?;
    let (region_width, region_height) = (
        (region.max_x - region.min_x + 1) as f32,
        (region.max_y - region.min_y + 1) as f32,
    );
    let (margin_x, margin_y) = (region_width * MARGIN, region_height * MARGIN);
    let left = (region.min_x as f32 - margin_x).max(0.0);
    let top = (region.min_y as f32 - margin_y).max(0.0);
    let right = (region.max_x as f32 + 1.0 + margin_x).min(width as f32);
    let bottom = (region.max_y as f32 + 1.0 + margin_y).min(height as f32);
    Some(Area {
        x: left / width as f32,
        y: top / height as f32,
        width: (right - left) / width as f32,
        height: (bottom - top) / height as f32,
    })
}

/// The four corners of the page, for a camera that looks at the table at an
/// angle, with the same room around it as [`find_page`] leaves — measured on
/// the table rather than in the picture, so the corners still mark a
/// rectangle there. `None` when nothing in view looks like a page, or its
/// corners cannot be told apart.
///
/// A corner is the page's point furthest in its diagonal direction: top left
/// is where `x + y` is least, top right where `x - y` is greatest. That holds
/// for a page lying roughly square to the camera, which is how setup asks for
/// it to be put down.
pub fn find_page_corners(luma: &[u8], width: usize, height: usize) -> Option<Corners> {
    let region = largest_page(luma, width, height)?;
    // Pixel centres, as fractions of the picture.
    let at = |(x, y): (usize, usize)| {
        (
            (x as f64 + 0.5) / width as f64,
            (y as f64 + 0.5) / height as f64,
        )
    };
    let page = Corners::new(region.corners.map(|corner| at(corner.1))).ok()?;
    Some(page.with_margin(MARGIN as f64))
}

/// The page's own corners in a greyscale picture, as fractions, with no room
/// round them, and the share of the picture the page covers. For finding the
/// letter in a photograph already straightened by setup's corners, which lies
/// a little off them — see [`crate::straighten::straighten_trimmed`].
pub fn page_corners(luma: &[u8], width: usize, height: usize) -> Option<(Corners, f64)> {
    let region = largest_page(luma, width, height)?;
    let at = |(x, y): (usize, usize)| {
        (
            (x as f64 + 0.5) / width as f64,
            (y as f64 + 0.5) / height as f64,
        )
    };
    let corners = Corners::new(region.corners.map(|corner| at(corner.1))).ok()?;
    Some((corners, region.count as f64 / (width * height) as f64))
}

/// The largest bright region, if it could be a page.
fn largest_page(luma: &[u8], width: usize, height: usize) -> Option<Region> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 || luma.len() < pixels {
        return None;
    }
    let luma = &luma[..pixels];
    let threshold = otsu(luma);

    // Connected bright regions, four-connected. The text on a page leaves
    // dark holes in its region but does not split it: the margins join up
    // around every line.
    let mut seen = vec![false; pixels];
    let mut stack = Vec::new();
    let mut best: Option<Region> = None;
    for start in 0..pixels {
        if seen[start] || luma[start] <= threshold {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut region = Region::at(start % width, start / width);
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            region.add(x, y);
            let mut visit = |next: usize| {
                if !seen[next] && luma[next] > threshold {
                    seen[next] = true;
                    stack.push(next);
                }
            };
            if x > 0 {
                visit(index - 1);
            }
            if x + 1 < width {
                visit(index + 1);
            }
            if y > 0 {
                visit(index - width);
            }
            if y + 1 < height {
                visit(index + width);
            }
        }
        if best.is_none_or(|best| region.count > best.count) {
            best = Some(region);
        }
    }

    let region = best?;
    if (region.count as f32) < MIN_SHARE * pixels as f32 {
        return None;
    }
    // Brightness across nearly the whole view is not a page on a table: it is
    // a white table, or a picture washed out by the exposure.
    let (region_width, region_height) = (
        (region.max_x - region.min_x + 1) as f32,
        (region.max_y - region.min_y + 1) as f32,
    );
    if region_width > 0.95 * width as f32 && region_height > 0.95 * height as f32 {
        return None;
    }
    Some(region)
}

#[derive(Debug, Clone, Copy)]
struct Region {
    count: usize,
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    /// The furthest point in each diagonal direction, with how far it is:
    /// top left, top right, bottom right, bottom left.
    corners: [(i64, (usize, usize)); 4],
}

impl Region {
    fn at(x: usize, y: usize) -> Self {
        Self {
            count: 0,
            min_x: x,
            min_y: y,
            max_x: x,
            max_y: y,
            corners: [(i64::MIN, (x, y)); 4],
        }
    }

    fn add(&mut self, x: usize, y: usize) {
        self.count += 1;
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
        let (sx, sy) = (x as i64, y as i64);
        let reach = [-sx - sy, sx - sy, sx + sy, sy - sx];
        for (corner, reach) in self.corners.iter_mut().zip(reach) {
            if reach > corner.0 {
                *corner = (reach, (x, y));
            }
        }
    }
}

/// The grey level that best splits a picture into dark and bright: the one
/// maximising the variance between the two groups.
fn otsu(pixels: &[u8]) -> u8 {
    let mut histogram = [0u64; 256];
    for &pixel in pixels {
        histogram[pixel as usize] += 1;
    }
    let total = pixels.len() as f64;
    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(value, &count)| value as f64 * count as f64)
        .sum();

    let (mut weight_below, mut sum_below) = (0.0, 0.0);
    let (mut best, mut best_variance) = (0u8, -1.0);
    for (value, &count) in histogram.iter().enumerate() {
        weight_below += count as f64;
        if weight_below == 0.0 {
            continue;
        }
        let weight_above = total - weight_below;
        if weight_above == 0.0 {
            break;
        }
        sum_below += value as f64 * count as f64;
        let mean_below = sum_below / weight_below;
        let mean_above = (sum_all - sum_below) / weight_above;
        let variance = weight_below * weight_above * (mean_below - mean_above).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best = value as u8;
        }
    }
    best
}
