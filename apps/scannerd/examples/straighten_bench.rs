//! How long straightening takes, stage by stage:
//! `cargo run --release --example straighten_bench [photo.jpg]`.
use std::time::Instant;

use scannerd::straighten::{straighten, Corners};

fn main() {
    let jpeg = match std::env::args().nth(1) {
        Some(path) => std::fs::read(path).unwrap(),
        None => {
            let picture = image::RgbImage::from_fn(2592, 1944, |x, y| {
                image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
            });
            let mut out = std::io::Cursor::new(Vec::new());
            picture
                .write_to(&mut out, image::ImageFormat::Jpeg)
                .unwrap();
            out.into_inner()
        }
    };

    let started = Instant::now();
    let picture = image::load_from_memory(&jpeg).unwrap().into_rgb8();
    println!("decode {:?}", started.elapsed());
    let started = Instant::now();
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, 92);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    encoder
        .encode(
            picture.as_raw(),
            picture.width() as u16,
            picture.height() as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .unwrap();
    println!("encode {:?}", started.elapsed());

    let corners = Corners::parse("0.2,0.05,0.8,0.05,1,1,0,1").unwrap();
    let started = Instant::now();
    let out = straighten(&jpeg, &corners).unwrap();
    println!("all: {} bytes in {:?}", out.len(), started.elapsed());
}
