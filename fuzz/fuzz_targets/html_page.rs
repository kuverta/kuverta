//! The HTML a letter arrives in, made into a page the window's frame shows.
//!
//! The reader beside this one (`read_html`) makes text, and text cannot do
//! anything. This makes markup, and markup can — which is why the page is
//! held behind a Content-Security-Policy and a sandboxed frame whatever
//! this file finds. What is checked here is what can be checked of every
//! input at all, under the same rule as the text reader's: it returns, its
//! output is bounded, a refusal is whole, and the same input makes the same
//! page.
//!
//! One thing more can be said of every page, because the page is written
//! from an allow-list rather than copied: no element that could run or
//! fetch is ever named in it. That is checked by looking for the tags
//! themselves. An input can put the *letters* `<script` into the page by
//! writing `&lt;script` — that is text, shown as text, which the test in
//! `crates/core-rpc/tests/html_view.rs` holds to — so what is looked for
//! here is the element: `<script` followed by what ends a tag name, and
//! every `on…=` attribute, which a page made here never has.
#![no_main]

use core_rpc::html::Refused;
use core_rpc::html_view::{to_page, View};
use libfuzzer_sys::fuzz_target;

/// Mirrors `MOST_PAGE`, which is private: see `read_html`'s note on why a
/// copy is right.
const MOST_PAGE: usize = 4 << 20;

const PICTURES: [(&str, usize); 1] = [("logo@example", 0)];

fuzz_target!(|html: &str| {
    let pictures: Vec<(String, usize)> = PICTURES
        .iter()
        .map(|(cid, index)| (cid.to_string(), *index))
        .collect();
    for remote in [false, true] {
        let view = View {
            remote,
            open: "kuverta-mail://localhost/open/s3cret",
            parts: "kuverta-mail://localhost/part/1/2/",
            pictures: &pictures,
        };
        let Ok(page) = to_page(html, &view) else {
            continue;
        };
        // Escaping can make six bytes of one, a link adds its own address
        // and words to what it had, and the skeleton is fixed: the bound is
        // loose, and loose is the point — see `read_html`.
        assert!(
            page.html.len() <= MOST_PAGE + 8 * html.len() + 2048,
            "{} bytes of page out of {} bytes of HTML",
            page.html.len(),
            html.len()
        );
        assert_eq!(Ok(&page), to_page(html, &view).as_ref());

        let lower = page.html.to_ascii_lowercase();
        for forbidden in [
            "<script", "<iframe", "<object", "<embed", "<form", "<input", "<link", "<base",
            "<meta http-equiv=\"refresh", "<svg", "<frame", "<applet", "<textarea", "<button",
            "<select", "<video", "<audio", "<source", "<template",
        ] {
            for (at, _) in lower.match_indices(forbidden) {
                let after = lower.as_bytes().get(at + forbidden.len()).copied();
                let starts_an_element = after
                    .is_none_or(|byte| byte == b'>' || byte == b'/' || byte.is_ascii_whitespace());
                assert!(
                    !starts_an_element || forbidden.starts_with("<meta"),
                    "{forbidden} in the page at {at}"
                );
                if forbidden.starts_with("<meta") {
                    panic!("a refresh in the page at {at}");
                }
            }
        }
        // No `on…=` attribute inside any tag the page wrote. Tags are the
        // page's own, so each runs from `<` to the next `>` outside quotes
        // — which is how the page wrote them.
        for tag in tags_of(&page.html) {
            let lower = tag.to_ascii_lowercase();
            for (at, _) in lower.match_indices(" on") {
                let rest = &lower[at + 3..];
                let name_end = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
                assert!(
                    !rest[name_end..].trim_start().starts_with('='),
                    "an on… attribute in {tag:?}"
                );
            }
            assert!(!lower.contains("javascript:"), "a javascript: address in {tag:?}");
            assert!(!lower.contains("srcdoc"), "a srcdoc in {tag:?}");
        }
    }
});

/// Every tag in a page the page maker wrote, quotes respected.
fn tags_of(html: &str) -> Vec<&str> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = bytes[at..].iter().position(|b| *b == b'<').map(|i| i + at) {
        let mut quote: Option<u8> = None;
        let mut close = None;
        for (i, byte) in bytes.iter().enumerate().skip(open + 1) {
            match (quote, *byte) {
                (Some(q), b) if b == q => quote = None,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(*byte),
                (None, b'>') => {
                    close = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        if let Some(tag) = html.get(open..=close) {
            out.push(tag);
        }
        at = close + 1;
    }
    out
}

#[allow(dead_code)]
fn every_refusal_is_accounted_for(why: Refused) {
    match why {
        Refused::TooBig | Refused::TooDeep | Refused::Unterminated | Refused::Empty => {}
    }
}
