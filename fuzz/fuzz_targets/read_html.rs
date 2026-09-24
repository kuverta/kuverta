//! The HTML a letter arrives in, read as text.
//!
//! This is the parser with the widest mouth in the program: anybody who can
//! send mail can hand it a megabyte of anything. It is also the newest, and
//! it was written under one rule — prefer refusing to guessing — so what is
//! checked here is that rule, not that the text comes out pretty.
//!
//! Four things must hold for every input there is:
//!
//! **It returns.** No panic, no arithmetic that overflows (the fuzz profile
//! turns those into panics on purpose), and no recursion to run the stack
//! out — the reader counts depth rather than nesting calls, and this is what
//! says so for inputs nobody would think to write down.
//!
//! **What comes out is bounded** by the reader's own limit and the length of
//! the input, so a short message cannot make a long one. A parser whose
//! output grows faster than its input is a way to run a machine out of
//! memory with one message.
//!
//! **A refusal is a refusal**, not a half-read message. That is the whole
//! design: a truncated conversion reads exactly like a good one, so there is
//! no such thing as a partial answer here.
//!
//! **Nothing about executable content is asserted here**, and getting to
//! that took three wrong answers, each narrower than the last.
//!
//! Forbidding `<script` outright failed on the first real mail it saw:
//! `&lt;script&gt;` unescapes to the characters `<script>`, correctly, and a
//! message quoting HTML is not a message carrying it. Excluding inputs with
//! a character reference in them failed next, on `javascript:` in an href.
//! Excluding bytes the sender wrote as well failed on this:
//!
//! ```text
//! From: Tom Fisher!vbscript<>:m
//! ```
//!
//! `<>` is an empty tag. Dropping it leaves `vbscript` next to `:m`, and the
//! text says `vbscript:` where the sender's bytes never did. That is not a
//! defect — it is the whole job. `<b>java</b>script:` has to read as
//! `javascript:`, because the words either side of markup are one word.
//!
//! So there is no form of this an input-only check can take. Joining text
//! across removed markup can produce any bytes at all, and a fuzzer cannot
//! see the difference between that and a parser carrying something through.
//! Whether a script's content is dropped, whether an event handler goes with
//! it, and whether an empty link shows an address worth showing are claims
//! about particular documents, where what the sender wrote and what the
//! reader did are both known. They are made, and kept, in
//! `crates/core-rpc/tests/html.rs`.
//!
//! One thing follows for callers rather than for this file: the text that
//! comes out is text, and it can hold `javascript:` whatever the markup was.
//! Anything that ever turns it back into links or markup has to say so for
//! itself. `to_text` promises text, not safe markup.
#![no_main]

use core_rpc::html::{self, Refused};
use libfuzzer_sys::fuzz_target;

/// Mirrors `MOST_TEXT`, which is private. A copy on purpose: if the reader's
/// own limit is raised without meaning to, this fails rather than following
/// it quietly.
const MOST_TEXT: usize = 1 << 19;

fuzz_target!(|html: &str| {
    // Cheap and total: it must never disagree with itself about the same
    // input, and it must never panic on one.
    let badly = html::looks_converted_badly(html);
    assert_eq!(badly, html::looks_converted_badly(html));

    let Ok(text) = html::to_text(html) else {
        // Every refusal is one of the four the caller knows how to handle;
        // the match is exhaustive, so a new one added without a thought about
        // the caller stops compiling here.
        return;
    };

    // The limit is checked once round the loop, so the run being read when it
    // is passed is finished before anyone notices — and a run is at most the
    // whole input. Generous on purpose: a bound that is tight but wrong fails
    // on somebody's mail rather than on a bug, which is the mistake this line
    // used to make.
    assert!(
        text.len() <= MOST_TEXT + html.len(),
        "{} bytes of text out of {} bytes of HTML",
        text.len(),
        html.len()
    );

    // Reading it again must give the same text: the reader keeps no state
    // between messages, and a reader whose answer drifts is one that cannot
    // be reasoned about.
    assert_eq!(Ok(text), html::to_text(html));
});

/// Never called: it makes the compiler check that `Refused` still has exactly
/// the four cases the caller handles, so a fifth cannot be added without
/// somebody reading this file.
#[allow(dead_code)]
fn every_refusal_is_accounted_for(why: Refused) {
    match why {
        Refused::TooBig | Refused::TooDeep | Refused::Unterminated | Refused::Empty => {}
    }
}
