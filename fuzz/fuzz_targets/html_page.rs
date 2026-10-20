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
            "<script",
            "<iframe",
            "<object",
            "<embed",
            "<form",
            "<input",
            "<link",
            "<base",
            "<meta http-equiv=\"refresh",
            "<svg",
            "<frame",
            "<applet",
            "<textarea",
            "<button",
            "<select",
            "<video",
            "<audio",
            "<source",
            "<template",
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
        // No `on…` attribute, no `srcdoc`, and no `javascript:` address in
        // any tag the page wrote. Tags are the page's own, so each runs from
        // `<` to the next `>` outside quotes, and every attribute in one is
        // double-quoted with `"` escaped inside — which is how the page
        // wrote them, and why the names and values can be read back apart.
        // A title that says `a onx=b` is words in a value, not an attribute,
        // and no browser reads it as one.
        for tag in tags_of(&page.html) {
            for (name, value) in attributes_of(tag) {
                assert!(!name.starts_with("on"), "an on… attribute in {tag:?}");
                assert!(name != "srcdoc", "a srcdoc in {tag:?}");
                if matches!(
                    name.as_str(),
                    "href" | "src" | "action" | "formaction" | "xlink:href"
                ) {
                    let scheme: String = value
                        .chars()
                        .filter(|c| !c.is_whitespace() && !c.is_control())
                        .collect::<String>()
                        .to_ascii_lowercase();
                    assert!(
                        !scheme.starts_with("javascript:") && !scheme.starts_with("vbscript:"),
                        "a {name} that runs in {tag:?}"
                    );
                }
            }
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

/// The attributes of one tag the page wrote, lower-cased names with their
/// values: every value is double-quoted, and a quote inside one is `&quot;`,
/// so a value runs to the next `"`.
fn attributes_of(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let inner = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');
    // Past the element name.
    let mut rest = inner.trim_start();
    rest = rest.find(char::is_whitespace).map_or("", |at| &rest[at..]);
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let name_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let value = match rest.strip_prefix('=') {
            Some(after) => {
                let after = after.trim_start();
                let Some(quoted) = after.strip_prefix('"') else {
                    panic!("an unquoted value for {name} in {tag:?}");
                };
                let end = quoted
                    .find('"')
                    .unwrap_or_else(|| panic!("an unclosed value for {name} in {tag:?}"));
                rest = &quoted[end + 1..];
                quoted[..end].to_string()
            }
            None => String::new(),
        };
        out.push((name, value));
    }
    out
}

#[allow(dead_code)]
fn every_refusal_is_accounted_for(why: Refused) {
    match why {
        Refused::TooBig | Refused::TooDeep | Refused::Unterminated | Refused::Empty => {}
    }
}
