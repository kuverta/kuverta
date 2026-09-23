//! Straightening a page photographed at an angle.

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, Rgb, RgbImage};
use scannerd::straighten::{straighten, Corners, Homography};

fn close(a: f64, b: f64, within: f64) -> bool {
    (a - b).abs() < within
}

/// A trapezium, as a camera tilted towards the far edge sees a page.
fn trapezium() -> Corners {
    Corners::new([(0.30, 0.15), (0.70, 0.15), (0.90, 0.85), (0.10, 0.85)]).unwrap()
}

#[test]
fn the_square_lands_on_the_corners() {
    let corners = trapezium();
    let map = Homography::square_to(&corners);
    for ((u, v), (x, y)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .into_iter()
        .zip(corners.0)
    {
        let (mx, my) = map.apply(u, v);
        assert!(
            close(mx, x, 1e-9) && close(my, y, 1e-9),
            "({u},{v}) went to ({mx},{my})"
        );
    }
}

#[test]
fn corners_in_the_wrong_order_or_off_the_picture_are_refused() {
    assert!(Corners::parse("0.3,0.15,0.7,0.15,0.9,0.85,0.1,0.85").is_ok());
    for bad in [
        // top right and bottom right swapped: the edges cross
        "0.3,0.15,0.9,0.85,0.7,0.15,0.1,0.85",
        // anticlockwise
        "0.3,0.15,0.1,0.85,0.9,0.85,0.7,0.15",
        "0.3,0.15,0.7,0.15,1.2,0.85,0.1,0.85",
        "0.3,0.15,0.7,0.15,0.9,0.85",
        "0.5,0.5,0.51,0.5,0.51,0.51,0.5,0.51",
        "a,b,c,d,e,f,g,h",
    ] {
        assert!(Corners::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_crop_holds_every_corner_and_the_corners_are_found_in_it() {
    let corners = Corners::parse("0.3004,0.1502,0.7,0.15,0.9,0.85,0.1,0.8517").unwrap();
    let crop = corners.crop();
    assert_eq!(crop, "0.100,0.150,0.800,0.702");
    assert!(scannerd::settings::valid_roi(&crop).is_ok());

    let inside = corners.within(&crop).unwrap();
    for (x, y) in inside.0 {
        assert!((-1e-9..=1.0 + 1e-9).contains(&x), "{inside:?}");
        assert!((-1e-9..=1.0 + 1e-9).contains(&y), "{inside:?}");
    }
    assert!(close(inside.0[3].0, 0.0, 1e-9));
    assert!(close(inside.0[1].1, 0.0, 1e-9));
}

#[test]
fn room_around_a_page_is_added_on_the_table_and_kept_in_the_picture() {
    let square = Corners::new([(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]).unwrap();
    let grown = square.with_margin(0.1);
    // 10% of a 0.4-wide page on each side.
    assert!(
        close(grown.0[0].0, 0.26, 1e-9) && close(grown.0[2].1, 0.74, 1e-9),
        "{grown:?}"
    );

    // At a picture's edge the room shrinks rather than leaving it.
    let at_edge = Corners::new([(0.0, 0.2), (0.5, 0.2), (0.5, 0.8), (0.0, 0.8)]).unwrap();
    let grown = at_edge.with_margin(0.1);
    assert!(grown
        .0
        .iter()
        .all(|&(x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)));

    // In perspective, the far edge grows by less than the near one, as it
    // does on the table.
    let grown = trapezium().with_margin(0.1);
    let far = grown.0[1].0 - grown.0[0].0;
    let near = grown.0[2].0 - grown.0[3].0;
    assert!(
        far > 0.4 && near > 0.8 && near - 0.8 > far - 0.4,
        "{grown:?}"
    );
}

/// A photograph of a table with a page on it, seen at an angle. The page is
/// split into four quarters, dark top left and bottom right, so that both
/// directions of the result can be checked.
fn photograph(width: u32, height: u32, corners: &Corners) -> Vec<u8> {
    let mut picture = RgbImage::from_pixel(width, height, Rgb([60, 90, 60]));
    let map = Homography::square_to(corners);
    // Painted from the page's side, finely enough to leave no gaps.
    let steps = 1200;
    for i in 0..steps {
        for j in 0..steps {
            let (u, v) = (i as f64 / steps as f64, j as f64 / steps as f64);
            let dark = (u < 0.5) == (v < 0.5);
            let (x, y) = map.apply(u, v);
            let (px, py) = ((x * width as f64) as u32, (y * height as f64) as u32);
            if px < width && py < height {
                let colour = if dark {
                    Rgb([20, 20, 20])
                } else {
                    Rgb([240, 240, 240])
                };
                picture.put_pixel(px, py, colour);
            }
        }
    }
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode(picture.as_raw(), width, height, ExtendedColorType::Rgb8)
        .unwrap();
    jpeg
}

#[test]
fn a_page_seen_at_an_angle_comes_out_square() {
    let corners = trapezium();
    let jpeg = photograph(400, 300, &corners);

    let straight = straighten(&jpeg, &corners).unwrap();
    let picture = image::load_from_memory(&straight).unwrap().into_rgb8();

    // The top edge is 160 pixels, the bottom 320; the sides are each
    // hypot(80, 210).
    assert_eq!(picture.width(), 240);
    assert_eq!(picture.height(), (80f64.hypot(210.0)).round() as u32);

    let grey = |fx: f64, fy: f64| {
        let pixel = picture.get_pixel(
            (fx * picture.width() as f64) as u32,
            (fy * picture.height() as f64) as u32,
        );
        pixel.0.iter().map(|&c| c as u32).sum::<u32>() / 3
    };
    // Each quarter's middle, and near each corner: no table anywhere.
    for (fx, fy, dark) in [
        (0.25, 0.25, true),
        (0.75, 0.25, false),
        (0.25, 0.75, false),
        (0.75, 0.75, true),
        (0.03, 0.03, true),
        (0.97, 0.03, false),
        (0.03, 0.97, false),
        (0.97, 0.97, true),
    ] {
        let value = grey(fx, fy);
        if dark {
            assert!(value < 50, "({fx},{fy}) should be dark, is {value}");
        } else {
            assert!(value > 200, "({fx},{fy}) should be light, is {value}");
        }
    }
    // The quarters meet in the middle, where the page's middle is — not
    // where the trapezium's middle is in the picture, which is lower.
    assert!(grey(0.25, 0.46) < 50 && grey(0.25, 0.54) > 200);
    assert!(grey(0.46, 0.25) < 50 && grey(0.54, 0.25) > 200);
}

#[test]
fn what_is_not_a_photograph_is_an_error_rather_than_a_panic() {
    assert!(straighten(b"\xff\xd8not really", &trapezium()).is_err());
    let tiny = Corners::new([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]).unwrap();
    assert!(straighten(&photograph(8, 8, &tiny), &tiny).is_err());
}

#[test]
fn a_quarter_turn_clockwise_puts_the_left_edge_on_top() {
    // Tall and dark on the left: turned clockwise it is wide and dark on top.
    let (width, height) = (60u32, 100u32);
    let picture = RgbImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgb([20, 20, 20])
        } else {
            Rgb([240, 240, 240])
        }
    });
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode(picture.as_raw(), width, height, ExtendedColorType::Rgb8)
        .unwrap();

    let turned = straighten(&jpeg, &Corners::WHOLE.turned(1)).unwrap();
    let turned = image::load_from_memory(&turned).unwrap().into_rgb8();
    assert_eq!((turned.width(), turned.height()), (height, width));
    assert!(turned.get_pixel(50, 10).0[0] < 50, "top is dark");
    assert!(turned.get_pixel(50, 50).0[0] > 200, "bottom is light");

    let half = straighten(&jpeg, &Corners::WHOLE.turned(2)).unwrap();
    let half = image::load_from_memory(&half).unwrap().into_rgb8();
    assert_eq!((half.width(), half.height()), (width, height));
    assert!(half.get_pixel(10, 50).0[0] > 200, "left is light");
    assert!(half.get_pixel(50, 50).0[0] < 50, "right is dark");

    assert_eq!(Corners::WHOLE.turned(4), Corners::WHOLE);
    assert!(
        Corners::new(Corners::WHOLE.turned(3).0).is_ok(),
        "still clockwise"
    );
}

#[test]
fn a_live_frame_is_straightened_into_the_pages_own_shape() {
    use scannerd::straighten::straighten_luma;
    // A 320×240 frame of a crop twice as wide as tall, so stretched: a dark
    // page in the middle half of it, on a light table.
    let (w, h) = (320usize, 240usize);
    let frame: Vec<u8> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if (80..240).contains(&x) && (60..180).contains(&y) {
                20
            } else {
                230
            }
        })
        .collect();
    let page = Corners::new([(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)]).unwrap();
    let picture = straighten_luma(&frame, w, h, 2.0, &page, 320, 320);
    // Half the crop's width by half its height, in a crop twice as wide: a
    // page twice as wide as tall, all of it dark.
    assert_eq!((picture.width, picture.height), (320, 160));
    assert!(picture.luma.iter().all(|&value| value == 20));

    // Turned a quarter, it is as tall as it was wide.
    let turned = straighten_luma(&frame, w, h, 2.0, &page.turned(1), 320, 320);
    assert_eq!((turned.width, turned.height), (160, 320));
}

/// Packed RGB: a grey table with a white quadrilateral page, `page` being its
/// corners as fractions.
fn table_with_page(w: usize, h: usize, page: [(f64, f64); 4]) -> Vec<u8> {
    let inside = |x: f64, y: f64| {
        (0..4).all(|i| {
            let (a, b) = (page[i], page[(i + 1) % 4]);
            (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0) >= 0.0
        })
    };
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let white = inside((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
            let v = if white { 235 } else { 110 };
            rgb.extend([v, v, v]);
        }
    }
    rgb
}

#[test]
fn a_letter_lying_off_the_corners_is_straightened_onto_itself() {
    use scannerd::straighten::trim_to_page;
    let (w, h) = (600usize, 800usize);
    // Shifted right and down, and a little turned: table left, top and in wedges.
    let page = [(0.18, 0.10), (0.97, 0.14), (0.93, 0.98), (0.14, 0.94)];
    let (out, ow, oh) = trim_to_page(table_with_page(w, h, page), w as u32, h as u32);
    assert!(ow < w as u32 && oh < h as u32, "{ow}×{oh}");
    // Nearly all of what is left is page: the rest is the thin edge kept on
    // purpose, 1.5% a side. Before, a third of it was table.
    let white = out.chunks_exact(3).filter(|p| p[0] > 200).count();
    assert!(
        white * 100 >= (ow * oh) as usize * 90,
        "{white} of {}",
        ow * oh
    );
}

#[test]
fn without_a_clear_page_the_photograph_is_left_alone() {
    use scannerd::straighten::trim_to_page;
    let (w, h) = (300usize, 400usize);
    // A small bright thing: not trusted to be the page.
    let small = [(0.4, 0.4), (0.6, 0.4), (0.6, 0.6), (0.4, 0.6)];
    let rgb = table_with_page(w, h, small);
    let (out, ow, oh) = trim_to_page(rgb.clone(), w as u32, h as u32);
    assert_eq!((ow, oh), (w as u32, h as u32));
    assert_eq!(out, rgb);
    // A page that already fills it: nothing to do.
    let full = table_with_page(w, h, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    let (_, ow, oh) = trim_to_page(full, w as u32, h as u32);
    assert_eq!((ow, oh), (w as u32, h as u32));
}

#[test]
fn paper_smaller_than_a_page_is_an_envelope() {
    use scannerd::straighten::is_envelope;
    // On corners a page fills: C5 about 0.6 of it, DL about 0.4.
    assert!(!is_envelope(Some(0.95), 1.0));
    assert!(!is_envelope(Some(0.8), 1.0));
    assert!(is_envelope(Some(0.6), 1.0));
    assert!(is_envelope(Some(0.39), 1.0));
    // Nothing found, or a speck: not an envelope either.
    assert!(!is_envelope(None, 1.0));
    assert!(!is_envelope(Some(0.05), 1.0));
    // On corners set generously, where a page covers 0.7 of them: a page is
    // still a page, and a C5 envelope still an envelope.
    assert!(!is_envelope(Some(0.68), 0.7));
    assert!(is_envelope(Some(0.42), 0.7));
}

#[test]
fn a_photograph_says_how_much_of_it_the_paper_covers() {
    use scannerd::straighten::straighten_trimmed;
    let (w, h) = (400usize, 560usize);
    let jpeg = |page: [(f64, f64); 4]| {
        let rgb = table_with_page(w, h, page);
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90)
            .encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        out
    };
    // A DL envelope lying across the middle of an A4 page's corners.
    let envelope = [(0.1, 0.3), (0.9, 0.3), (0.9, 0.78), (0.1, 0.78)];
    let straight = straighten_trimmed(&jpeg(envelope), &Corners::WHOLE).unwrap();
    let covered = straight.covered.expect("found");
    assert!((covered - 0.38).abs() < 0.05, "{covered}");
    assert!(scannerd::straighten::is_envelope(straight.covered, 1.0));

    let page = [(0.03, 0.02), (0.98, 0.03), (0.97, 0.98), (0.02, 0.97)];
    let straight = straighten_trimmed(&jpeg(page), &Corners::WHOLE).unwrap();
    assert!(
        !scannerd::straighten::is_envelope(straight.covered, 1.0),
        "{:?}",
        straight.covered
    );
}
