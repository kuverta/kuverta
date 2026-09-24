//! The rig's arithmetic over raw pixels, fed sizes that do not match the
//! pixels there are.
//!
//! None of these functions parses a format. They take a buffer, a width and a
//! height, and index into it — which is the whole reason they are here. Every
//! one of them computes offsets from the two numbers rather than from the
//! buffer's length, so the question this asks is what happens when the three
//! disagree: a JPEG that decoded short, a camera frame that arrived truncated,
//! a page whose dimensions came off a file somebody else wrote.
//!
//! In Rust a disagreement like that is a panic rather than a read out of
//! bounds, which is the right failure and still a failure: `bmp` is served
//! over the rig's HTTP port, and a panic there is a page of the setup screen
//! that stops answering. So what is checked is that each one either returns
//! or declines, for every width and height there is — including the ones that
//! overflow when multiplied together, which the fuzz profile turns into
//! panics on purpose.
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use scannerd::flatten::flatten;
use scannerd::locate::page_corners;
use scannerd::picture::Picture;
use scannerd::quality::assess;
use scannerd::straighten::trim_to_page;
use scannerd::web::bmp;

#[derive(Arbitrary, Debug)]
struct Picture4 {
    /// Kept small on purpose: what is being explored is the relationship
    /// between these and the buffer, not how long a big page takes.
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

fuzz_target!(|input: Picture4| {
    let Picture4 {
        width,
        height,
        pixels,
    } = input;
    // A megapixel is plenty to find an off-by-one with and keeps a run from
    // spending all its time in one allocation.
    let (width, height) = (u32::from(width) % 1200, u32::from(height) % 1200);

    // One byte a pixel: the camera's luma plane, as detection and the display
    // see it.
    let _ = bmp(&pixels, width, height);
    let _ = Picture::scaled(
        &pixels,
        width as usize,
        height as usize,
        // Two thumbnail sizes, one of them not a divisor of anything.
        64,
        37,
    );
    if width > 0 && height > 0 {
        let _ = page_corners(&pixels, width as usize, height as usize);
        // `assess` reads a histogram over a region it works out itself, and
        // has no way to decline: it is only ever called on a decoded JPEG,
        // where the size came from the decoder. Held to the same standard
        // here anyway, since the decoder is not this crate's code.
        if pixels.len() >= (width as usize).saturating_mul(height as usize) {
            let _ = assess(&pixels, width as usize, height as usize);
        }
    }

    // Three bytes a pixel: a straightened page, as it is trimmed and as its
    // light is flattened.
    let mut rgb = pixels.clone();
    let before = rgb.len();
    if flatten(&mut rgb, width, height).is_some() {
        // A tone curve moves no pixels and loses none. A flattening that
        // changed the size of the picture would be a bug that only showed up
        // as a corrupt JPEG much later.
        assert_eq!(rgb.len(), before, "{width}×{height} changed size");
    }
    // `trim_to_page` asks for three bytes a pixel and hands a buffer it does
    // not recognise straight back, dimensions and all — so it is fed anything
    // to see that it returns, and held to what it promises only when it was
    // given what it asks for.
    let enough = pixels.len() >= (width as usize).saturating_mul(height as usize) * 3;
    let (out, out_w, out_h) = trim_to_page(pixels, width, height);
    if enough && out_w > 0 && out_h > 0 {
        // What comes back has to describe itself: the caller hands these
        // three straight to a JPEG encoder, which reads as many bytes as the
        // width and the height between them say are there.
        assert!(
            out.len() >= (out_w as usize).saturating_mul(out_h as usize) * 3,
            "trimmed to {out_w}×{out_h} but kept only {} bytes",
            out.len()
        );
    }
});
