//! The page detector, against frames no camera took.
//!
//! Detection is a fraction of changed pixels over a luma plane, which means it
//! can be reasoned about here rather than guessed at on a Pi. The behaviour
//! that matters is not "does it see paper" — that is a threshold — but the
//! transitions: never capture while a hand is still in shot, and never capture
//! the same page twice.

use scannerd::detect::{changed_fraction, paper_like, Detector, State, Step, Thresholds};

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

#[test]
fn a_change_of_exposure_alone_is_not_a_change() {
    // The camera exposes every frame afresh. The same table a shade lighter is
    // the same table.
    let lighter: Vec<u8> = desk().iter().map(|p| p + 40).collect();
    assert_eq!(changed_fraction(&desk(), &lighter, 24), 0.0);

    // And a page is still a page when the exposure moves with it.
    let page_darker: Vec<u8> = desk_with_page(0.3)
        .iter()
        .map(|p| p.saturating_sub(30))
        .collect();
    assert!(changed_fraction(&desk(), &page_darker, 24) >= 0.18);
}

#[test]
fn taking_the_page_away_arms_again_when_the_camera_comes_back_lighter() {
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);
    for _ in 0..4 {
        detector.observe(&page);
    }
    assert_eq!(detector.state(), State::Spent);

    let lighter: Vec<u8> = desk().iter().map(|p| p + 35).collect();
    detector.observe(&lighter);
    assert_eq!(detector.state(), State::Waiting);
}

#[test]
fn a_table_that_never_looks_quite_as_it_did_arms_again_once_no_page_is_there() {
    // Found on the Pi: after each photograph the table had to look almost
    // exactly as learnt, it never did, and every page needed the table
    // learning again by hand.
    let mut detector = detector();
    detector.observe(&desk());
    let page = desk_with_page(0.4);
    for _ in 0..4 {
        detector.observe(&page);
    }

    // Something left behind across a tenth of the view: more than "the same
    // table", less than a page.
    let mut changed = desk();
    for pixel in changed.iter_mut().rev().take(PIXELS / 10) {
        *pixel = 20;
    }
    detector.observe(&changed);
    detector.observe(&changed);
    assert_eq!(
        detector.state(),
        State::Spent,
        "not at the first sight of it"
    );
    detector.observe(&changed);
    assert_eq!(detector.state(), State::Waiting);

    // Judged against the table as it is now, a page on it is still a page.
    let mut page_now = changed.clone();
    for pixel in page_now.iter_mut().take(PIXELS * 4 / 10) {
        *pixel = 235;
    }
    for _ in 0..3 {
        detector.observe(&page_now);
    }
    assert_eq!(detector.observe(&page_now), Step::Capture);
}

#[test]
fn the_last_measurements_are_kept_for_the_setup_page() {
    let mut detector = detector();
    assert_eq!(detector.last_measure(), None);

    detector.observe(&desk());
    detector.observe(&desk_with_page(0.4));

    let (table, movement) = detector.last_measure().unwrap();
    assert!(table >= 0.18 && movement >= 0.18, "{table} {movement}");
}

// -- the next page, laid on top or turned over --------------------------------------

use scannerd::detect::page_changed;

/// A page on the desk, its top left at (`x`, `y`), with lines of text on it
/// in a pattern chosen by `layout` — or none, for the back of a page.
fn page_at(x: usize, y: usize, layout: Option<usize>) -> Vec<u8> {
    let mut frame = desk();
    for row in y..(y + 190).min(H) {
        for column in x..(x + 230).min(W) {
            let (px, py) = (column - x, row - y);
            let text = layout.is_some_and(|layout| {
                let line = (py + layout * 7) % 14 < 5;
                let word = (px + layout * 11) % 23 < 16;
                py > 15 && py < 175 && px > 15 && px < 215 && line && word
            });
            frame[row * W + column] = if text { 90 } else { 235 };
        }
    }
    frame
}

/// Frames of a hand over the page.
fn hand(over: &[u8]) -> Vec<Vec<u8>> {
    vec![jittered(over, 0.4), jittered(over, 0.3)]
}

/// Photographs the first page, then runs the rest and says how many more
/// photographs were taken.
fn after_the_first(first: &[u8], then: Vec<Vec<u8>>) -> usize {
    let mut detector = detector();
    detector.observe(&desk());
    let captured = (0..5)
        .map(|_| detector.observe(first))
        .filter(|step| *step == Step::Capture)
        .count();
    assert_eq!(captured, 1, "the first page");
    then.iter()
        .map(|frame| detector.observe(frame))
        .filter(|step| *step == Step::Capture)
        .count()
}

#[test]
fn a_page_turned_over_where_it_lies_is_photographed() {
    let front = page_at(40, 20, Some(0));
    let back = page_at(40, 20, None);
    let mut then = hand(&front);
    then.extend(std::iter::repeat_n(back, 5));
    assert_eq!(after_the_first(&front, then), 1);
}

#[test]
fn the_next_page_laid_on_top_is_photographed() {
    let first = page_at(40, 20, Some(0));
    let second = page_at(44, 22, Some(1));
    let mut then = hand(&first);
    then.extend(std::iter::repeat_n(second, 5));
    assert_eq!(after_the_first(&first, then), 1);
}

#[test]
fn a_page_nudged_is_the_same_page() {
    let page = page_at(40, 20, Some(0));
    let nudged = page_at(45, 23, Some(0));
    let mut then = hand(&page);
    then.extend(std::iter::repeat_n(nudged, 6));
    assert_eq!(after_the_first(&page, then), 0);
}

#[test]
fn a_hand_passing_over_the_page_takes_nothing_new() {
    let page = page_at(40, 20, Some(0));
    let mut then = hand(&page);
    then.extend(std::iter::repeat_n(page.clone(), 6));
    then.extend(hand(&page));
    then.extend(std::iter::repeat_n(page, 6));
    assert_eq!(after_the_first(&page_at(40, 20, Some(0)), then), 0);
}

/// A hand resting on `page`, reaching in from the bottom edge: skin, darker
/// than paper and than a line of print averaged over a block.
fn with_hand(page: &[u8]) -> Vec<u8> {
    let mut frame = page.to_vec();
    for row in 70..H {
        for column in 110..230 {
            frame[row * W + column] = 150;
        }
    }
    frame
}

/// Runs `frames` after the first page, counting photographs and pages taken
/// back.
fn steps_after_the_first(first: &[u8], frames: Vec<Vec<u8>>) -> (usize, usize) {
    let mut detector = detector();
    detector.observe(&desk());
    for _ in 0..5 {
        detector.observe(first);
    }
    let steps: Vec<Step> = frames.iter().map(|frame| detector.observe(frame)).collect();
    (
        steps.iter().filter(|step| **step == Step::Capture).count(),
        steps.iter().filter(|step| **step == Step::Retract).count(),
    )
}

#[test]
fn a_hand_left_resting_on_a_page_is_taken_back_once_it_goes() {
    // On the rig: the back of a page photographed, then photographed again
    // with a hand pressing it flat, then a third time once the hand had gone
    // — the last differed from the hand's photograph as much as another page
    // does. The hand's photograph is taken back, and nothing new is taken.
    let front = page_at(40, 20, Some(0));
    let back = page_at(40, 20, Some(1));
    let pressed = with_hand(&back);
    let mut frames = hand(&front);
    frames.extend(std::iter::repeat_n(back.clone(), 5));
    frames.extend(hand(&back));
    frames.extend(std::iter::repeat_n(pressed.clone(), 5));
    let (captured, _) = steps_after_the_first(&front, frames.clone());
    assert_eq!(captured, 2, "the back, and the back under a hand");

    frames.extend(hand(&pressed));
    frames.extend(std::iter::repeat_n(back, 6));
    assert_eq!(steps_after_the_first(&front, frames), (2, 1));
}

#[test]
fn a_page_turned_back_up_keeps_both_sides_and_takes_nothing_new() {
    // Turned over and back again: the back was a page of its own, and must
    // not be taken for a hand's photograph just because the front came back.
    let front = page_at(40, 20, Some(0));
    for back in [page_at(40, 20, Some(1)), page_at(40, 20, None)] {
        let mut frames = hand(&front);
        frames.extend(std::iter::repeat_n(back.clone(), 5));
        frames.extend(hand(&back));
        frames.extend(std::iter::repeat_n(front.clone(), 6));
        assert_eq!(steps_after_the_first(&front, frames), (1, 0));
    }
}

#[test]
fn a_page_left_lying_is_untouched_until_a_hand_comes() {
    let page = page_at(40, 20, Some(0));
    let mut detector = detector();
    detector.observe(&desk());
    for _ in 0..5 {
        detector.observe(&page);
    }
    assert_eq!(detector.state(), State::Spent);
    detector.observe(&page);
    assert!(detector.lying_untouched());
    for frame in hand(&page) {
        detector.observe(&frame);
    }
    assert!(!detector.lying_untouched(), "a hand is over it");
}

#[test]
fn pages_are_told_apart_in_blocks_and_nudges_are_not_changes() {
    let page = page_at(40, 20, Some(0));
    assert_eq!(page_changed(&page, &page, W), 0.0);
    assert!(page_changed(&page, &page_at(46, 24, Some(0)), W) < 0.22);
    assert!(page_changed(&page, &page_at(40, 20, None), W) >= 0.22);
    assert!(page_changed(&page, &page_at(40, 20, Some(1)), W) >= 0.22);
    // Frames it cannot compare are no change, rather than a panic.
    assert_eq!(page_changed(&page, &page[..100], W), 0.0);
}

// -- the light over the table, which is not paper --------------------------------

/// The desk as the sun goes down: a wash over the whole frame, and the far
/// side of it deeper in shadow than the near side.
fn evening(shade: i16) -> Vec<u8> {
    let mut frame = desk();
    for (i, pixel) in frame.iter_mut().enumerate() {
        let across = (i % W) as f32 / W as f32;
        let taken = shade as f32 * (0.55 + 0.45 * across);
        *pixel = (*pixel as f32 - taken).clamp(0.0, 255.0) as u8;
    }
    frame
}

/// A window frame's shadow: a band across the frame, sharp-edged, over as much
/// of it as a page would cover.
fn shadow_band(shade: i16) -> Vec<u8> {
    let mut frame = desk();
    for y in H / 5..H * 3 / 4 {
        for x in 0..W {
            let pixel = &mut frame[y * W + x];
            *pixel = (*pixel as i16 - shade).clamp(0, 255) as u8;
        }
    }
    frame
}

#[test]
fn paper_is_one_lump_and_the_evening_is_a_wash() {
    let page = paper_like(&desk(), &desk_with_page(0.5), W);
    assert!(
        page.share > 0.4,
        "a page covers the blocks it lies on: {page:?}"
    );
    assert!(page.fill > 0.9, "and they sit together: {page:?}");

    // The sun going down changes the whole frame, and darkens it: against a
    // dark table that is not what paper does at all.
    let dusk = paper_like(&desk(), &evening(30), W);
    assert!(dusk.share < 0.06, "nothing lies there: {dusk:?}");

    // A shadow with an edge to it is compact enough, but still the wrong way.
    let band = paper_like(&desk(), &shadow_band(40), W);
    assert!(band.share < 0.06, "a shadow is not paper: {band:?}");
}

#[test]
fn a_page_the_camera_darkened_is_still_a_page() {
    // The camera sets its exposure for every frame: a white page in view makes
    // it pull everything down, so the page reads far darker than the paper on
    // the table looks to a person, and the table around it darker still. The
    // first version of this asked for paper to reach a level and stopped
    // seeing pages altogether — paper is *the pale thing in the frame*, not a
    // number.
    let mut frame: Vec<u8> = (0..PIXELS).map(|i| 65 + (i % 3) as u8).collect();
    for pixel in frame.iter_mut().take(PIXELS / 2) {
        *pixel = 120;
    }
    let seen = paper_like(&desk(), &frame, W);
    assert!(seen.share > 0.4, "the page is still a page: {seen:?}");
    assert!(seen.fill > 0.9, "{seen:?}");

    let mut detector = detector();
    detector.learn_empty(&desk());
    let mut taken = false;
    for _ in 0..6 {
        taken |= detector.observe(&frame) == Step::Capture;
    }
    assert!(taken, "and it is photographed");
}

#[test]
fn the_sun_going_down_is_not_photographed() {
    let mut detector = detector();
    detector.learn_empty(&desk());

    // Frame after frame of a room getting darker, each still against the one
    // before — which is exactly what a page settling looks like by every other
    // measure.
    for shade in 1..=40 {
        for _ in 0..3 {
            assert_eq!(
                detector.observe(&evening(shade)),
                Step::Wait,
                "photographed the evening at {shade}"
            );
        }
    }
    // And a real page, put down in that light, is still seen.
    let mut page = evening(40);
    for pixel in page.iter_mut().take(PIXELS / 2) {
        *pixel = 225;
    }
    let mut taken = false;
    for _ in 0..6 {
        taken |= detector.observe(&page) == Step::Capture;
    }
    assert!(taken, "a page in the new light is still a page");
}

#[test]
fn a_page_lying_at_an_angle_is_still_one_lump() {
    // How pages actually lie on the rig: turned, corner to corner, so the
    // upright box round them is the whole frame and a tilted rectangle fills
    // only about half of it. Measured there: 0.45 to 0.50. The threshold has
    // to sit well under that, or a page is refused for lying crooked.
    let mut frame = desk();
    let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
    let angle: f32 = 0.5; // about 29°
    let (half_w, half_h) = (W as f32 * 0.32, H as f32 * 0.40);
    for y in 0..H {
        for x in 0..W {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let along = dx * angle.cos() + dy * angle.sin();
            let across = -dx * angle.sin() + dy * angle.cos();
            if along.abs() < half_w && across.abs() < half_h {
                frame[y * W + x] = 225;
            }
        }
    }
    let seen = paper_like(&desk(), &frame, W);
    assert!(
        seen.share > 0.2,
        "a turned page is most of the frame: {seen:?}"
    );
    assert!(
        seen.fill > Thresholds::default().paper_fill,
        "and it is not refused for lying crooked: {seen:?} against {}",
        Thresholds::default().paper_fill
    );

    let mut detector = detector();
    detector.learn_empty(&desk());
    let mut taken = false;
    for _ in 0..6 {
        taken |= detector.observe(&frame) == Step::Capture;
    }
    assert!(taken, "and it is photographed");
}

#[test]
fn one_lit_block_in_a_corner_does_not_make_a_page_look_scattered() {
    // On the rig a real page measured share 0.47 and fill 0.47 — the same
    // number, which is the giveaway: the box was the whole frame, because a
    // stray block at the far edge stretched it there. Measured on the lump
    // itself, a page is a page.
    let mut frame = desk();
    // A page over the middle third, tidy and solid.
    for y in H / 3..2 * H / 3 {
        for x in W / 4..3 * W / 4 {
            frame[y * W + x] = 225;
        }
    }
    // And one bright speck in the far corner — a cup's rim catching the light.
    for y in 0..20 {
        for x in W - 20..W {
            frame[y * W + x] = 200;
        }
    }
    let seen = paper_like(&desk(), &frame, W);
    assert!(seen.fill > 0.85, "the page fills its own box: {seen:?}");
    assert!(seen.share > 0.1, "{seen:?}");
}

#[test]
fn a_page_refused_is_never_learnt_as_the_table() {
    // What broke the rig: a page was refused — the rule was too strict — and
    // then, because something had changed and it did not look like paper, the
    // page was learnt as the table. After that the page *was* the table and
    // nothing was ever seen again, not even after learning the empty table by
    // hand, because the next frame swallowed the next page the same way.
    //
    // Refusing a page is a threshold to argue about. Swallowing one is a rig
    // that has stopped working, so only a frame with nothing paper-like in it
    // at all may become the table.
    let mut strict = Detector::new(
        Thresholds {
            // As strict as the version that broke: nothing will be paper.
            paper_fill: 1.01,
            ..Thresholds::default()
        },
        3,
    );
    strict.learn_empty(&desk());
    let generation = strict.baseline_generation();

    for _ in 0..60 {
        assert_eq!(strict.observe(&desk_with_page(0.5)), Step::Wait);
    }
    assert_eq!(
        strict.baseline_generation(),
        generation,
        "the page was learnt as the table"
    );
    // And with the real thresholds it is simply photographed.
    let mut detector = detector();
    detector.learn_empty(&desk());
    let mut taken = false;
    for _ in 0..6 {
        taken |= detector.observe(&desk_with_page(0.5)) == Step::Capture;
    }
    assert!(taken);
}

#[test]
fn a_table_the_light_has_moved_over_is_learnt_again() {
    let mut detector = detector();
    detector.learn_empty(&desk());
    let generation = detector.baseline_generation();

    // A shadow that stays — the sun is where it is now.
    for _ in 0..40 {
        detector.observe(&shadow_band(60));
    }
    assert!(
        detector.baseline_generation() > generation,
        "the table it sees now is the table"
    );
    assert_eq!(detector.state(), State::Waiting);
    // Which means it is quiet about it from then on.
    let paper = detector.last_paper().expect("measured");
    assert!(paper.share < 0.06, "{paper:?}");
}
