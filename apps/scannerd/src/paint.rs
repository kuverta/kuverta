//! Drawing for the LCD: smooth shapes and smooth type.
//!
//! The panel's first screens were drawn straight into pixels — rectangles with
//! hard edges and a bitmap font — and looked it. Here the picture is painted
//! with `tiny-skia`, which draws paths with anti-aliasing, and the type is
//! Inter (SIL Open Font License, in `assets/fonts`) rasterised by `fontdue`, so
//! letters have soft edges at any size. Both are pure Rust and build for the Pi
//! like everything else.
//!
//! What comes out is an RGBA picture. [`crate::lcd`] turns it into the panel's
//! 16-bit colour, dithering as it goes so that a gradient does not band.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use fontdue::{Font, FontSettings};
use tiny_skia::{
    Color, FillRule, LinearGradient, Paint, PathBuilder, Pixmap, PixmapPaint, PixmapRef, Point,
    Rect, Shader, Stroke, Transform,
};

/// Inter, the one typeface: regular for what is read, semi-bold for what is
/// looked at.
const REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Semibold,
}

struct Fonts {
    regular: Font,
    semibold: Font,
    /// Glyphs already rasterised: the same words are drawn again every frame.
    glyphs: HashMap<(Weight, u32, char), (fontdue::Metrics, Vec<u8>)>,
}

fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let load = |bytes: &[u8]| {
            Font::from_bytes(bytes, FontSettings::default()).expect("the typeface is built in")
        };
        Mutex::new(Fonts {
            regular: load(REGULAR),
            semibold: load(SEMIBOLD),
            glyphs: HashMap::new(),
        })
    })
}

impl Fonts {
    fn font(&self, weight: Weight) -> &Font {
        match weight {
            Weight::Regular => &self.regular,
            Weight::Semibold => &self.semibold,
        }
    }

    fn glyph(&mut self, weight: Weight, size: f32, ch: char) -> (fontdue::Metrics, Vec<u8>) {
        let key = (weight, (size * 4.0) as u32, ch);
        if let Some(glyph) = self.glyphs.get(&key) {
            return glyph.clone();
        }
        let glyph = self.font(weight).rasterize(ch, size);
        self.glyphs.insert(key, glyph.clone());
        glyph
    }
}

/// How wide `words` are, and how far their letters reach above the line.
pub fn measure(words: &str, weight: Weight, size: f32) -> (f32, f32) {
    let fonts = fonts().lock().unwrap();
    let font = fonts.font(weight);
    let mut width = 0.0;
    let mut top = 0.0f32;
    let mut previous = None;
    for ch in words.chars() {
        if let Some(previous) = previous {
            width += font.horizontal_kern(previous, ch, size).unwrap_or(0.0);
        }
        let metrics = font.metrics(ch, size);
        width += metrics.advance_width;
        top = top.max((metrics.height as f32) + metrics.ymin as f32);
        previous = Some(ch);
    }
    (width, top)
}

/// Draws `words` with their left edge at `x` and their top at `y`.
pub fn text(
    pixmap: &mut Pixmap,
    words: &str,
    at: (f32, f32),
    weight: Weight,
    size: f32,
    colour: Color,
) {
    let mut fonts = fonts().lock().unwrap();
    let (_, top) = {
        let font = fonts.font(weight);
        let mut top = 0.0f32;
        for ch in words.chars() {
            let metrics = font.metrics(ch, size);
            top = top.max(metrics.height as f32 + metrics.ymin as f32);
        }
        (0.0, top)
    };
    let mut pen = at.0;
    let mut previous = None;
    for ch in words.chars() {
        if let Some(previous) = previous {
            pen += fonts
                .font(weight)
                .horizontal_kern(previous, ch, size)
                .unwrap_or(0.0);
        }
        let (metrics, bitmap) = fonts.glyph(weight, size, ch);
        let left = pen + metrics.xmin as f32;
        // `ymin` is how far the glyph hangs below its own box; the tallest
        // letter of the run sits at `y`.
        let glyph_top = at.1 + top - (metrics.height as f32 + metrics.ymin as f32);
        blend(
            pixmap,
            &bitmap,
            metrics.width,
            metrics.height,
            left,
            glyph_top,
            colour,
        );
        pen += metrics.advance_width;
        previous = Some(ch);
    }
}

/// `words` at the size that fits `width`, the first of `sizes` that does —
/// cut short with an ellipsis when even the smallest does not.
pub fn fitted(words: &str, weight: Weight, sizes: &[f32], width: f32) -> (String, f32) {
    let smallest = *sizes.last().unwrap_or(&12.0);
    for size in sizes {
        if measure(words, weight, *size).0 <= width {
            return (words.to_string(), *size);
        }
    }
    let mut cut: Vec<char> = words.chars().collect();
    while cut.pop().is_some() {
        let shortened = format!("{}...", cut.iter().collect::<String>().trim_end());
        if measure(&shortened, weight, smallest).0 <= width {
            return (shortened, smallest);
        }
    }
    (words.to_string(), smallest)
}

/// A glyph's coverage, blended onto the picture in `colour`.
fn blend(
    pixmap: &mut Pixmap,
    coverage: &[u8],
    width: usize,
    height: usize,
    x: f32,
    y: f32,
    colour: Color,
) {
    if width == 0 || height == 0 {
        return;
    }
    let (ox, oy) = (x.round() as i32, y.round() as i32);
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let (r, g, b, a) = (colour.red(), colour.green(), colour.blue(), colour.alpha());
    let pixels = pixmap.pixels_mut();
    for row in 0..height as i32 {
        let py = oy + row;
        if py < 0 || py >= ph {
            continue;
        }
        for column in 0..width as i32 {
            let px = ox + column;
            if px < 0 || px >= pw {
                continue;
            }
            let alpha = coverage[row as usize * width + column as usize] as f32 / 255.0 * a;
            if alpha <= 0.0 {
                continue;
            }
            let under = pixels[(py * pw + px) as usize];
            let mix = |over: f32, under: u8| {
                ((over * alpha + under as f32 / 255.0 * (1.0 - alpha)) * 255.0).round() as u8
            };
            pixels[(py * pw + px) as usize] = tiny_skia::PremultipliedColorU8::from_rgba(
                mix(r, under.red()),
                mix(g, under.green()),
                mix(b, under.blue()),
                255,
            )
            .expect("opaque");
        }
    }
}

/// A rounded rectangle, filled.
pub fn rounded(pixmap: &mut Pixmap, rect: (f32, f32, f32, f32), radius: f32, shader: Shader) {
    let Some(path) = rounded_path(rect, radius) else {
        return;
    };
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.shader = shader;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// The same, with a hairline of `edge` around it.
pub fn rounded_edge(pixmap: &mut Pixmap, rect: (f32, f32, f32, f32), radius: f32, edge: Color) {
    let Some(path) = rounded_path(rect, radius) else {
        return;
    };
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(edge);
    pixmap.stroke_path(
        &path,
        &paint,
        &Stroke {
            width: 1.0,
            ..Stroke::default()
        },
        Transform::identity(),
        None,
    );
}

pub fn rounded_path((x, y, w, h): (f32, f32, f32, f32), radius: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = radius.min(w / 2.0).min(h / 2.0);
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    path.finish()
}

/// A colour, from the usual eight bits a channel.
pub fn colour(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

pub fn faded(base: Color, alpha: f32) -> Color {
    Color::from_rgba(base.red(), base.green(), base.blue(), alpha).expect("a colour")
}

/// `base` lightened towards white by `amount`.
pub fn lighter(base: Color, amount: f32) -> Color {
    let up = |channel: f32| channel + (1.0 - channel) * amount;
    Color::from_rgba(
        up(base.red()),
        up(base.green()),
        up(base.blue()),
        base.alpha(),
    )
    .expect("a colour")
}

/// A shader of one colour.
pub fn flat(colour: Color) -> Shader<'static> {
    Shader::SolidColor(colour)
}

/// A shader that goes from `top` to `bottom` down the rectangle.
pub fn down(rect: (f32, f32, f32, f32), top: Color, bottom: Color) -> Shader<'static> {
    LinearGradient::new(
        Point::from_xy(rect.0, rect.1),
        Point::from_xy(rect.0, rect.1 + rect.3),
        vec![
            tiny_skia::GradientStop::new(0.0, top),
            tiny_skia::GradientStop::new(1.0, bottom),
        ],
        tiny_skia::SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or_else(|| flat(top))
}

/// Draws `picture` — greyscale, `width`×`height` — to fit `rect`, with the
/// corners rounded off. Returns where it ended up.
pub fn grey_picture(
    pixmap: &mut Pixmap,
    picture: &[u8],
    width: u32,
    height: u32,
    rect: (f32, f32, f32, f32),
    radius: f32,
) -> Option<(f32, f32, f32, f32)> {
    if width == 0 || height == 0 || picture.len() < (width * height) as usize {
        return None;
    }
    let mut source = Pixmap::new(width, height)?;
    for (pixel, &grey) in source.pixels_mut().iter_mut().zip(picture) {
        *pixel = tiny_skia::PremultipliedColorU8::from_rgba(grey, grey, grey, 255)?;
    }
    let scale = (rect.2 / width as f32).min(rect.3 / height as f32);
    let (w, h) = (width as f32 * scale, height as f32 * scale);
    let (x, y) = (rect.0 + (rect.2 - w) / 2.0, rect.1 + (rect.3 - h) / 2.0);
    let path = rounded_path((x, y, w, h), radius)?;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.shader = tiny_skia::Pattern::new(
        PixmapRef::from_bytes(source.data(), width, height)?,
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        Transform::from_scale(scale, scale).post_translate(x, y),
    );
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Some((x, y, w, h))
}

/// The line drawings on the buttons and beside the headlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Camera,
    Check,
    Cross,
    Folder,
    Bin,
    Undo,
    Play,
    Stop,
    Envelope,
    Clock,
    Warning,
    Table,
    /// A chevron pointing back.
    Back,
}

/// Draws `icon` in a box `size` across with its top left at `at`.
pub fn icon(pixmap: &mut Pixmap, icon: Icon, at: (f32, f32), size: f32, colour: Color) {
    let (x, y, s) = (at.0, at.1, size);
    let p = |u: f32, v: f32| (x + u * s, y + v * s);
    let mut path = PathBuilder::new();
    let line = |path: &mut PathBuilder, points: &[(f32, f32)], close: bool| {
        if let Some((first, rest)) = points.split_first() {
            let start = p(first.0, first.1);
            path.move_to(start.0, start.1);
            for (u, v) in rest {
                let point = p(*u, *v);
                path.line_to(point.0, point.1);
            }
            if close {
                path.close();
            }
        }
    };
    match icon {
        Icon::Check => line(&mut path, &[(0.15, 0.55), (0.4, 0.8), (0.85, 0.22)], false),
        Icon::Cross => {
            line(&mut path, &[(0.2, 0.2), (0.8, 0.8)], false);
            line(&mut path, &[(0.8, 0.2), (0.2, 0.8)], false);
        }
        Icon::Folder => {
            line(
                &mut path,
                &[
                    (0.1, 0.8),
                    (0.1, 0.25),
                    (0.42, 0.25),
                    (0.52, 0.38),
                    (0.9, 0.38),
                    (0.9, 0.8),
                ],
                true,
            );
        }
        Icon::Bin => {
            line(&mut path, &[(0.15, 0.28), (0.85, 0.28)], false);
            line(
                &mut path,
                &[(0.25, 0.28), (0.3, 0.85), (0.7, 0.85), (0.75, 0.28)],
                false,
            );
            line(
                &mut path,
                &[(0.38, 0.28), (0.4, 0.15), (0.6, 0.15), (0.62, 0.28)],
                false,
            );
        }
        Icon::Undo => {
            let (cx, cy, r) = (0.5, 0.55, 0.3);
            let start = p(cx - r, cy);
            path.move_to(start.0, start.1);
            let mid = p(cx, cy - r * 1.1);
            let end = p(cx + r, cy);
            path.quad_to(mid.0, mid.1, end.0, end.1);
            let low = p(cx, cy + r * 0.95);
            path.quad_to(end.0, p(cx + r, cy + r).1, low.0, low.1);
            line(&mut path, &[(0.2, 0.28), (0.2, 0.55), (0.46, 0.55)], false);
        }
        Icon::Play => line(&mut path, &[(0.28, 0.16), (0.84, 0.5), (0.28, 0.84)], true),
        Icon::Stop => line(
            &mut path,
            &[(0.24, 0.24), (0.76, 0.24), (0.76, 0.76), (0.24, 0.76)],
            true,
        ),
        Icon::Camera => {
            line(
                &mut path,
                &[
                    (0.08, 0.78),
                    (0.08, 0.32),
                    (0.3, 0.32),
                    (0.38, 0.2),
                    (0.62, 0.2),
                    (0.7, 0.32),
                    (0.92, 0.32),
                    (0.92, 0.78),
                ],
                true,
            );
            let centre = p(0.5, 0.53);
            path.push_circle(centre.0, centre.1, 0.17 * s);
        }
        Icon::Envelope => {
            line(
                &mut path,
                &[(0.1, 0.25), (0.9, 0.25), (0.9, 0.75), (0.1, 0.75)],
                true,
            );
            line(&mut path, &[(0.1, 0.28), (0.5, 0.56), (0.9, 0.28)], false);
        }
        Icon::Clock => {
            let centre = p(0.5, 0.5);
            path.push_circle(centre.0, centre.1, 0.36 * s);
            line(&mut path, &[(0.5, 0.28), (0.5, 0.52), (0.68, 0.62)], false);
        }
        Icon::Warning => {
            line(&mut path, &[(0.5, 0.18), (0.5, 0.58)], false);
            line(&mut path, &[(0.5, 0.74), (0.5, 0.8)], false);
        }
        Icon::Back => line(&mut path, &[(0.62, 0.2), (0.34, 0.5), (0.62, 0.8)], false),
        Icon::Table => {
            line(
                &mut path,
                &[(0.12, 0.3), (0.88, 0.3), (0.88, 0.72), (0.12, 0.72)],
                true,
            );
            line(&mut path, &[(0.12, 0.51), (0.88, 0.51)], false);
        }
    }
    let Some(path) = path.finish() else {
        return;
    };
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(colour);
    let filled = matches!(icon, Icon::Play | Icon::Stop);
    if filled {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    } else {
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: (s * 0.1).max(1.6),
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
}

/// A picture to draw on, filled with `colour`.
pub fn picture(width: u32, height: u32, colour: Color) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("a picture of the panel's size");
    pixmap.fill(colour);
    pixmap
}

/// Fills a whole rectangle, square corners.
pub fn fill_rect(pixmap: &mut Pixmap, rect: (f32, f32, f32, f32), shader: Shader) {
    let Some(rect) = Rect::from_xywh(rect.0, rect.1, rect.2, rect.3) else {
        return;
    };
    let mut paint = Paint {
        anti_alias: false,
        ..Paint::default()
    };
    paint.shader = shader;
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

/// Draws a picture over another, as it is.
pub fn draw_over(pixmap: &mut Pixmap, other: &Pixmap, at: (f32, f32)) {
    pixmap.draw_pixmap(
        at.0 as i32,
        at.1 as i32,
        other.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}
