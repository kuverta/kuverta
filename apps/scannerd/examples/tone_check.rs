//! Flattens a page's light and says what it did: `tone_check <in.jpg> <out.jpg>`.

use anyhow::{Context, Result};
use jpeg_encoder::{ColorType, Encoder};
use scannerd::flatten::flatten;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .context("usage: tone_check <in.jpg> <out.jpg>")?;
    let output = args
        .next()
        .context("usage: tone_check <in.jpg> <out.jpg>")?;

    let source = image::ImageReader::open(&input)?
        .with_guessed_format()?
        .decode()?
        .into_rgb8();
    let (width, height) = (source.width(), source.height());
    let mut rgb = source.into_raw();

    let before = levels(&rgb);
    let started = std::time::Instant::now();
    let tone = flatten(&mut rgb, width, height);
    let took = started.elapsed();
    let after = levels(&rgb);

    println!("{width}×{height}, flattened in {} ms", took.as_millis());
    println!(
        "  before: ink {} paper {} (as taken: {tone:?})",
        before.0, before.1
    );
    println!("  after:  ink {} paper {}", after.0, after.1);

    let mut out = Vec::new();
    Encoder::new(&mut out, 92).encode(&rgb, width as u16, height as u16, ColorType::Rgb)?;
    std::fs::write(&output, out)?;
    Ok(())
}

/// The half-percentile and the median of a picture's luma.
fn levels(rgb: &[u8]) -> (u8, u8) {
    let mut counts = [0u32; 256];
    for pixel in rgb.chunks_exact(3) {
        let luma = (77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8;
        counts[luma as usize] += 1;
    }
    let total = (rgb.len() / 3) as u32;
    let at = |fraction: f64| {
        let wanted = (total as f64 * fraction) as u32;
        let mut seen = 0;
        for (value, &count) in counts.iter().enumerate() {
            seen += count;
            if seen > wanted {
                return value as u8;
            }
        }
        255
    };
    (at(0.005), at(0.5))
}
