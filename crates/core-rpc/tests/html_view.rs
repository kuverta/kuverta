//! HTML mail as a page: what the page keeps, and — mostly — what it does
//! not. Each test is a claim about a particular document, where what the
//! sender wrote and what the page shows are both known; the fuzz target
//! `html_page` holds the claims that are true of every input.

use core_rpc::html::Refused;
use core_rpc::html_view::{
    link_target, percent_decode, percent_encode, query_values, to_page, Page, View,
};

const OPEN: &str = "kuverta-mail://localhost/open/s3cret";
const PARTS: &str = "kuverta-mail://localhost/part/1/2/";

fn page(html: &str) -> Page {
    to_page(html, &view(false, &[])).expect("a page")
}

fn view<'a>(remote: bool, pictures: &'a [(String, usize)]) -> View<'a> {
    View {
        remote,
        open: OPEN,
        parts: PARTS,
        pictures,
    }
}

/// The page between the app's own wrapper and its end.
fn body(page: &Page) -> &str {
    let from = page
        .html
        .find("<div class=\"k-body\">")
        .expect("the wrapper")
        + 20;
    let to = page.html.rfind("</div></body></html>").expect("the end");
    &page.html[from..to]
}

#[test]
fn text_and_layout_are_kept_as_they_were() {
    let made = page(
        r##"<table width="600" cellpadding="0" bgcolor="#ffffff"><tr><td align="center" style="padding:4px">Hallo <b>Erika</b></td></tr></table>"##,
    );
    assert_eq!(
        body(&made),
        r##"<table width="600" bgcolor="#ffffff" cellpadding="0"><tr><td style="padding:4px" align="center">Hallo <b>Erika</b></td></tr></table>"##
    );
    assert!(made.html.starts_with("<!doctype html>"));
    assert!(made.html.contains("Content-Security-Policy"));
}

#[test]
fn scripts_handlers_and_frames_do_not_survive() {
    let made = page(
        r##"<p onclick="alert(1)">a</p><script>alert(2)</script><iframe src="https://x"></iframe><object data="x"></object><embed src="x"><form action="https://x"><input name="pw"><button>go</button></form><svg onload="alert(3)"><script>4</script></svg><p>b</p>"##,
    );
    let shown = body(&made);
    assert_eq!(shown, "<p>a</p>go<p>b</p>");
    assert!(!made.html.contains("alert"));
    assert!(!made.html.contains("onclick"));
}

#[test]
fn a_meta_refresh_a_base_and_a_stylesheet_link_are_not_there() {
    let made = page(
        r##"<head><meta http-equiv="refresh" content="0;url=https://evil.example"><base href="https://evil.example/"><link rel="stylesheet" href="https://evil.example/x.css"><style>p{color:red}</style></head><body><p>x</p></body>"##,
    );
    assert_eq!(
        body(&made),
        "<style>p{color:red}</style><div><p>x</p></div>"
    );
    assert!(!made.html.contains("evil.example"));
    assert!(!made.html.contains("<meta http-equiv=\"refresh"));
}

#[test]
fn a_link_goes_to_the_app_with_its_real_address_and_words() {
    let made = page(
        r##"<p><a href="https://example.de/x?a=1&b=2" target="_blank" onclick="x()">Sendung <b>verfolgen</b></a></p>"##,
    );
    assert_eq!(made.links, 1);
    assert_eq!(
        body(&made),
        concat!(
            r##"<p><a href="kuverta-mail://localhost/open/s3cret?to=https%3A%2F%2Fexample.de%2Fx%3Fa%3D1%26b%3D2&amp;text=Sendung%20verfolgen" "##,
            r##"title="https://example.de/x?a=1&amp;b=2" rel="noreferrer noopener">Sendung <b>verfolgen</b></a></p>"##
        )
    );
}

#[test]
fn a_link_to_anything_but_a_page_an_address_or_a_number_is_not_a_link() {
    for href in [
        "javascript:alert(1)",
        "  JaVaScRiPt:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "vbscript:x",
        "kuverta-mail://localhost/open/s3cret?to=https%3A%2F%2Fx",
        "/relative",
        "#top",
        "",
    ] {
        let made = page(&format!(r##"<p><a href="{href}">click</a></p>"##));
        assert_eq!(body(&made), "<p><a>click</a></p>", "{href:?}");
        assert_eq!(made.links, 0, "{href:?}");
        assert_eq!(link_target(href), None, "{href:?}");
    }
    assert_eq!(
        link_target("MAILTO:a@example.de"),
        Some("MAILTO:a@example.de".into())
    );
    assert_eq!(link_target("tel:+49301234"), Some("tel:+49301234".into()));
    assert_eq!(
        link_target("https://ex\nample.de"),
        Some("https://example.de".into())
    );
}

#[test]
fn a_picture_on_the_internet_is_counted_and_left_out_until_asked_for() {
    let html = r##"<img src="https://t.example/open.gif" width="1" height="1" alt=""><img src="http://t.example/b.png">"##;
    let blocked = page(html);
    assert_eq!(blocked.remote_blocked, 2);
    assert_eq!(
        body(&blocked),
        r##"<img data-blocked="" alt="" width="1" height="1"><img data-blocked="">"##
    );
    assert!(!blocked.html.contains("t.example"));
    assert!(!blocked.csp.contains("https:"));

    let shown = to_page(html, &view(true, &[])).unwrap();
    assert_eq!(shown.remote_blocked, 0);
    assert!(body(&shown).contains(r##"<img src="https://t.example/open.gif""##));
    assert!(shown
        .csp
        .contains("img-src data: kuverta-mail://localhost/part/1/2/ https: http:"));
}

#[test]
fn a_picture_inside_the_message_is_fetched_from_the_app_by_its_index() {
    let pictures = vec![("logo@example".to_string(), 3usize)];
    let made = to_page(
        r##"<img src="cid:logo@example"><img src="CID:LOGO@EXAMPLE"><img src="cid:unknown">"##,
        &view(false, &pictures),
    )
    .unwrap();
    assert_eq!(
        body(&made),
        r##"<img src="kuverta-mail://localhost/part/1/2/3"><img src="kuverta-mail://localhost/part/1/2/3"><img data-blocked="">"##
    );
    assert_eq!(made.remote_blocked, 0);
}

#[test]
fn a_picture_written_into_the_message_is_kept_and_a_document_is_not() {
    let made = page(
        r##"<img src="data:image/png;base64,iVBORw0KGgo="><img src="data:image/svg+xml;base64,PHN2Zz4=">"##,
    );
    assert_eq!(
        body(&made),
        r##"<img src="data:image/png;base64,iVBORw0KGgo="><img data-blocked="">"##
    );
}

#[test]
fn style_keeps_its_look_and_loses_its_reach() {
    let made = page(
        r##"<style>.a{background:url(https://t.example/x.png);color:red}@import "x.css";.b{background-image:u\72l("https://t.example/y")}</style><p style="background: URL( 'https://t.example/z' ); font-weight: bold; behavior: url(x.htc)">x</p>"##,
    );
    let shown = body(&made);
    assert!(!shown.contains(":url("), "{shown}");
    assert!(!shown.contains(" url("), "{shown}");
    assert!(!shown.contains("URL("), "{shown}");
    assert!(!shown.contains("72l("), "{shown}");
    assert_eq!(shown.matches("no-url(").count(), 4, "{shown}");
    assert!(shown.contains("@no-import"), "{shown}");
    assert!(shown.contains("no-behavior"), "{shown}");
    assert!(shown.contains("color:red"));
    assert!(shown.contains("font-weight: bold"));
    assert!(shown.contains("no-url( 'https://t.example/z' )"), "{shown}");
    assert_eq!(made.remote_blocked, 3);

    let shown = to_page(
        r##"<p style="background:url(https://t.example/z)">x</p>"##,
        &view(true, &[]),
    )
    .unwrap();
    assert!(body(&shown).contains("url(https://t.example/z)"));
    assert_eq!(shown.remote_blocked, 0);
}

#[test]
fn a_body_becomes_a_block_with_its_colours() {
    let made = page(r##"<body bgcolor="#f0f0f0" style="margin:0" onload="x()"><p>x</p></body>"##);
    assert_eq!(
        body(&made),
        r##"<div style="margin:0;background-color:#f0f0f0"><p>x</p></div>"##
    );
}

#[test]
fn attribute_values_cannot_close_the_tag_they_are_in() {
    let made = page(
        r##"<p title="&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;" class='x" onmouseover="y'>t</p>"##,
    );
    assert_eq!(
        body(&made),
        r##"<p class="x&quot; onmouseover=&quot;y" title="&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;">t</p>"##
    );
}

#[test]
fn what_is_only_quoted_markup_stays_quoted() {
    let made = page("<p>Write &lt;script&gt;alert(1)&lt;/script&gt; to embed one.</p>");
    assert_eq!(
        body(&made),
        "<p>Write &lt;script&gt;alert(1)&lt;/script&gt; to embed one.</p>"
    );
}

#[test]
fn a_background_on_a_cell_is_a_picture_on_the_internet() {
    let html = r##"<table background="https://t.example/bg.jpg"><tr><td background="javascript:x">x</td></tr></table>"##;
    let blocked = page(html);
    assert_eq!(body(&blocked), "<table><tr><td>x</td></tr></table>");
    assert_eq!(blocked.remote_blocked, 1);
    let shown = to_page(html, &view(true, &[])).unwrap();
    assert_eq!(
        body(&shown),
        r##"<table background="https://t.example/bg.jpg"><tr><td>x</td></tr></table>"##
    );
}

#[test]
fn measures_and_colours_are_what_they_say_or_nothing() {
    let made = page(
        r##"<td width="50%" height="x;expression(1)" bgcolor="red" color="url(x)" dir="rtl" align="left" valign="x y">t</td>"##,
    );
    assert_eq!(
        body(&made),
        r##"<td dir="rtl" align="left" width="50%" bgcolor="red">t</td>"##
    );
}

#[test]
fn too_much_too_deep_unterminated_and_nothing_are_refused_whole() {
    let nothing = &[][..];
    let refused = |html: &str| to_page(html, &view(false, nothing)).err();
    assert_eq!(refused(&"<div>".repeat(200)), Some(Refused::TooDeep));
    assert_eq!(
        refused(&format!("<p>{}</p>", "x".repeat(2 << 20))),
        Some(Refused::TooBig)
    );
    assert_eq!(
        refused("<p>a<!-- never closed"),
        Some(Refused::Unterminated)
    );
    assert_eq!(refused("<script>forever"), Some(Refused::Unterminated));
    assert_eq!(refused("<style>forever"), Some(Refused::Unterminated));
    assert_eq!(refused("<p>a<b"), Some(Refused::Unterminated));
    assert_eq!(
        refused("<table><tr><td> </td></tr></table>"),
        Some(Refused::Empty)
    );
    assert_eq!(refused(""), Some(Refused::Empty));
    // A picture alone is a message.
    assert!(to_page(
        r##"<img src="data:image/gif;base64,R0lGODlh">"##,
        &view(false, nothing)
    )
    .is_ok());
}

#[test]
fn the_policy_names_nothing_but_style_and_the_apps_own_pictures() {
    let made = page("<p>x</p>");
    assert_eq!(
        made.csp,
        "default-src 'none'; style-src 'unsafe-inline'; img-src data: kuverta-mail://localhost/part/1/2/; form-action 'none'; base-uri 'none'; sandbox"
    );
    // The same policy inside the page, without the directive a <meta>
    // cannot carry.
    assert!(made.html.contains(
        r##"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data: kuverta-mail://localhost/part/1/2/; form-action 'none'; base-uri 'none'">"##
    ));
    assert!(made
        .html
        .contains(r##"<meta name="referrer" content="no-referrer">"##));
}

#[test]
fn a_links_words_are_shortened_and_the_query_reads_back() {
    let long = "w".repeat(500);
    let made = page(&format!(r##"<a href="https://example.de/">{long}</a>"##));
    let text = made
        .html
        .split("&amp;text=")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(text.len(), 120);

    let query = format!(
        "to={}&text={}",
        percent_encode("https://example.de/ä?x=1&y"),
        percent_encode("Jetzt zahlen")
    );
    let values = query_values(&query);
    assert_eq!(
        values,
        vec![
            ("to".to_string(), "https://example.de/ä?x=1&y".to_string()),
            ("text".to_string(), "Jetzt zahlen".to_string())
        ]
    );
    assert_eq!(percent_decode("a%2"), "a%2");
    assert_eq!(percent_decode("%FF"), "\u{fffd}");
}

#[test]
fn the_same_page_comes_out_twice() {
    let html = r##"<html><body><table><tr><td><a href="https://e.de">x</a><img src="https://e.de/i.png"></td></tr></table></body></html>"##;
    assert_eq!(page(html), page(html));
}
