//! The `text/calendar` part of a message, read.
//!
//! Anyone who can send mail can attach one of these, so the reader is on the
//! same footing as the HTML reader: it may refuse, and it may not panic.
//! Three things are checked for every input there is.
//!
//! **It returns, and says the same thing twice.** The parser keeps no state
//! between parts, so a second read of the same bytes is the same invitation.
//! A reader whose answer drifts cannot be reasoned about.
//!
//! **What comes out is bounded by what went in.** Every field is a piece of
//! the input, and the number of people on an invitation cannot exceed the
//! number of lines there was room for. A parser whose output grows faster
//! than its input is a way to run a machine out of memory with one message.
//!
//! **No instant is invented.** This is the property worth fuzzing rather
//! than asserting in one test: `When::at` is an epoch second, and it is
//! filled only where the sender gave a time that needs no zone database.
//! A `TZID` names a wall clock, and a date is not an instant at all — so if
//! either is set, `at` must be empty. A card showing a confident wrong hour
//! is worse than one showing none, and this is the line between them.
//!
//! One more, about who may be answered: an invitation from a stranger is
//! never answerable, whatever the invitation says. The sender controls every
//! byte here, so the check belongs where the bytes are arbitrary.
#![no_main]

use core_offers::{
    invites::{self, When},
    Offer, Origin, Standing, Withheld,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let Some(invite) = invites::read(text) else {
        return;
    };

    assert_eq!(
        Some(&invite),
        invites::read(text).as_ref(),
        "the same part read twice is the same invitation"
    );

    for field in [&invite.uid, &invite.summary, &invite.description, &invite.location] {
        if let Some(value) = field {
            assert!(
                value.len() <= text.len(),
                "{} bytes out of {} bytes of part",
                value.len(),
                text.len()
            );
        }
    }
    assert!(
        invite.attendees.len() <= text.len(),
        "{} people out of {} bytes of part",
        invite.attendees.len(),
        text.len()
    );

    for when in [invite.begins.as_ref(), invite.ends.as_ref()].into_iter().flatten() {
        no_instant_is_invented(when);
        assert!(when.written.len() <= text.len() + 2, "a date is not longer than the line it came from");
    }

    // The sender wrote all of this; none of it can earn an answer on its own.
    let stranger = Offer::from_invite(Origin::Message(1), invite.clone(), Standing::Stranger);
    assert!(!stranger.answerable());
    assert!(!stranger.key.is_empty(), "a key is what the ledger goes by");

    // From a correspondent, only what asks a question may be answered.
    let known = Offer::from_invite(Origin::Message(1), invite.clone(), Standing::Known);
    assert_eq!(
        known.answerable(),
        invite.intent.asks_an_answer(),
        "a published event and a cancellation ask nothing"
    );
    assert_eq!(known.key, stranger.key, "the key is the thing's, not the sender's");
});

/// `at` is an instant, and an instant is only ever claimed where the sender
/// gave one.
fn no_instant_is_invented(when: &When) {
    if when.at.is_some() {
        assert!(
            when.zone.is_none(),
            "a TZID names a wall clock, not an instant: {when:?}"
        );
        assert!(!when.all_day, "a whole day is not an instant: {when:?}");
    }
}

/// Never called: it makes the compiler check that the reasons an answer is
/// withheld are still the two the card knows how to explain, so a third
/// cannot be added without somebody reading this file.
#[allow(dead_code)]
fn every_reason_is_accounted_for(why: Withheld) {
    match why {
        Withheld::Stranger | Withheld::NothingAsked => {}
    }
}
