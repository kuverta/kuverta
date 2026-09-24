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
//! **What comes out is bounded**, so a short input cannot make a long output.
//! A parser whose output grows faster than its input is a way to run a
//! machine out of memory with one message.
//!
//! **A refusal is a refusal**, not a half-read message. That is the whole
//! design: a truncated conversion reads exactly like a good one, so there is
//! no such thing as a partial answer here.
//!
//! **Nothing executable survives.** Whatever the input, no `<script>` body,
//! event handler or `javascript:` URL may appear in the text handed to
//! somebody. `crates/core-rpc/tests/html.rs` checks that on written-out
//! examples; this checks it on inputs nobody wrote.
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

    // A little slack over the limit for the last line pushed before it is
    // noticed. Not unbounded: the point is that output is a function of the
    // limit and not of what the sender sent.
    assert!(
        text.len() <= MOST_TEXT + (1 << 16),
        "{} bytes of text out of {} bytes of HTML",
        text.len(),
        html.len()
    );

    // Nothing that a mail window could be talked into running or fetching.
    // Lowercased first: `JaVaScRiPt:` is the oldest trick there is.
    let lowered = text.to_lowercase();
    for forbidden in [
        "<script", "</script", "javascript:", "vbscript:", "onerror=", "onclick=", "onload=",
        "<iframe", "<object", "<embed", "srcdoc=", "data:text/html",
    ] {
        assert!(
            !lowered.contains(forbidden),
            "`{forbidden}` survived into the text: {text:?}"
        );
    }

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
