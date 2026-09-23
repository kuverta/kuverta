//! The touchscreen: readings into taps, and taps into rows and buttons.

use scannerd::picture::Picture;
use scannerd::touch::{hit, Rows, Taps, PRESSED, X};

#[test]
fn the_rig_panels_readings_land_on_the_rows_they_were_measured_at() {
    // Measured on the rig: 560 at row 30 and 3644 at row 290 of 320.
    assert!((Rows::RIG.row(560, 320) - 30).abs() <= 2);
    assert!((Rows::RIG.row(3644, 320) - 290).abs() <= 2);
    assert_eq!(Rows::RIG.row(0, 320), 0);
    assert_eq!(Rows::RIG.row(4095, 320), 319);
}

#[test]
fn rows_are_read_from_the_env_file() {
    let rows = Rows::parse("x:204:4000").unwrap();
    assert_eq!(rows, Rows::RIG);
    assert_eq!(rows.command, X);
    // Upside down works too.
    let flipped = Rows::parse("y:4000:204").unwrap();
    assert!(flipped.row(4000, 320) < 5);
    assert!(Rows::parse("z:1:2").is_err());
    assert!(Rows::parse("x:100:120").is_err());
}

#[test]
fn a_press_is_a_tap_where_the_finger_mostly_was() {
    let mut taps = Taps::default();
    let press = [
        (0, 0),
        (300, 900),
        (400, 1000),
        (420, 1010),
        (380, 990),
        (0, 4095),
    ];
    let mut tapped = press
        .iter()
        .filter_map(|&(pressure, position)| taps.reading(pressure, position))
        .collect::<Vec<_>>();
    tapped.extend(taps.reading(0, 4095));
    assert_eq!(tapped, vec![1000]);
}

#[test]
fn a_brush_of_the_film_is_not_a_tap() {
    let mut taps = Taps::default();
    let tapped: Vec<_> = [(PRESSED, 1000), (0, 0), (0, 0), (0, 0)]
        .iter()
        .filter_map(|&(pressure, position)| taps.reading(pressure, position))
        .collect();
    assert!(tapped.is_empty());
}

#[test]
fn one_light_reading_in_the_middle_of_a_press_does_not_end_it() {
    let mut taps = Taps::default();
    let readings = [
        (500, 1000),
        (500, 1000),
        (100, 3000),
        (500, 1000),
        (500, 1000),
        (0, 0),
        (0, 0),
    ];
    let tapped: Vec<_> = readings
        .iter()
        .filter_map(|&(pressure, position)| taps.reading(pressure, position))
        .collect();
    assert_eq!(tapped, vec![1000]);
}

#[test]
fn a_tap_falls_on_the_band_it_is_in_or_just_beside() {
    let bands = [(100, 150, 'a'), (160, 210, 'b')];
    assert_eq!(hit(&bands, 120), Some('a'));
    assert_eq!(hit(&bands, 153), Some('a'));
    assert_eq!(hit(&bands, 200), Some('b'));
    assert_eq!(hit(&bands, 40), None);
}

#[test]
fn a_picture_is_averaged_down_and_keeps_its_shape() {
    // A 4×2 picture of two grey levels, halved.
    let luma = [0, 100, 200, 200, 0, 100, 200, 200];
    let small = Picture::scaled(&luma, 4, 2, 2, 2);
    assert_eq!((small.width, small.height), (2, 1));
    assert_eq!(*small.luma, vec![50, 200]);
    // Never made larger.
    let same = Picture::scaled(&luma, 4, 2, 100, 100);
    assert_eq!((same.width, same.height), (4, 2));
}

#[test]
fn a_picture_turns_a_quarter_clockwise() {
    // 1 2
    // 3 4
    let square = Picture::scaled(&[1, 2, 3, 4], 2, 2, 2, 2);
    assert_eq!(*square.turned(90).luma, vec![3, 1, 4, 2]);
    assert_eq!(*square.turned(180).luma, vec![4, 3, 2, 1]);
    assert_eq!(*square.turned(270).luma, vec![2, 4, 1, 3]);
    let wide = Picture::scaled(&[1, 2, 3, 4, 5, 6], 3, 2, 3, 2);
    let tall = wide.turned(90);
    assert_eq!((tall.width, tall.height), (2, 3));
}
