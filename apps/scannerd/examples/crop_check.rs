//! What a photograph becomes, and how much of the page survives it.
//!
//! ```text
//! cargo run --example crop_check -- photo.jpg <8 corner fractions> [x,y,w,h roi]
//! ```
//!
//! For asking a real photograph the question that matters when a letter comes
//! out cut: where is the paper, where are the marks, and which of the two
//! steps — the warp onto the marks, or the trim onto the page inside them —
//! took the missing part. It writes each step out beside the photograph:
//!
//! - `-roi.jpg`, the sensor crop, when an roi is given;
//! - `-looking.jpg`, the photograph warped onto the marked corners;
//! - `-trimmed.jpg`, that trimmed onto the page found inside it;
//! - `-page.jpg`, what a capture actually keeps;
//! - `-paper.jpg`, the photograph warped onto the paper's own corners.
//!
//! and prints where the paper lies in the marks' own frame, where 0 and 1 are
//! their edges — so anything outside that range is page the marks would cut.
use anyhow::{bail, Context, Result};
use scannerd::straighten::{Corners, Homography};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, corners] = &args[..2.min(args.len())] else {
        bail!("usage: crop_check <photo.jpg> <8 corner fractions> [x,y,w,h roi]");
    };
    let corners = Corners::parse(corners)?;

    let mut jpeg = std::fs::read(path)?;
    if let Some(roi) = args.get(2) {
        jpeg = crop_to_roi(&jpeg, roi)?;
        std::fs::write(format!("{path}-roi.jpg"), &jpeg)?;
    }

    // Where the paper is in the photograph itself, which is the question the
    // marked corners cannot answer: a letter that overhangs them is cut
    // before anything downstream ever sees it.
    let photo = image::load_from_memory(&jpeg)?.into_luma8();
    let (pw, ph) = (photo.width() as usize, photo.height() as usize);
    match scannerd::locate::page_corners(photo.as_raw(), pw, ph) {
        Some((paper, lit)) => {
            println!(
                "the paper covers {:.3} of the photograph, {lit:.3} of it lit",
                paper.area()
            );
            let back = Homography::square_to(&corners)
                .invert()
                .context("the marked corners are not a quad")?;
            println!("and lies, in the marks' own frame:");
            for (name, &(x, y)) in ["top left", "top right", "bottom right", "bottom left"]
                .iter()
                .zip(paper.0.iter())
            {
                let (u, v) = back.apply(x, y);
                println!("  {name:>12}: u {u:+.3}  v {v:+.3}");
            }
            std::fs::write(
                format!("{path}-paper.jpg"),
                scannerd::straighten::straighten(&jpeg, &paper)?,
            )?;
        }
        None => println!("no paper found in the photograph itself"),
    }

    let looking = scannerd::straighten::straighten(&jpeg, &corners)?;
    std::fs::write(format!("{path}-looking.jpg"), &looking)?;

    // The trim on its own, so it can be told apart from the warp before it.
    let look = image::load_from_memory(&looking)?.into_rgb8();
    let (lw, lh) = (look.width(), look.height());
    let (trimmed, tw, th) = scannerd::straighten::trim_to_page(look.into_raw(), lw, lh);
    println!("the trim: {lw}×{lh} -> {tw}×{th}");
    write_rgb(&format!("{path}-trimmed.jpg"), trimmed, tw, th)?;

    let kept = scannerd::straighten::straighten_trimmed(&jpeg, &corners)?;
    std::fs::write(format!("{path}-page.jpg"), &kept.jpeg)?;
    println!(
        "the page covers {:?} of what the marks enclose",
        kept.covered
    );
    Ok(())
}

fn write_rgb(path: &str, rgb: Vec<u8>, width: u32, height: u32) -> Result<()> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_raw(width, height, rgb)
        .context("the trimmed picture is the wrong size")?
        .write_to(&mut out, image::ImageFormat::Jpeg)?;
    std::fs::write(path, out.into_inner())?;
    Ok(())
}

/// The sensor crop `rpicam-still --roi` would have made, so a full-sensor
/// photograph can be put through the pipeline a capture goes through.
fn crop_to_roi(jpeg: &[u8], roi: &str) -> Result<Vec<u8>> {
    let numbers: Vec<f64> = roi
        .split(',')
        .map(|n| n.trim().parse().context("an roi number"))
        .collect::<Result<_>>()?;
    let [x, y, w, h] = numbers[..] else {
        bail!("the roi is x,y,w,h");
    };
    let image = image::load_from_memory(jpeg)?.into_rgb8();
    let (iw, ih) = (image.width() as f64, image.height() as f64);
    let cropped = image::imageops::crop_imm(
        &image,
        (x * iw) as u32,
        (y * ih) as u32,
        (w * iw) as u32,
        (h * ih) as u32,
    )
    .to_image();
    let mut out = std::io::Cursor::new(Vec::new());
    cropped.write_to(&mut out, image::ImageFormat::Jpeg)?;
    Ok(out.into_inner())
}
