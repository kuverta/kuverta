//! Straightening a page photographed at an angle.
//!
//! A camera that cannot be mounted straight above the table sees a page as a
//! trapezium: the far edge shorter than the near one. A crop cannot fix that,
//! so setup marks the four corners of a page lying where letters go, and every
//! photograph is warped so that those corners become a rectangle's.
//!
//! This is the one place scannerd decodes a photograph. Paperless deskews a
//! page that lies a little crooked, but it does not undo perspective, and its
//! OCR reads a trapezium's far lines at a smaller size than its near ones.

use anyhow::{bail, Context, Result};
use image::RgbImage;
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

/// Four points as fractions of a picture, in the order top left, top right,
/// bottom right, bottom left — clockwise as the picture is looked at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners(pub [(f64, f64); 4]);

/// The smallest share of the view the corners may enclose.
const MIN_AREA: f64 = 0.01;

impl Corners {
    /// The whole picture: straightening with these changes nothing but the
    /// turn.
    pub const WHOLE: Corners = Corners([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);

    /// The same corners, starting `quarters` places further round, so the
    /// straightened page comes out turned that many quarter turns clockwise:
    /// what was its left edge becomes its top.
    pub fn turned(&self, quarters: u16) -> Corners {
        let k = (quarters % 4) as usize;
        Corners(std::array::from_fn(|i| self.0[(i + 4 - k) % 4]))
    }

    /// `x1,y1,x2,y2,x3,y3,x4,y4`, as the settings keep them.
    pub fn parse(text: &str) -> Result<Self> {
        let numbers: Vec<f64> = text
            .split(',')
            .map(|part| part.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| anyhow::anyhow!("corners are eight numbers: x,y for each of four"))?;
        let [x1, y1, x2, y2, x3, y3, x4, y4] = numbers[..] else {
            bail!("corners are eight numbers: x,y for each of four");
        };
        Self::new([(x1, y1), (x2, y2), (x3, y3), (x4, y4)])
    }

    /// Four points that are inside the picture and in order.
    pub fn new(points: [(f64, f64); 4]) -> Result<Self> {
        let corners = Self(points);
        corners.check()?;
        Ok(corners)
    }

    fn check(&self) -> Result<()> {
        if self
            .0
            .iter()
            .any(|&(x, y)| !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y))
        {
            bail!("each corner is a fraction of the picture, between 0 and 1");
        }
        // Clockwise on screen, where y grows downwards, is a positive turn at
        // every corner. A corner dragged past another turns one of them round.
        let turns = (0..4).map(|i| {
            let (a, b, c) = (self.0[i], self.0[(i + 1) % 4], self.0[(i + 2) % 4]);
            (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)
        });
        if turns.into_iter().any(|turn| turn <= 0.0) {
            bail!("the corners must be top left, top right, bottom right and bottom left, with no edges crossing");
        }
        if self.area() < MIN_AREA {
            bail!("the corners enclose too little of the picture");
        }
        Ok(())
    }

    fn area(&self) -> f64 {
        let twice: f64 = (0..4)
            .map(|i| {
                let (a, b) = (self.0[i], self.0[(i + 1) % 4]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        twice.abs() / 2.0
    }

    pub fn to_setting(&self) -> String {
        self.0
            .iter()
            .map(|(x, y)| format!("{x:.3},{y:.3}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The crop that holds all four corners, as `x,y,w,h`. The camera still
    /// crops on the sensor, so detection watches only this and the photograph
    /// spends its pixels on it.
    pub fn crop(&self) -> String {
        let xs = self.0.map(|(x, _)| x);
        let ys = self.0.map(|(_, y)| y);
        let lowest = |values: [f64; 4]| values.into_iter().fold(1.0, f64::min);
        let highest = |values: [f64; 4]| values.into_iter().fold(0.0, f64::max);
        // Outwards to the next thousandth, which is what the crop is written to,
        // so no corner falls outside it.
        let (left, top) = (floor3(lowest(xs)), floor3(lowest(ys)));
        let (right, bottom) = (ceil3(highest(xs)).min(1.0), ceil3(highest(ys)).min(1.0));
        format!("{left:.3},{top:.3},{:.3},{:.3}", right - left, bottom - top)
    }

    /// The same corners as fractions of the crop `x,y,w,h` — of the
    /// photograph the camera takes with it.
    pub fn within(&self, crop: &str) -> Option<Corners> {
        let parts: Vec<f64> = crop
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [x, y, w, h] = parts[..] else {
            return None;
        };
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some(Corners(self.0.map(|(cx, cy)| ((cx - x) / w, (cy - y) / h))))
    }

    /// The corners moved outwards by `margin` of the page's size on each
    /// side, as they would be on the table — so a letter put down a little
    /// off the marked page is still inside. The margin shrinks as needed to
    /// keep every corner in the picture.
    pub fn with_margin(&self, margin: f64) -> Corners {
        let map = Homography::square_to(self);
        let mut margin = margin;
        while margin > 0.001 {
            let grown = Corners([
                map.apply(-margin, -margin),
                map.apply(1.0 + margin, -margin),
                map.apply(1.0 + margin, 1.0 + margin),
                map.apply(-margin, 1.0 + margin),
            ]);
            let inside = grown
                .0
                .iter()
                .all(|&(x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
            if inside && grown.check().is_ok() {
                return grown;
            }
            margin *= 0.8;
        }
        *self
    }
}

fn floor3(value: f64) -> f64 {
    ((value * 1000.0 + 1e-6).floor() / 1000.0).max(0.0)
}

fn ceil3(value: f64) -> f64 {
    (value * 1000.0 - 1e-6).ceil() / 1000.0
}

/// A perspective map from the unit square to four points: `(0,0)` to the
/// first, `(1,0)` to the second, `(1,1)` to the third, `(0,1)` to the fourth.
/// Heckbert's closed form, which needs no matrix solver.
#[derive(Debug, Clone, Copy)]
pub struct Homography {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
    g: f64,
    h: f64,
}

impl Homography {
    pub fn square_to(corners: &Corners) -> Self {
        let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = corners.0;
        let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
        let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
        let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
            // A parallelogram: no perspective at all.
            (0.0, 0.0)
        } else {
            let den = dx1 * dy2 - dx2 * dy1;
            ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
        };
        Self {
            a: x1 - x0 + g * x1,
            b: x3 - x0 + h * x3,
            c: x0,
            d: y1 - y0 + g * y1,
            e: y3 - y0 + h * y3,
            f: y0,
            g,
            h,
        }
    }

    pub fn apply(&self, u: f64, v: f64) -> (f64, f64) {
        let w = self.g * u + self.h * v + 1.0;
        (
            (self.a * u + self.b * v + self.c) / w,
            (self.d * u + self.e * v + self.f) / w,
        )
    }
}

/// JPEG quality of a straightened page. High, because the OCR reads it and
/// it has already been compressed once.
const QUALITY: u8 = 92;

/// A photograph, warped so that `corners` — fractions of it — become the
/// corners of the whole result. The result is as wide as the average of the
/// top and bottom edges and as tall as the average of the sides, so a page
/// keeps roughly the proportions it has on the table.
pub fn straighten(jpeg: &[u8], corners: &Corners) -> Result<Vec<u8>> {
    let source = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .context("could not read the photograph")?
        .into_rgb8();
    let pixels = Corners(
        corners
            .0
            .map(|(x, y)| (x * source.width() as f64, y * source.height() as f64)),
    );
    let length = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let [tl, tr, br, bl] = pixels.0;
    let width = ((length(tl, tr) + length(bl, br)) / 2.0).round() as u32;
    let height = ((length(tl, bl) + length(tr, br)) / 2.0).round() as u32;
    if width < 16 || height < 16 {
        bail!("the corners enclose too little of the photograph ({width}×{height})");
    }

    let straight = warp(&source, &Homography::square_to(&pixels), width, height);

    let (w, h) = (
        u16::try_from(width).context("the straightened page is too wide")?,
        u16::try_from(height).context("the straightened page is too tall")?,
    );
    let mut out = Vec::with_capacity(jpeg.len());
    let mut encoder = Encoder::new(&mut out, QUALITY);
    // Colour at half resolution, as the camera wrote it: at this quality the
    // encoder would otherwise keep it whole, which on a Pi Zero doubles the
    // time and adds nothing the photograph had.
    encoder.set_sampling_factor(SamplingFactor::R_4_2_0);
    encoder
        .encode(&straight, w, h, ColorType::Rgb)
        .context("could not write the straightened photograph")?;
    Ok(out)
}

/// Samples `source` through `map` for every pixel of a `width`×`height`
/// result, bilinearly, as packed RGB. `map` takes the unit square to source
/// pixels.
///
/// The weights are integers out of 256: on a Pi Zero, twelve floating-point
/// multiplications a pixel were a third of the time straightening took.
fn warp(source: &RgbImage, map: &Homography, width: u32, height: u32) -> Vec<u8> {
    let (sw, sh) = (source.width() as usize, source.height() as usize);
    let raw = source.as_raw();
    let mut out = vec![0u8; width as usize * height as usize * 3];
    let (max_x, max_y) = ((sw - 1) as f64, (sh - 1) as f64);

    for (row, line) in out.chunks_exact_mut(width as usize * 3).enumerate() {
        let v = (row as f64 + 0.5) / height as f64;
        // Along a row the three sums are linear in u: step them rather than
        // multiply for every pixel.
        let du = 1.0 / width as f64;
        let u0 = du / 2.0;
        let (mut nx, mut ny, mut w) = (
            map.a * u0 + map.b * v + map.c,
            map.d * u0 + map.e * v + map.f,
            map.g * u0 + map.h * v + 1.0,
        );
        let (step_x, step_y, step_w) = (map.a * du, map.d * du, map.g * du);

        for pixel in line.chunks_exact_mut(3) {
            // Pixel centres sit at half-integers.
            let x = (nx / w - 0.5).clamp(0.0, max_x);
            let y = (ny / w - 0.5).clamp(0.0, max_y);
            nx += step_x;
            ny += step_y;
            w += step_w;

            let (x0, y0) = (x as usize, y as usize);
            let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
            let fx = ((x - x0 as f64) * 256.0) as u32;
            let fy = ((y - y0 as f64) * 256.0) as u32;
            let (gx, gy) = (256 - fx, 256 - fy);
            let (i00, i10) = ((y0 * sw + x0) * 3, (y0 * sw + x1) * 3);
            let (i01, i11) = ((y1 * sw + x0) * 3, (y1 * sw + x1) * 3);
            for channel in 0..3 {
                let top = raw[i00 + channel] as u32 * gx + raw[i10 + channel] as u32 * fx;
                let bottom = raw[i01 + channel] as u32 * gx + raw[i11 + channel] as u32 * fx;
                pixel[channel] = ((top * gy + bottom * fy + 32768) >> 16) as u8;
            }
        }
    }
    out
}
