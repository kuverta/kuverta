//! The button, from the bytes the kernel hands out.

use scannerd::button::{presses, Button, EVENT_SIZE, KEY_ENTER, TIMEVAL};

/// One `struct input_event` as 64-bit Linux writes it.
fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut bytes = [0u8; EVENT_SIZE];
    bytes[..8].copy_from_slice(&1_789_400_000u64.to_le_bytes());
    bytes[TIMEVAL..TIMEVAL + 2].copy_from_slice(&kind.to_le_bytes());
    bytes[TIMEVAL + 2..TIMEVAL + 4].copy_from_slice(&code.to_le_bytes());
    bytes[TIMEVAL + 4..TIMEVAL + 8].copy_from_slice(&value.to_le_bytes());
    bytes
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const KEY_A: u16 = 30;

#[test]
fn a_press_is_counted_and_the_release_and_repeats_are_not() {
    // What holding the button for a moment actually produces: press, a few
    // auto-repeats, release, each followed by a sync event.
    let events: Vec<u8> = [
        event(EV_KEY, KEY_ENTER, 1),
        event(EV_SYN, 0, 0),
        event(EV_KEY, KEY_ENTER, 2),
        event(EV_KEY, KEY_ENTER, 2),
        event(EV_KEY, KEY_ENTER, 0),
        event(EV_SYN, 0, 0),
    ]
    .concat();
    assert_eq!(presses(&events, KEY_ENTER), 1);
}

#[test]
fn only_the_configured_key_counts() {
    let events = [event(EV_KEY, KEY_A, 1), event(EV_KEY, KEY_ENTER, 1)].concat();
    assert_eq!(presses(&events, KEY_ENTER), 1);
    assert_eq!(presses(&events, KEY_A), 1);
}

#[test]
fn a_trailing_partial_event_is_ignored_rather_than_misread() {
    let mut events = event(EV_KEY, KEY_ENTER, 1).to_vec();
    events.extend_from_slice(&event(EV_KEY, KEY_ENTER, 1)[..10]);
    assert_eq!(presses(&events, KEY_ENTER), 1);
}

#[test]
fn presses_between_two_looks_count_once() {
    // A double tap closes the letter; it must not also close the next one.
    let (press, button) = Button::channel();
    assert!(!button.pressed());

    press.send(()).unwrap();
    press.send(()).unwrap();
    assert!(button.pressed());
    assert!(!button.pressed());
}
