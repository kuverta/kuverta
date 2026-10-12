//! What the assistant reads off the web, as bytes anyone can serve.
//!
//! The page itself goes through the HTML reader, which has a target of its
//! own. This is the rest: the scanner that finds a page's title and links,
//! the reader of DuckDuckGo's results page, the skill file parser and the
//! list of what stands out in one. Each takes a string somebody else wrote,
//! and each promises to return, to stay within its own limits, and to hand
//! back only web addresses as links.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let base = url::Url::parse("https://fuzz.example/a/b?c=d").unwrap();

    let results = core_rpc::web::parse_duckduckgo(text);
    assert!(results.len() <= 10);
    for result in &results {
        assert!(result.url.starts_with("http://") || result.url.starts_with("https://"));
    }
    let _ = core_rpc::web::title_of(text);
    let links = core_rpc::web::links_in(text, &base);
    assert!(links.len() <= 60);
    for link in &links {
        assert!(link.url.starts_with("http://") || link.url.starts_with("https://"));
        assert!(link.text.chars().count() <= 80);
    }
    let _ = core_rpc::web::parse_searxng(&serde_json::from_str(text).unwrap_or(serde_json::Value::Null));
    let _ = core_rpc::web::urls_in_text(text);
    let _ = core_rpc::web::without_mail(text);

    let parsed = core_rpc::skills::parse_skill(text, "https://fuzz.example/skills/x/SKILL.md");
    assert!(parsed.name.chars().count() <= 80);
    let warnings = core_rpc::skills::warnings(text);
    assert!(warnings.len() <= 41);
});
