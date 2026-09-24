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
//! **Nothing executable survives** — but only where that can be said about
//! any input at all, which is narrower than it first looks. `&lt;script&gt;`
//! unescapes to the characters `<script>`, correctly: that is what the sender
//! wrote, as text, and the window renders it as text. So a message quoting
//! HTML puts `<script` in the output with nothing wrong having happened, and
//! an assertion that simply forbids the substring fails on ordinary mail —
//! which is how this target failed in CI the first time it ran against the
//! seed corpus. The check therefore applies to inputs with no character
//! reference in them, where nothing can put those bytes there but the reader
//! carrying markup through. That a script's *content* is dropped, and an
//! event handler with it, is a claim about particular documents, and it is
//! made about particular documents in `crates/core-rpc/tests/html.rs`.
#![no_main]

use core_rpc::html::{self, Refused};
use libfuzzer_sys::fuzz_target;

/// Mirrors `MOST_TEXT`, which is private. A copy on purpose: if the reader's
/// own limit is raised without meaning to, this fails rather than following
/// it quietly.
const MOST_TEXT: usize = 1 << 19;

/// Bytes that can only have come from the reader carrying markup through,
/// rather than from a sender writing about it.
const NOT_FROM_A_READER: [&str; 12] = [
    "<script",
    "</script",
    "javascript:",
    "vbscript:",
    "onerror=",
    "onclick=",
    "onload=",
    "<iframe",
    "<object",
    "<embed",
    "srcdoc=",
    "data:text/html",
];

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

    // Nothing a mail window could be talked into running or fetching — asked
    // only of messages with no character reference in them, for the reason in
    // the note at the top. Lowercased first: `JaVaScRiPt:` is the oldest
    // trick there is.
    if !html.contains('&') {
        let lowered = text.to_lowercase();
        for forbidden in NOT_FROM_A_READER {
            assert!(
                !lowered.contains(forbidden),
                "`{forbidden}` survived into the text: {text:?}"
            );
        }
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
