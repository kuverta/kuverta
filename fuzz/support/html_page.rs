//! Checks shared by the HTML fuzz target and its deterministic regressions.
//! These inspect the renderer's generated markup independently of its parser.

pub fn assert_safe_page(html: &str) {
    let lower = html.to_ascii_lowercase();
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
            let starts_an_element =
                after.is_none_or(|byte| byte == b'>' || byte == b'/' || byte.is_ascii_whitespace());
            assert!(
                !starts_an_element || forbidden.starts_with("<meta"),
                "{forbidden} in the page at {at}"
            );
            if forbidden.starts_with("<meta") {
                panic!("a refresh in the page at {at}");
            }
        }
    }
    for tag in tags_of(html) {
        // A word inside an alt/title or URL is not an attribute. In particular,
        // the fuzzed tracking URL may contain "srcdoc" without creating one.
        for (name, value) in attributes_of(tag) {
            let name = name.to_ascii_lowercase();
            assert!(!name.starts_with("on"), "an on… attribute in {tag:?}");
            assert_ne!(name, "srcdoc", "a srcdoc in {tag:?}");
            // These are the URL-bearing attributes the renderer writes. A
            // JavaScript scheme is forbidden; that word in a path/title is not.
            if matches!(name.as_str(), "href" | "src") {
                let address: String = value.chars().filter(|c| !c.is_ascii_control()).collect();
                assert!(
                    !address
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("javascript:"),
                    "a javascript: address in {tag:?}"
                );
            }
        }
    }
}

/// Read names and values from generated tags without treating quoted content
/// as markup. This is deliberately independent of the production HTML parser.
fn attributes_of(tag: &str) -> Vec<(&str, &str)> {
    let bytes = tag.as_bytes();
    let mut at = 1;
    // Skip the element name (or the entire closing tag).
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>' {
        at += 1;
    }
    let mut attributes = Vec::new();
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        if at == bytes.len() || bytes[at] == b'>' {
            break;
        }
        let start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'=' | b'>' | b'/')
        {
            at += 1;
        }
        assert!(at > start, "invalid generated attribute in {tag:?}");
        let name = &tag[start..at];
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = "";
        if bytes.get(at) == Some(&b'=') {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let quote = bytes.get(at).copied().filter(|b| matches!(b, b'"' | b'\''));
            if quote.is_some() {
                at += 1;
            }
            let start = at;
            while at < bytes.len()
                && match quote {
                    Some(q) => bytes[at] != q,
                    None => !bytes[at].is_ascii_whitespace() && bytes[at] != b'>',
                }
            {
                at += 1;
            }
            value = &tag[start..at];
            if quote.is_some() && at < bytes.len() {
                at += 1;
            }
        }
        attributes.push((name, value));
    }
    attributes
}

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

#[cfg(test)]
mod tests {
    use super::assert_safe_page;

    #[test]
    fn srcdoc_inside_a_url_is_not_an_attribute() {
        // Reduced from the failing html_page run on PR #28.
        assert_safe_page(
            r#"<img src="https://track.shop.example.srcdocen?id=99213&amp;u=dev%40kuverta.test" alt="" width="1" height="1">"#,
        );
        assert_safe_page(r#"<img src='https://example.test/srcdoc.png' alt='srcdoc'>"#);
    }

    #[test]
    fn quoted_words_do_not_become_attributes_or_url_schemes() {
        assert_safe_page(
            r#"<a href="https://example.test/javascript:guide" title="Notes onload=example srcdoc javascript:">read</a>"#,
        );
        assert_safe_page(r#"<p title='a > b; onerror="example" srcdoc'>text</p>"#);
    }

    #[test]
    fn actual_active_attributes_are_rejected() {
        for html in [
            r#"<img srcdoc="<p>unsafe</p>">"#,
            "<p SRCDOC = 'unsafe'>",
            "<p srcdoc>",
            "<img onload='run()'>",
            "<img\nONERROR = run()>",
            r#"<a href="JaVaScRiPt:run()">"#,
            "<img src=' javascript:run()'>",
            "<a href='java\tscript:run()'>",
        ] {
            assert!(
                std::panic::catch_unwind(|| assert_safe_page(html)).is_err(),
                "unsafe markup was accepted: {html}"
            );
        }
    }

    #[test]
    fn forbidden_elements_are_still_rejected() {
        for html in [
            "<script>x()</script>",
            "<iframe></iframe>",
            "<form>",
            "<svg/>",
            r#"<meta http-equiv="refresh" content="0">"#,
        ] {
            assert!(std::panic::catch_unwind(|| assert_safe_page(html)).is_err());
        }
    }
}
