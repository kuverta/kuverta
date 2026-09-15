//! Finding the page in a picture of the whole view.

use scannerd::locate::{find_page, Area};

const W: usize = 320;
const H: usize = 240;

fn table() -> Vec<u8> {
    vec![90; W * H]
}

/// A sheet of paper from (x0, y0) to (x1, y1), with lines of dark text on it.
fn with_page(mut frame: Vec<u8>, (x0, y0): (usize, usize), (x1, y1): (usize, usize)) -> Vec<u8> {
    for y in y0..y1 {
        for x in x0..x1 {
            let text_line = (y - y0) % 8 == 4 && x > x0 + 8 && x + 8 < x1;
            frame[y * W + x] = if text_line { 30 } else { 225 };
        }
    }
    frame
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

#[test]
fn a_page_on_a_table_is_found_with_room_around_it() {
    let frame = with_page(table(), (100, 40), (220, 200));
    let area = find_page(&frame, W, H).expect("a page");

    // 120×160 pixels, plus 12% of that on each side.
    let (margin_x, margin_y) = (120.0 * 0.12, 160.0 * 0.12);
    assert!(close(area.x, (100.0 - margin_x) / W as f32), "{area:?}");
    assert!(close(area.y, (40.0 - margin_y) / H as f32), "{area:?}");
    assert!(
        close(area.width, (120.0 + 2.0 * margin_x) / W as f32),
        "{area:?}"
    );
    assert!(
        close(area.height, (160.0 + 2.0 * margin_y) / H as f32),
        "{area:?}"
    );
}

#[test]
fn the_text_on_a_page_does_not_split_it() {
    // Dense text: every other row dark across the middle of the page.
    let mut frame = with_page(table(), (60, 30), (260, 210));
    for y in (40..200).step_by(2) {
        for x in 70..250 {
            frame[y * W + x] = 25;
        }
    }
    let area = find_page(&frame, W, H).expect("one page, not many lines");
    assert!(area.width > 0.6, "{area:?}");
}

#[test]
fn a_page_at_the_edge_is_kept_inside_the_picture() {
    let frame = with_page(table(), (0, 100), (90, 240));
    let area = find_page(&frame, W, H).expect("a page");
    assert_eq!(area.x, 0.0);
    assert!(area.y + area.height <= 1.0 + f32::EPSILON, "{area:?}");
}

#[test]
fn of_two_bright_things_the_larger_is_the_page() {
    let frame = with_page(
        with_page(table(), (10, 10), (40, 40)),
        (150, 60),
        (300, 220),
    );
    let area = find_page(&frame, W, H).expect("a page");
    assert!(area.x > 0.35, "the small one was chosen: {area:?}");
}

#[test]
fn nothing_page_sized_is_no_suggestion() {
    assert_eq!(find_page(&table(), W, H), None, "an empty table");

    let mug = with_page(table(), (150, 110), (160, 120));
    assert_eq!(find_page(&mug, W, H), None, "a reflection or a mug");

    let washed_out = vec![240; W * H];
    assert_eq!(find_page(&washed_out, W, H), None, "a white picture");

    assert_eq!(find_page(&[1, 2, 3], W, H), None, "too few pixels");
}

#[test]
fn an_area_is_written_as_the_crop_setting_takes_it() {
    let area = Area {
        x: 0.25,
        y: 0.1,
        width: 0.5,
        height: 0.8,
    };
    assert_eq!(area.to_roi(), "0.250,0.100,0.500,0.800");
    assert!(scannerd::settings::valid_roi(&area.to_roi()).is_ok());
}
