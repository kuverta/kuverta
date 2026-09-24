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

/// Room left round the marked corners in the crop, as a share of what they
/// enclose. A letter is never put down in exactly the same place twice, and
/// what is outside the crop was never photographed. See [`Corners::crop`].
const CROP_MARGIN: f64 = 0.08;

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

    /// The share of the picture the corners enclose.
    pub fn area(&self) -> f64 {
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

    /// The crop that holds all four corners with room round them, as
    /// `x,y,w,h`. The camera crops on the sensor, so this decides what is in
    /// the photograph at all.
    ///
    /// The room is the point. Cropped to the marks exactly, a letter put down
    /// a little further left — or a sheet bigger than the one the rig was set
    /// up with — is cut by the sensor before any of this code runs, and no
    /// amount of straightening afterwards can find what was never
    /// photographed. That took the letterhead off letters, which is the one
    /// part of a page the sender is read from. The crop costs nothing to
    /// widen: [`crate::camera::Camera::capture_size`] takes sensor pixels one
    /// for one, so a wider crop is a bigger photograph rather than a coarser
    /// one, and the page keeps every pixel it had.
    pub fn crop(&self) -> String {
        let xs = self.0.map(|(x, _)| x);
        let ys = self.0.map(|(_, y)| y);
        let lowest = |values: [f64; 4]| values.into_iter().fold(1.0, f64::min);
        let highest = |values: [f64; 4]| values.into_iter().fold(0.0, f64::max);
        let (left, top) = (lowest(xs), lowest(ys));
        let (right, bottom) = (highest(xs), highest(ys));
        let (room_x, room_y) = ((right - left) * CROP_MARGIN, (bottom - top) * CROP_MARGIN);
        // Outwards to the next thousandth, which is what the crop is written
        // to, so no corner falls outside it.
        let (left, top) = (floor3(left - room_x), floor3(top - room_y));
        let (right, bottom) = (
            ceil3(right + room_x).min(1.0),
            ceil3(bottom + room_y).min(1.0),
        );
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

    /// The smallest quad in these corners' own perspective that holds both
    /// them and `other`.
    ///
    /// Taken corner by corner in the marked quad's own coordinates, so the
    /// result keeps its shape and its angle to the camera and can only ever
    /// be bigger. That matters: the marks and a page found by its brightness
    /// are each right about something. The marks are where the table is, and
    /// are exact; the found page is where this letter actually lies, and its
    /// edges are as good as the light on them. Taking whichever reaches
    /// further on each side keeps a letter that overhangs the marks without
    /// trusting a dim edge over a drawn one.
    pub fn around(&self, other: &Corners) -> Option<Corners> {
        let map = Homography::square_to(self);
        let back = map.invert()?;
        let (mut left, mut top, mut right, mut bottom) = (0.0_f64, 0.0_f64, 1.0_f64, 1.0_f64);
        for &(x, y) in other.0.iter() {
            let (u, v) = back.apply(x, y);
            left = left.min(u);
            top = top.min(v);
            right = right.max(u);
            bottom = bottom.max(v);
        }
        let grown = Corners([
            map.apply(left, top),
            map.apply(right, top),
            map.apply(right, bottom),
            map.apply(left, bottom),
        ]);
        // A corner that has left the picture is pulled back to its edge; if
        // that bends the quad out of shape, the marks stand as they are.
        let grown = Corners(grown.0.map(|(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))));
        grown.check().ok().map(|()| grown)
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

    /// The map the other way, when there is one: from the four points back to
    /// the unit square.
    pub fn invert(&self) -> Option<Self> {
        let (a, b, c) = (self.a, self.b, self.c);
        let (d, e, f) = (self.d, self.e, self.f);
        let (g, h) = (self.g, self.h);
        // The adjugate of [[a,b,c],[d,e,f],[g,h,1]], divided through by its
        // last entry so the result is in the same form.
        let last = a * e - b * d;
        if last.abs() < 1e-12 {
            return None;
        }
        Some(Self {
            a: (e - f * h) / last,
            b: (c * h - b) / last,
            c: (b * f - c * e) / last,
            d: (f * g - d) / last,
            e: (a - c * g) / last,
            f: (c * d - a * f) / last,
            g: (d * h - e * g) / last,
            h: (b * g - a * h) / last,
        })
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
    let (straight, width, height) = straighten_rgb(jpeg, corners)?;
    encode(&straight, width, height, jpeg.len())
}

/// The same, then trimmed to the page: a letter never lies exactly where the
/// corners were set, and the table it shows beside the page is cut away (see
/// [`trim_to_page`]). What a capture keeps.
///
/// The page that comes out is warped **once** from the photograph, even
/// though it takes two warps to find: the first is only looked at, and the
/// page's corners found in it are carried back into the photograph's own
/// pixels through the same homography. Resampling a resampling costs
/// sharpness, and sharpness is what the OCR reads.
pub fn straighten_trimmed(jpeg: &[u8], corners: &Corners) -> Result<Straightened> {
    let source = decode(jpeg)?;
    // The marks are where a page lay at setup, not where this one lies. When
    // this one reaches past them they would cut it away unrecoverably, so the
    // paper's own corners are added to theirs.
    let overhanging = paper_overhanging(&source, corners);
    let marked = match &overhanging {
        Some(paper) => corners
            .around(paper)
            .unwrap_or(*corners)
            .with_margin(TRIM_EDGE),
        None => *corners,
    };
    let quad = corners_in_pixels(&marked, source.width(), source.height());
    let (width, height) = warped_size(&quad)?;
    let first = Homography::square_to(&quad);
    let looking = warp(&source, &first, width, height);

    let page = page_within(&looking, width, height);
    // Looking again once the paper has already been found in the photograph
    // would be asking the same question of the same edges twice, and the
    // second answer can only be worse: this picture was made from the first
    // one, and where an edge was too dim to find it is dimmer here. It cut
    // real page off letters. The measurement is still wanted — it is what
    // tells an envelope from a sheet — so only the warp is skipped.
    let look_again = page.corners.filter(|_| overhanging.is_none());
    let (rgb, width, height) = match look_again {
        // Worth a second look: the page is somewhere inside what the corners
        // enclose, so the picture is warped onto the page itself — from the
        // photograph, not from the picture just made of it.
        Some(page) => {
            let page = Corners(page.0.map(|(x, y)| first.apply(x, y)));
            match warped_size(&page) {
                Ok((out_w, out_h)) => {
                    tracing::info!(from = ?(width, height), to = ?(out_w, out_h), "straightened onto the page itself");
                    (
                        warp(&source, &Homography::square_to(&page), out_w, out_h),
                        out_w,
                        out_h,
                    )
                }
                Err(_) => (looking, width, height),
            }
        }
        None => (looking, width, height),
    };
    Ok(Straightened {
        jpeg: encode(&rgb, width, height, jpeg.len())?,
        // Always against the marks, never against whatever was warped onto.
        // `covered` is how an envelope is told from a page, and the widening
        // above would otherwise shrink it for a page that overhangs — which
        // is a letter reported as the envelope it came in.
        covered: page
            .covered
            .map(|covered| (covered * marked.area() / corners.area()).min(1.0)),
    })
}

/// A photograph straightened and trimmed, and how much of the corners'
/// area the paper in it covered — which tells an envelope from a page.
pub struct Straightened {
    pub jpeg: Vec<u8>,
    /// The share of **the marked corners' area** the paper's own corners
    /// enclose: 0.8–1 for an A4 page on setup's A4 corners, about 0.4 for a
    /// DL envelope and 0.6 for a C5. `None` when no paper was found.
    ///
    /// Against the marks rather than against the picture that came out,
    /// because those are not always the same quad any more — see
    /// [`paper_overhanging`] — and this number only means anything if its
    /// yardstick never moves.
    pub covered: Option<f64>,
}

/// Paper covering less than this share of what a page covers is an
/// envelope: every envelope a page is folded into is smaller — DL about 0.4 of
/// an A4 page, C5 about 0.6. C4, the size of an unfolded page, is not told
/// apart.
pub const ENVELOPE: f64 = 0.72;
/// Less than this share of the corners' area is not believed to be paper.
const PAPER_AT_LEAST: f64 = 0.1;

/// Whether paper covering `covered` of the corners' area is an envelope,
/// where a page covers `page` of it. Measured against a page rather than the
/// corners, because the corners may be set generously round where letters
/// land — a page then covers only some of them, and would otherwise be taken
/// for an envelope.
pub fn is_envelope(covered: Option<f64>, page: f64) -> bool {
    covered.is_some_and(|covered| covered >= PAPER_AT_LEAST && covered < page * ENVELOPE)
}

struct Trimmed {
    rgb: Vec<u8>,
    width: u32,
    height: u32,
}

/// A bright area enclosing less than this share of the picture is not
/// trusted to be the paper it is cut down to: better table in the photograph
/// than text cut off.
const TRIM_MIN_SHARE: f64 = 0.2;
/// And it must fill this much of what its corners enclose: paper is one
/// bright sheet, with only its text dark.
const TRIM_MIN_FILL: f64 = 0.6;
/// Left round the page when trimming, as a share of its size.
const TRIM_EDGE: f64 = 0.015;
/// The long side of the small copy the page is looked for in.
const TRIM_SAMPLE: usize = 400;

/// Packed RGB straightened once more, onto the page it shows: setup's corners
/// mark where a page lay then, and the next letter lies a little off them —
/// further along, or a few degrees round — so its photograph has strips and
/// wedges of table beside it. The page's own corners are found in a small grey
/// copy ([`crate::locate::page_corners`]) and the picture warped onto them,
/// with a thin edge. Left as it is when no page is found, or when what is found
/// covers less than half of the picture: better table in a photograph than
/// text cut off.
pub fn trim_to_page(rgb: Vec<u8>, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let trimmed = trim_and_measure(rgb, width, height);
    (trimmed.rgb, trimmed.width, trimmed.height)
}

/// Where the paper is inside a straightened picture, and how much of the
/// picture it covers: the corners as fractions, for whoever warps onto them.
/// `None` corners mean "leave the picture as it is" — no paper found, too
/// little of it, or it already fills the picture.
struct Page {
    corners: Option<Corners>,
    covered: Option<f64>,
}

/// How far the paper may reach outside the marked corners before the marks
/// are taken as the wrong answer, as a share of the photograph.
const OVERHANG: f64 = 0.02;

/// How far a point lies outside a quad, as a share of the picture: zero when
/// it is inside. The corners are clockwise with `y` downwards ([`Corners::
/// check`]), so a point to the left of every edge is within them.
fn outside(quad: &Corners, (x, y): (f64, f64)) -> f64 {
    (0..4).fold(0.0_f64, |worst, i| {
        let (a, b) = (quad.0[i], quad.0[(i + 1) % 4]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let length = ex.hypot(ey);
        if length < 1e-9 {
            return worst;
        }
        worst.max(((x - a.0) * ey - (y - a.1) * ex) / length)
    })
}

/// The paper's own corners, when the marked ones are cutting it.
///
/// Setup's corners say where a page lay when the rig was set up. A letter put
/// down further left, or a bigger sheet, reaches past them — and everything
/// outside them is thrown away by the first warp, before the page is ever
/// looked for, so no amount of trimming afterwards can bring it back. That is
/// a letterhead cut off its own letter, which is both the worst part of the
/// page to lose and the part the sender is read from.
///
/// So the photograph itself is asked where the paper is, and its answer is
/// used **only** when the marks are demonstrably cutting it: something
/// page-shaped, page-sized against the marks, and reaching more than
/// [`OVERHANG`] outside them. Anything else leaves the marks alone, because
/// they are also what keeps the rest of the table out of the picture. Even
/// then the answer is not taken in place of the marks but added to them
/// ([`Corners::around`]): a page edge found by its brightness is only as
/// good as the light on it, and where the marks reach further they are the
/// better of the two.
fn paper_overhanging(source: &RgbImage, marked: &Corners) -> Option<Corners> {
    let (small, sw, sh) = small_grey(source.as_raw(), source.width(), source.height())?;
    let (paper, lit) = crate::locate::page_corners(&small, sw, sh)?;
    let covered = paper.area();
    if covered < TRIM_MIN_SHARE || lit < covered * TRIM_MIN_FILL {
        return None;
    }
    // The marks point at a sheet of paper on a table. What is found has to be
    // that sheet rather than a bright patch somewhere else in the frame, and
    // a sheet is about the size the marks say.
    let marked_area = marked.area();
    if covered < marked_area * 0.5 || covered > marked_area * 2.0 {
        return None;
    }
    let worst = paper
        .0
        .iter()
        .fold(0.0_f64, |worst, &corner| worst.max(outside(marked, corner)));
    if worst <= OVERHANG {
        return None;
    }
    tracing::info!(
        overhang = format!("{worst:.3}"),
        "the letter reaches past the marked corners; keeping the paper instead"
    );
    Some(paper)
}

/// How much darker than the page a strip has to be before it counts as the
/// table the page is lying on.
const TABLE_SHARE: f64 = 0.82;
/// How far beyond a side to look for that table, as a share of the picture.
/// Close, because the question is whether this side is the sheet's edge, not
/// what lies over by the frame.
const TABLE_STEP: f64 = 0.02;

/// Sides of a found page that still have paper beyond them, put back where
/// the picture ends.
///
/// The page is found by its brightness, so an edge lying in shadow — the far
/// one, usually, under a camera on a stalk — is found short. Trimming to it
/// then cuts into the sheet, which is how letters lost the column down their
/// right-hand side while the rest of the page looked perfectly well framed.
///
/// So before a side is cut, the strip it would cut away is looked at. If that
/// strip is as bright as the page, it is more page, and the side goes back
/// out to the edge of the picture. Only a side with something darker beyond
/// it — the table — is a side worth trimming to.
fn keep_paper(small: &[u8], sw: usize, sh: usize, page: Corners) -> Corners {
    let at = |x: f64, y: f64| {
        let (px, py) = (
            ((x * sw as f64) as usize).min(sw - 1),
            ((y * sh as f64) as usize).min(sh - 1),
        );
        small[py * sw + px] as f64
    };
    // The picture is already straightened onto the marks, so the page in it
    // is near enough square to look beyond it in strips.
    let [tl, tr, br, bl] = page.0;
    let (left, right) = (tl.0.min(bl.0), tr.0.max(br.0));
    let (top, bottom) = (tl.1.min(tr.1), bl.1.max(br.1));

    let mean = |points: &[(f64, f64)]| -> f64 {
        points.iter().map(|&(x, y)| at(x, y)).sum::<f64>() / points.len().max(1) as f64
    };
    let inside = mean(
        &(1..10)
            .flat_map(|i| (1..10).map(move |j| (i, j)))
            .map(|(i, j)| {
                (
                    left + (right - left) * i as f64 / 10.0,
                    top + (bottom - top) * j as f64 / 10.0,
                )
            })
            .collect::<Vec<_>>(),
    );

    /// Just outside a side — not halfway to the picture's edge, which only
    /// ever samples the table near the frame and says nothing about whether
    /// the side itself is inside the sheet.
    fn beyond(side: f64, edge: f64) -> Option<f64> {
        let room = edge - side;
        (room.abs() > 0.01).then(|| side + room.signum() * TABLE_STEP.min(room.abs() * 0.8))
    }
    let along = |from: f64, to: f64| {
        (1..10)
            .map(move |i| from + (to - from) * i as f64 / 10.0)
            .collect::<Vec<_>>()
    };
    let still_paper =
        |strip: Vec<(f64, f64)>| !strip.is_empty() && mean(&strip) > inside * TABLE_SHARE;

    let down = |x: f64| {
        along(top, bottom)
            .into_iter()
            .map(|y| (x, y))
            .collect::<Vec<_>>()
    };
    let across = |y: f64| {
        along(left, right)
            .into_iter()
            .map(|x| (x, y))
            .collect::<Vec<_>>()
    };
    let keep_left = beyond(left, 0.0).map(down).is_some_and(&still_paper);
    let keep_right = beyond(right, 1.0).map(down).is_some_and(&still_paper);
    let keep_top = beyond(top, 0.0).map(across).is_some_and(&still_paper);
    let keep_bottom = beyond(bottom, 1.0).map(across).is_some_and(&still_paper);

    // Only the sides with paper beyond them move; the rest of the quad — and
    // with it the turn of a letter lying a few degrees off — stays as found.
    let x = |keep: bool, edge: f64, found: f64| if keep { edge } else { found };
    Corners([
        (x(keep_left, 0.0, tl.0), x(keep_top, 0.0, tl.1)),
        (x(keep_right, 1.0, tr.0), x(keep_top, 0.0, tr.1)),
        (x(keep_right, 1.0, br.0), x(keep_bottom, 1.0, br.1)),
        (x(keep_left, 0.0, bl.0), x(keep_bottom, 1.0, bl.1)),
    ])
}

/// A small grey copy of packed RGB, which is all the paper has to be found
/// in: at this size the edge of a sheet is still an edge and the work is a
/// fortieth of the pixels.
fn small_grey(rgb: &[u8], width: u32, height: u32) -> Option<(Vec<u8>, usize, usize)> {
    let (w, h) = (width as usize, height as usize);
    if w < 16 || h < 16 || rgb.len() < w * h * 3 {
        return None;
    }
    let step = (w.max(h) / TRIM_SAMPLE).max(1);
    let (sw, sh) = (w / step, h / step);
    if sw < 16 || sh < 16 {
        return None;
    }
    let small = (0..sh)
        .flat_map(|y| (0..sw).map(move |x| (x, y)))
        .map(|(x, y)| {
            let i = ((y * step) * w + x * step) * 3;
            ((77 * rgb[i] as u32 + 150 * rgb[i + 1] as u32 + 29 * rgb[i + 2] as u32) >> 8) as u8
        })
        .collect();
    Some((small, sw, sh))
}

fn page_within(rgb: &[u8], width: u32, height: u32) -> Page {
    let nothing = Page {
        corners: None,
        covered: None,
    };
    let Some((small, sw, sh)) = small_grey(rgb, width, height) else {
        return nothing;
    };
    let Some((page, lit)) = crate::locate::page_corners(&small, sw, sh) else {
        return nothing;
    };
    let covered = page.area();
    if covered < TRIM_MIN_SHARE || lit < covered * TRIM_MIN_FILL {
        return Page {
            corners: None,
            covered: Some(covered).filter(|_| lit >= covered * TRIM_MIN_FILL),
        };
    }
    let page = keep_paper(&small, sw, sh, page.with_margin(TRIM_EDGE));
    // Already the page, near enough: not worth warping again.
    let whole = Corners::WHOLE.0;
    let off = page
        .0
        .iter()
        .zip(whole.iter())
        .map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs()))
        .fold(0.0, f64::max);
    Page {
        corners: (off >= 0.01).then_some(page),
        covered: Some(covered),
    }
}

fn trim_and_measure(rgb: Vec<u8>, width: u32, height: u32) -> Trimmed {
    let page = page_within(&rgb, width, height);
    let untouched = Trimmed { rgb, width, height };
    let Some(corners) = page.corners else {
        return untouched;
    };
    let Some(source) = RgbImage::from_raw(width, height, untouched.rgb.clone()) else {
        return untouched;
    };
    let quad = corners_in_pixels(&corners, width, height);
    let Ok((out_w, out_h)) = warped_size(&quad) else {
        return untouched;
    };
    tracing::info!(from = ?(width, height), to = ?(out_w, out_h), "straightened onto the page itself");
    Trimmed {
        rgb: warp(&source, &Homography::square_to(&quad), out_w, out_h),
        width: out_w,
        height: out_h,
    }
}

fn decode(jpeg: &[u8]) -> Result<RgbImage> {
    Ok(
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .context("could not read the photograph")?
            .into_rgb8(),
    )
}

/// Corners given as fractions, in a picture's own pixels.
fn corners_in_pixels(corners: &Corners, width: u32, height: u32) -> Corners {
    Corners(
        corners
            .0
            .map(|(x, y)| (x * width as f64, y * height as f64)),
    )
}

/// How big the warped picture is: the **longest** of each pair of opposite
/// edges, in the pixels it is warped from.
///
/// The longest rather than the average, because a page lying at an angle to
/// the camera has its near edge resolved best of all. Averaging throws those
/// pixels away for the sake of a tidier number; keeping them costs a little
/// upsampling at the far edge, where there was nothing to keep.
fn warped_size(quad: &Corners) -> Result<(u32, u32)> {
    let length = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let [tl, tr, br, bl] = quad.0;
    let width = length(tl, tr).max(length(bl, br)).round() as u32;
    let height = length(tl, bl).max(length(tr, br)).round() as u32;
    if width < 16 || height < 16 {
        bail!("the corners enclose too little of the photograph ({width}×{height})");
    }
    Ok((width, height))
}

/// Warps a photograph so `corners` become its corners, as packed RGB.
fn straighten_rgb(jpeg: &[u8], corners: &Corners) -> Result<(Vec<u8>, u32, u32)> {
    let source = decode(jpeg)?;
    let pixels = corners_in_pixels(corners, source.width(), source.height());
    let (width, height) = warped_size(&pixels)?;
    let straight = warp(&source, &Homography::square_to(&pixels), width, height);
    Ok((straight, width, height))
}

fn encode(straight: &[u8], width: u32, height: u32, size_hint: usize) -> Result<Vec<u8>> {
    let (w, h) = (
        u16::try_from(width).context("the straightened page is too wide")?,
        u16::try_from(height).context("the straightened page is too tall")?,
    );
    let mut out = Vec::with_capacity(size_hint);
    let mut encoder = Encoder::new(&mut out, QUALITY);
    // Colour at half resolution, as the camera wrote it: at this quality the
    // encoder would otherwise keep it whole, which on a Pi Zero doubles the
    // time and adds nothing the photograph had.
    encoder.set_sampling_factor(SamplingFactor::R_4_2_0);
    encoder
        .encode(straight, w, h, ColorType::Rgb)
        .context("could not write the straightened photograph")?;
    Ok(out)
}

/// A small greyscale frame straightened as a photograph would be, for the
/// live view on the LCD — so it shows the page the corners mark, the way the
/// photograph will show it, and not the crop around it.
///
/// `aspect` is the frame's true width over its height: a preview frame is
/// 320×240 whatever the shape of the crop, so it arrives stretched, and the
/// page's proportions are worked out in the crop's own. The result fits in
/// `max_width`×`max_height`.
pub fn straighten_luma(
    luma: &[u8],
    width: usize,
    height: usize,
    aspect: f64,
    corners: &Corners,
    max_width: u32,
    max_height: u32,
) -> crate::picture::Picture {
    if width < 2 || height < 2 || luma.len() < width * height || aspect <= 0.0 {
        return crate::picture::Picture::empty();
    }
    let true_size = |(x, y): (f64, f64)| (x * aspect, y);
    let [tl, tr, br, bl] = corners.0.map(true_size);
    let length = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let page_width = (length(tl, tr) + length(bl, br)) / 2.0;
    let page_height = (length(tl, bl) + length(tr, br)) / 2.0;
    if page_width <= 0.0 || page_height <= 0.0 {
        return crate::picture::Picture::empty();
    }
    let scale = (max_width as f64 / page_width).min(max_height as f64 / page_height);
    let (out_w, out_h) = (
        ((page_width * scale).round() as usize).max(1),
        ((page_height * scale).round() as usize).max(1),
    );
    let pixels = Corners(
        corners
            .0
            .map(|(x, y)| (x * width as f64, y * height as f64)),
    );
    let map = Homography::square_to(&pixels);
    let (max_x, max_y) = ((width - 1) as f64, (height - 1) as f64);
    let mut out = Vec::with_capacity(out_w * out_h);
    for row in 0..out_h {
        let v = (row as f64 + 0.5) / out_h as f64;
        for column in 0..out_w {
            let u = (column as f64 + 0.5) / out_w as f64;
            let (x, y) = map.apply(u, v);
            let (x, y) = ((x - 0.5).clamp(0.0, max_x), (y - 0.5).clamp(0.0, max_y));
            out.push(luma[y.round() as usize * width + x.round() as usize]);
        }
    }
    crate::picture::Picture {
        width: out_w as u32,
        height: out_h as u32,
        luma: std::sync::Arc::new(out),
    }
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
