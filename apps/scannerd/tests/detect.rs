//! The page detector, against frames no camera took.
//!
//! Detection is a fraction of changed pixels over a luma plane, which means it
//! can be reasoned about here rather than guessed at on a Pi. The behaviour
//! that matters is not "does it see paper" — that is a threshold — but the
//! transitions: never capture while a hand is still in shot, and never capture
//! the same page twice.

use scannerd::detect::{changed_fraction, Detector, State, Step, Thresholds};

const W: usize = 320;
const H: usize = 240;
const PIXELS: usize = W * H;

/// An empty desk: mid-grey, with a little sensor noise.
fn desk() -> Vec<u8> {
    (0..PIXELS).map(|i| 90 + (i % 3) as u8).collect()
}

/// A desk with a pale rectangle covering `fraction` of the frame.
fn desk_with_page(fraction: f32) -> Vec<u8> {
    let mut frame = desk();
    let covered = (PIXELS as f32 * fraction) as usize;
    for pixel in frame.iter_mut().take(covered) {
        *pixel = 235;
    }
    frame
}

/// The same scene, moved — a hand still in shot.
fn jittered(frame: &[u8], fraction: f32) -> Vec<u8> {
    let mut moved = frame.to_vec();
    let changed = (PIXELS as f32 * fraction) as usize;
    for (i, pixel) in moved.iter_mut().enumerate().take(changed) {
        *pixel = if i % 2 == 0 { 20 } else { 250 };
    }
    moved
}

fn detector() -> Detector {
    Detector::new(Thresholds::default(), 3)
}

#[test]
fn a_frame_compared_with_itself_has_changed_nothing() {
    assert_eq!(changed_fraction(&desk(), &desk(), 24), 0.0);
}

#[test]
fn frames_of_different_sizes_do_not_panic() {
    // A camera that changes resolution mid-run should stall the state machine,
    // not take the daemon down with it.
    assert_eq!(changed_fraction(&desk(), &[1, 2, 3], 24), 0.0);
    assert_eq!(changed_fraction(&[], &[], 24), 0.0);
}

#[test]
fn sensor_noise_is_not_a_page() {
    // Every pixel wobbles by a few counts between frames. If that read as
    // movement the detector would never settle; if it read as a page it would
    // photograph an empty desk all day.
    let noise: Vec<u8> = desk().iter().map(|p| p.wrapping_add(3)).collect();
    assert!(changed_fraction(&desk(), &noise, 24) < 0.01);
}

#[test]
fn the_first_frame_becomes_the_baseline() {
    let mut detector = detector();
    assert!(!detector.has_baseline());

    assert_eq!(detector.observe(&desk()), Step::Wait);

    assert!(detector.has_baseline());
    assert_eq!(detector.state(), State::Waiting);
}

#[test]
fn a_page_put_down_and_left_alone_is_captured() {
    let mut detector = detector();
    detector.observe(&desk());

    let page = desk_with_page(0.4);
    // Arrives — something is there, but one frame proves nothing.
    assert_eq!(detector.observe(&page), Step::Wait);
    // Three consecutive still frames, which is what `settle_frames` asks for.
    assert_eq!(detector.observe(&page), Step::Wait);
    assert_eq!(detector.observe(&page), Step::Wait);
    assert_eq!(detector.observe(&page), Step::Capture);
}

#[test]
fn a_hand_still_in_shot_is_not_photographed() {
    // The failure this exists to prevent: a page is in frame a moment before
    // the hand that put it there has left, and capturing then gets a thumb.
    let mut detector = detector();
    detector.observe(&desk());

    let page = desk_with_page(0.4);
    detector.observe(&page);

    // Moving, repeatedly. Never a capture, however long it goes on.
    for _ in 0..10 {
        assert_eq!(detector.observe(&jittered(&page, 0.2)), Step::Wait);
        assert_eq!(detector.observe(&page), Step::Wait);
    }
}

#[test]
fn a_pause_halfway_does_not_add_up_to_a_still_page() {
    // Stillness counts consecutively and resets on movement. Were it
    // cumulative, a hand that hesitated three times would trigger a capture.
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);
    detector.observe(&page);

    for _ in 0..5 {
        assert_eq!(detector.observe(&page), Step::Wait);
        assert_eq!(detector.observe(&jittered(&page, 0.2)), Step::Wait);
    }
}

#[test]
fn something_passing_over_the_desk_is_not_a_page() {
    let mut detector = detector();
    detector.observe(&desk());

    // Appears...
    detector.observe(&desk_with_page(0.4));
    assert!(matches!(detector.state(), State::Settling { .. }));

    // ...and is gone again before it ever settled.
    assert_eq!(detector.observe(&desk()), Step::Wait);
    assert_eq!(detector.state(), State::Waiting);
}

#[test]
fn one_page_is_photographed_once() {
    // The failure that turns one letter into forty: the page is still there
    // after the capture, and still still.
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);

    for _ in 0..4 {
        detector.observe(&page);
    }
    assert_eq!(detector.state(), State::Spent);

    for _ in 0..50 {
        assert_eq!(detector.observe(&page), Step::Wait);
    }
}

#[test]
fn clearing_the_desk_arms_it_again() {
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);
    for _ in 0..4 {
        detector.observe(&page);
    }
    assert_eq!(detector.state(), State::Spent);

    // Taken away.
    detector.observe(&desk());
    assert_eq!(detector.state(), State::Waiting);

    // And the next one is captured.
    let next = desk_with_page(0.45);
    detector.observe(&next);
    detector.observe(&next);
    detector.observe(&next);
    assert_eq!(detector.observe(&next), Step::Capture);
}

#[test]
fn the_baseline_is_relearnt_when_the_desk_is_seen_empty() {
    // Daylight moves across a desk over an afternoon. A baseline learnt once
    // at dawn would slowly read as a page.
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);
    for _ in 0..4 {
        detector.observe(&page);
    }

    // The desk, but brighter — later in the day, nothing on it.
    let brighter: Vec<u8> = desk().iter().map(|p| p.saturating_add(6)).collect();
    detector.observe(&brighter);
    assert_eq!(detector.state(), State::Waiting);

    // A page against the *new* baseline still reads as a page.
    let page_now: Vec<u8> = desk_with_page(0.4)
        .iter()
        .map(|p| p.saturating_add(6))
        .collect();
    detector.observe(&page_now);
    detector.observe(&page_now);
    detector.observe(&page_now);
    assert_eq!(detector.observe(&page_now), Step::Capture);
}

#[test]
fn a_small_change_is_not_a_page() {
    // A pen, a coffee cup, a shadow. Below the threshold, and nothing happens.
    let mut detector = detector();
    detector.observe(&desk());

    let small = desk_with_page(0.05);
    for _ in 0..10 {
        assert_eq!(detector.observe(&small), Step::Wait);
    }
    assert_eq!(detector.state(), State::Waiting);
}
