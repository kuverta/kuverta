//! A signature cut out of its photograph.
//!
//! What a person has is a photo or a scan of their signature: ink on paper
//! that is grey rather than white, lit from one side, with a shadow and the
//! grain of the paper in it. Put on a page as it is, that is a grey patch
//! with a signature in it. What a page wants is the ink alone, with the
//! paper transparent, so the strokes sit on the line they are signed on.
//!
//! So a picture is cleaned when it is chosen, in four steps:
//!
//! 1. **The paper's brightness is measured** block by block — the bright end
//!    of each block is the paper there — and interpolated, so that a shadow
//!    on one side or a lamp on the other is part of the measurement rather
//!    than of the ink.
//! 2. **Each pixel is read against the paper under it**: how much darker it
//!    is than the paper there is how much ink it holds. Otsu's threshold on
//!    those darknesses separates ink from grain, and the alpha ramps across
//!    it, so edges stay soft.
//! 3. **Specks go**: anything inked that is too small to be part of a stroke
//!    — grain, a hair, a dot of dust — is dropped, by connected component.
//! 4. **The picture is cropped** to the strokes, with a little room.
//!
//! A picture that is already cut out — a PNG with a transparent background
//! — is only cropped. Everything here is bounded by the picture's size,
//! which was checked before.

use image::{DynamicImage, Rgba, RgbaImage};

/// Pictures wider than this are shrunk first: a signature wants no more,
/// and the cleaning is done pixel by pixel.
const MOST_WIDTH: u32 = 1600;
/// The least the paper is taken to be, so that a black photograph does not
/// read as all ink.
const LEAST_PAPER: f32 = 40.0;
/// The threshold used when the darknesses are not bimodal — a blank or a
/// near-blank picture.
const FALLBACK_THRESHOLD: f32 = 0.3;

/// The cleaned signature: ink on a transparent background, cropped.
pub fn clean(decoded: &DynamicImage) -> RgbaImage {
    let mut rgba = decoded.to_rgba8();
    if rgba.width() > MOST_WIDTH {
        let height = (u64::from(rgba.height()) * u64::from(MOST_WIDTH) / u64::from(rgba.width()))
            .max(1) as u32;
        rgba = image::imageops::resize(
            &rgba,
            MOST_WIDTH,
            height,
            image::imageops::FilterType::Triangle,
        );
    }
    if already_cut_out(&rgba) {
        return crop(&rgba);
    }
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let luma: Vec<f32> = rgba
        .pixels()
        .map(|p| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
        .collect();
    let paper = paper_brightness(&luma, w, h);
    let darkness: Vec<f32> = luma
        .iter()
        .zip(&paper)
        .map(|(l, p)| (1.0 - l / p).clamp(0.0, 1.0))
        .collect();
    let threshold = otsu(&darkness);
    let low = threshold * 0.6;
    let high = threshold * 1.4;
    let mut alpha: Vec<u8> = darkness
        .iter()
        .map(|d| (((d - low) / (high - low)).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    drop_specks(&mut alpha, w, h, (w * h / 20_000).max(20));

    let mut out = RgbaImage::new(w as u32, h as u32);
    for (at, pixel) in out.pixels_mut().enumerate() {
        let a = alpha[at];
        if a == 0 {
            *pixel = Rgba([0, 0, 0, 0]);
            continue;
        }
        // The ink's own colour, read against the paper: a blue pen stays
        // blue, and the paper's cast comes off it.
        let source = rgba.get_pixel((at % w) as u32, (at / w) as u32);
        let gain = 255.0 / paper[at];
        let ink = |c: u8| (f32::from(c) * gain).clamp(0.0, 255.0) as u8;
        *pixel = Rgba([ink(source[0]), ink(source[1]), ink(source[2]), a]);
    }
    crop(&out)
}

/// Whether the picture has a transparent background already: a tenth of it
/// or more is see-through.
fn already_cut_out(rgba: &RgbaImage) -> bool {
    let clear = rgba.pixels().filter(|p| p[3] < 128).count();
    clear * 10 >= rgba.pixels().len()
}

/// The paper's brightness under every pixel: the bright end of each block,
/// interpolated between block centres.
fn paper_brightness(luma: &[f32], w: usize, h: usize) -> Vec<f32> {
    let block = (w.min(h) / 12).clamp(16, 64);
    let cols = w.div_ceil(block);
    let rows = h.div_ceil(block);
    let mut grid = vec![0.0f32; cols * rows];
    let mut values = Vec::with_capacity(block * block);
    for row in 0..rows {
        for col in 0..cols {
            values.clear();
            for y in (row * block)..((row + 1) * block).min(h) {
                for x in (col * block)..((col + 1) * block).min(w) {
                    values.push(luma[y * w + x]);
                }
            }
            values.sort_by(|a, b| a.total_cmp(b));
            let at = (values.len() * 85 / 100).min(values.len() - 1);
            grid[row * cols + col] = values[at].max(LEAST_PAPER);
        }
    }
    let half = block as f32 / 2.0;
    let sample = |col: isize, row: isize| {
        let col = col.clamp(0, cols as isize - 1) as usize;
        let row = row.clamp(0, rows as isize - 1) as usize;
        grid[row * cols + col]
    };
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let gy = (y as f32 - half) / block as f32;
        let row = gy.floor() as isize;
        let ty = gy - gy.floor();
        for x in 0..w {
            let gx = (x as f32 - half) / block as f32;
            let col = gx.floor() as isize;
            let tx = gx - gx.floor();
            let top = sample(col, row) * (1.0 - tx) + sample(col + 1, row) * tx;
            let bottom = sample(col, row + 1) * (1.0 - tx) + sample(col + 1, row + 1) * tx;
            out[y * w + x] = top * (1.0 - ty) + bottom * ty;
        }
    }
    out
}

/// Otsu's threshold between the two populations of darkness — paper and
/// ink — or a fixed one when there is only one.
fn otsu(darkness: &[f32]) -> f32 {
    let mut histogram = [0u64; 256];
    for d in darkness {
        histogram[(d * 255.0).round() as usize] += 1;
    }
    let total = darkness.len() as f64;
    let sum: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, n)| i as f64 * *n as f64)
        .sum();
    let (mut weight_b, mut sum_b, mut best, mut at) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    for (i, n) in histogram.iter().enumerate() {
        weight_b += *n as f64;
        if weight_b == 0.0 {
            continue;
        }
        let weight_f = total - weight_b;
        if weight_f == 0.0 {
            break;
        }
        sum_b += i as f64 * *n as f64;
        let mean_b = sum_b / weight_b;
        let mean_f = (sum - sum_b) / weight_f;
        let between = weight_b * weight_f * (mean_b - mean_f).powi(2);
        if between > best {
            best = between;
            at = i;
        }
    }
    let threshold = at as f32 / 255.0;
    if threshold < 0.08 {
        FALLBACK_THRESHOLD
    } else {
        threshold
    }
}

/// Clears inked areas smaller than `least` pixels.
fn drop_specks(alpha: &mut [u8], w: usize, h: usize, least: usize) {
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut component = Vec::new();
    for start in 0..w * h {
        if alpha[start] == 0 || seen[start] {
            continue;
        }
        component.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(at) = stack.pop() {
            component.push(at);
            let (x, y) = (at % w, at / w);
            let neighbours = [
                (x > 0).then(|| at - 1),
                (x + 1 < w).then(|| at + 1),
                (y > 0).then(|| at - w),
                (y + 1 < h).then(|| at + w),
            ];
            for next in neighbours.into_iter().flatten() {
                if alpha[next] != 0 && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        if component.len() < least {
            for at in &component {
                alpha[*at] = 0;
            }
        }
    }
}

/// The picture cropped to where it is not transparent, with a margin.
fn crop(rgba: &RgbaImage) -> RgbaImage {
    let (w, h) = (rgba.width(), rgba.height());
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in rgba.enumerate_pixels() {
        if p[3] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 {
        return rgba.clone();
    }
    let margin = (w.max(h) * 3 / 100).max(4);
    let x0 = x0.saturating_sub(margin);
    let y0 = y0.saturating_sub(margin);
    let x1 = (x1 + margin + 1).min(w);
    let y1 = (y1 + margin + 1).min(h);
    image::imageops::crop_imm(rgba, x0, y0, x1 - x0, y1 - y0).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A photograph of a signature: paper lit from the left, grain, a thick
    /// stroke across, and a speck of dust.
    fn photo() -> DynamicImage {
        let (w, h) = (400u32, 160u32);
        let mut image = RgbaImage::new(w, h);
        for (x, y, p) in image.enumerate_pixels_mut() {
            let paper = 200.0 - 60.0 * (x as f32 / w as f32) + ((x * 7 + y * 13) % 11) as f32;
            let v = paper as u8;
            *p = Rgba([v, v, v, 255]);
        }
        for x in 40..360u32 {
            let yc = 80.0 + 25.0 * ((x as f32) / 40.0).sin();
            for dy in -6..=6i32 {
                let y = (yc as i32 + dy) as u32;
                let v = 30 + (x % 5) as u8;
                image.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        image.put_pixel(380, 20, Rgba([20, 20, 20, 255]));
        image.put_pixel(381, 20, Rgba([20, 20, 20, 255]));
        DynamicImage::ImageRgba8(image)
    }

    #[test]
    fn the_paper_becomes_transparent_and_the_ink_stays() {
        let cleaned = clean(&photo());
        assert!(
            cleaned.width() < 400 && cleaned.height() < 160,
            "{}x{}",
            cleaned.width(),
            cleaned.height()
        );
        let total = cleaned.pixels().len();
        let clear = cleaned.pixels().filter(|p| p[3] == 0).count();
        let inked = cleaned.pixels().filter(|p| p[3] == 255).count();
        assert!(clear * 2 > total, "clear {clear} of {total}");
        assert!(inked > 2_000, "inked {inked}");
        // The ink is dark on both the bright side and the dim side.
        let dark = cleaned.pixels().filter(|p| p[3] == 255).all(|p| p[0] < 90);
        assert!(dark);
        // The stroke's two ends are kept, the speck is gone: the crop ends
        // at the stroke and never reaches the speck's row.
        assert!(cleaned.height() < 100, "{}", cleaned.height());
    }

    #[test]
    fn a_picture_already_cut_out_is_only_cropped() {
        let mut image = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 0]));
        image.put_pixel(50, 50, Rgba([10, 20, 200, 255]));
        let cleaned = clean(&DynamicImage::ImageRgba8(image));
        assert!(
            cleaned.width() <= 11 && cleaned.height() <= 11,
            "{}x{}",
            cleaned.width(),
            cleaned.height()
        );
        assert!(cleaned.pixels().any(|p| *p == Rgba([10, 20, 200, 255])));
    }

    #[test]
    fn a_blank_page_yields_nothing_inked_rather_than_a_grey_patch() {
        let image = RgbaImage::from_pixel(200, 100, Rgba([180, 180, 180, 255]));
        let cleaned = clean(&DynamicImage::ImageRgba8(image));
        assert!(cleaned.pixels().all(|p| p[3] == 0));
    }

    #[test]
    fn specks_smaller_than_a_stroke_are_dropped() {
        let (w, h) = (20, 20);
        let mut alpha = vec![0u8; w * h];
        alpha[5 * w + 5] = 255;
        for x in 0..15 {
            alpha[10 * w + x] = 255;
        }
        drop_specks(&mut alpha, w, h, 5);
        assert_eq!(alpha[5 * w + 5], 0);
        assert_eq!(alpha[10 * w + 3], 255);
    }
}
