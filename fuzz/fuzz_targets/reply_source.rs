//! Answering a received message: its headers are read, quoted and written
//! into a new message. A hostile one may make the build fail; it must not
//! make it panic.
#![no_main]

use core_smtp::{Draft, Mailbox, ReplyMode, ReplySource};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(source) = ReplySource::from_rfc822(data) else {
        return;
    };
    let me = || Mailbox::new("erika@example.org");
    for draft in [
        Draft::reply(me(), &source, ReplyMode::Sender),
        Draft::reply(me(), &source, ReplyMode::All),
        Draft::forward(me(), &source),
    ] {
        let _ = draft.build();
    }
});
