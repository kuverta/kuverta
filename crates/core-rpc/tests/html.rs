//! HTML mail as text, and what it refuses.
//!
//! The case this exists for arrived from DHL: a parcel notice whose own
//! `text/plain` part its converter had given up on halfway, so the reading
//! pane showed `<table border="0" cellpadding="0">` and Outlook conditional
//! comments between the sentences. These are the two halves of the answer —
//! noticing that a plain part is that bad, and making a better one.

use core_rpc::html::{looks_converted_badly, to_text, Refused};

#[test]
fn a_parcel_notice_reads_as_words_rather_than_as_markup() {
    // Shortened from the real thing, conditional comments and all.
    let html = r#"<html><head><style>.x{color:red}</style><title>ignored</title></head>
<body>
<!-- Dynamic Mailix Button, target group needed, only -->
<p>Hallo Nicolas Zemke,</p>
<p>Ihre <b>Lap-works</b> Sendung ist unterwegs.</p>
<table border="0" cellpadding="0" cellspacing="0" width="360"><tr>
<td align="center">
  <!--[if mso]>
  <table width="160"><tr><td><a href="https://custcomm.dhl.de/go/10/A">only Outlook sees this</a>
  <![endif]-->
  <!--[if !mso]><! -->
  <a href="https://custcomm.dhl.de/go/10/B"><img src="https://x/icon.png" alt="Ablageort buchen"></a>
  <!--<![endif]-->
</td>
<td>Ihre Sendungsnummer</td>
</tr></table>
<p>Viele Gr&uuml;&szlig;e</p>
</body></html>"#;

    let text = to_text(html).expect("a parcel notice is readable");
    assert!(text.contains("Hallo Nicolas Zemke,"), "{text}");
    assert!(
        text.contains("Ihre Lap-works Sendung ist unterwegs."),
        "{text}"
    );
    assert!(text.contains("Viele Grüße"), "entities: {text}");
    assert!(
        text.contains("[Ablageort buchen]"),
        "the button's own words: {text}"
    );

    // None of the wreckage: no markup, no stylesheet, no Outlook-only branch,
    // and no page of addresses between the words.
    for gone in [
        "<table",
        "cellpadding",
        "color:red",
        "ignored",
        "only Outlook sees this",
        "https://",
    ] {
        assert!(!text.contains(gone), "{gone:?} survived:\n{text}");
    }
    // And it is short, because that was the complaint.
    assert!(text.lines().count() < 12, "still long:\n{text}");
}

#[test]
fn a_button_is_not_read_out_twice_because_its_icon_repeats_it() {
    // Every button in the real notice is an icon and a label inside one
    // link, and the icon's alt text is the label — so it came out as
    // "[Ablageort buchen] Ablageort buchen". The label is written with a
    // line break through the middle of it, which is why this looks past one.
    let html = r#"<a href="https://x"><img src="i.png" alt="Ablageort<br>buchen">
        <span>Ablageort<br>buchen</span></a>"#;
    assert_eq!(to_text(html).unwrap(), "Ablageort buchen");

    // A picture whose words are its own keeps them.
    let html = r#"<p><img src="i.png" alt="Unterschrift"> Mit freundlichen Grüßen</p>"#;
    assert_eq!(
        to_text(html).unwrap(),
        "[Unterschrift] Mit freundlichen Grüßen"
    );
}

#[test]
fn a_tag_ends_where_the_tag_ends_and_not_at_a_quoted_angle_bracket() {
    // The bug this is here for: `alt="Ablageort<br>buchen"` ended the tag at
    // the `>` inside the quotes, and the rest of the attribute spilled into
    // the message as words — style declarations and all.
    let html = r#"<p><img alt="a<br>b" style="width:66px;height:58px;">Text</p>"#;
    let text = to_text(html).unwrap();
    assert_eq!(
        text, "[a b]Text",
        "the alt is kept, the markup in it is not"
    );
    for gone in ["style", "width:66px", "58px", "\""] {
        assert!(!text.contains(gone), "{gone:?} spilled: {text}");
    }
}

#[test]
fn a_link_with_no_words_of_its_own_keeps_its_address() {
    let text = to_text(r#"<p>See <a href="https://example.de/x"></a></p>"#).unwrap();
    assert!(text.contains("https://example.de/x"), "{text}");
    // One that has words shows the words, and not the address as well.
    let text = to_text(r#"<p><a href="https://example.de/x">Sendung verfolgen</a></p>"#).unwrap();
    assert_eq!(text, "Sendung verfolgen");
}

#[test]
fn only_a_plain_part_that_is_half_markup_gives_way() {
    // The real one: table markup and a conditional, in the middle of words.
    let bad = "Hallo,\n<table border=\"0\" cellpadding=\"0\">\n<!--[if mso]>\n<td>x</td>";
    assert!(looks_converted_badly(bad));

    // What must not give way: plain text a sender wrote on purpose, even
    // where it talks about markup or carries one stray tag.
    for good in [
        "Hallo Erika,\n\nder Termin steht. Viele Grüße",
        "Use <b>bold</b> for emphasis — that is all the HTML you need.",
        "a < b and b > c",
        "",
    ] {
        assert!(!looks_converted_badly(good), "{good:?}");
    }
}

#[test]
fn what_cannot_be_read_is_refused_rather_than_half_read() {
    // Nothing here may panic, hang, or come back with half a message.
    assert_eq!(to_text(&"<div>".repeat(200)), Err(Refused::TooDeep));
    assert_eq!(
        to_text(&format!("<p>{}</p>", "x".repeat(2 << 20))),
        Err(Refused::TooBig)
    );
    assert_eq!(to_text("<p>a<!-- never closed"), Err(Refused::Unterminated));
    assert_eq!(to_text("<script>forever"), Err(Refused::Unterminated));
    assert_eq!(to_text("<p>a<b"), Err(Refused::Unterminated));
    assert_eq!(to_text("<p>  </p>"), Err(Refused::Empty));
    assert_eq!(to_text(""), Err(Refused::Empty));
}

#[test]
fn nothing_in_a_message_is_run_fetched_or_followed() {
    // A script is not text and is never shown, whatever it says; an event
    // handler is an attribute and attributes are not read out; an address is
    // a string and stays one.
    let nasty = r#"<html><body>
<script>alert('x')</script>
<img src="https://tracker.example/pixel.gif?who=me">
<div onclick="fetch('https://tracker.example/clicked')">Guten Tag</div>
<a href="javascript:alert(1)">Angebot</a>
<iframe src="https://tracker.example/frame"></iframe>
<style>body{background:url(https://tracker.example/bg.png)}</style>
</body></html>"#;
    let text = to_text(nasty).unwrap();
    assert_eq!(text, "Guten Tag\nAngebot");
    for gone in [
        "alert",
        "tracker.example",
        "javascript:",
        "onclick",
        "fetch",
    ] {
        assert!(!text.contains(gone), "{gone:?} survived:\n{text}");
    }
}

#[test]
fn a_table_of_cells_reads_as_rows_rather_than_as_one_run_of_words() {
    let text = to_text("<table><tr><td>Betrag</td><td>91,61</td></tr><tr><td>Datum</td><td>10.04.</td></tr></table>")
        .unwrap();
    assert_eq!(text, "Betrag 91,61\nDatum 10.04.");
}

#[test]
fn a_list_reads_as_a_list() {
    let text = to_text("<p>Bitte:</p><ul><li>zahlen</li><li>oder widersprechen</li></ul>").unwrap();
    assert_eq!(text, "Bitte:\n- zahlen\n- oder widersprechen");
}

#[test]
fn an_entity_window_that_lands_inside_a_character_does_not_cut_it_in_half() {
    // Found by fuzzing in two minutes. Entities are looked for in a window of
    // twelve bytes after an `&`; twelve bytes into this is the middle of the
    // second `Ľ`, and slicing a `str` there is a panic. A letter carrying it
    // would have taken down the thread rendering it, from anyone who can send
    // mail.
    // Returning at all is the whole assertion — the bug was a panic, and
    // either answer about this string is a fine one.
    let found = "&\n\0Ľ!Ľ&6\0Ľ<";
    let _ = to_text(found);

    // The same shape, deliberately: an `&` with a multi-byte character
    // straddling every byte of the window after it.
    for pad in 0..16 {
        let html = format!("<p>&{}Ľ;x</p>", "a".repeat(pad));
        let text = to_text(&html).expect("a paragraph with an `&` in it is readable");
        assert!(text.contains('x'), "{html:?} lost its text: {text:?}");
    }

    // And an entity that really is one still is, whatever is around it.
    let text = to_text("<p>Ľ &amp; Ľ &mdash; Ľ</p>").unwrap();
    assert_eq!(text.trim(), "Ľ & Ľ — Ľ");
}

#[test]
fn a_letter_about_html_gets_its_html_back_as_words_rather_than_as_markup() {
    // `&lt;script&gt;` is somebody writing about a script tag, not sending
    // one, and what they wrote is what they should read back. It comes out
    // as the characters `<script>` — text, which the window renders as text.
    //
    // This is here because the fuzzer's first run in CI asserted the opposite
    // and failed on ordinary mail: a target that forbids the *bytes* `<script`
    // anywhere in the output cannot tell a quoted tag from a carried one. The
    // difference is whether the reader put them there, and that is what the
    // rest of this file checks on documents where the answer is known.
    let text = to_text("<p>Write &lt;script&gt;alert(1)&lt;/script&gt; to embed one.</p>").unwrap();
    assert_eq!(text.trim(), "Write <script>alert(1)</script> to embed one.");

    // The same bytes, sent as a real script, are not in the text at all.
    let text = to_text("<p>Hello</p><script>alert(1)</script><p>Bye</p>").unwrap();
    assert!(
        !text.contains("alert"),
        "the script's body survived: {text:?}"
    );
    assert!(
        !text.contains("script"),
        "the script tag survived: {text:?}"
    );
    assert_eq!(text.trim(), "Hello\nBye");
}

#[test]
fn an_empty_link_shows_an_address_somebody_could_follow_and_no_other_kind() {
    // The reason an empty link shows its address at all is that showing
    // nothing would be worse. That reasoning does not reach a `javascript:`
    // address, which means nothing to a person reading it — and a fuzzer
    // found this one printed into the message word for word.
    for href in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "vbscript:msgbox",
        "data:text/html;base64,PHNjcmlwdD4=",
        "  \u{0}java\u{0}script:alert(1)",
        "file:///etc/passwd",
        "about:blank",
    ] {
        let text = to_text(&format!(
            r#"<p>Before</p><a href="{href}"></a><p>After</p>"#
        ))
        .unwrap();
        assert_eq!(
            text.trim(),
            "Before\nAfter",
            "`{href}` was written into the message"
        );
    }

    // And the addresses somebody could act on are still shown, whatever
    // spacing the markup put round them.
    for href in [
        "https://example.de/paket",
        "http://example.de",
        "mailto:hallo@example.de",
        "tel:+4940123456",
        "  https://example.de/gap  ",
    ] {
        let text = to_text(&format!(r#"<a href="{href}"></a>"#)).unwrap();
        assert_eq!(text.trim(), href.trim(), "`{href}` should still be shown");
    }

    // A link with words of its own shows its words and never its address,
    // whatever the address is.
    let text = to_text(r#"<a href="javascript:alert(1)">Sendung verfolgen</a>"#).unwrap();
    assert_eq!(text.trim(), "Sendung verfolgen");
}

#[test]
fn a_link_wrapped_round_a_paragraph_does_not_take_the_reader_with_it() {
    // The text is not only ever appended to. A block element runs `newline`,
    // which pops the trailing spaces, so by the closing `</a>` the text can
    // be shorter than it was at the opening one. Reading the link's own words
    // back by the offset noted at the `<a>` then panicked — index 104 into a
    // string of 101 bytes, from a fuzzer, on markup like this.
    let text = to_text("<p><a href=\"https://example.de\"> </h2><p></a>Danach</p>").unwrap();
    assert!(text.contains("Danach"), "{text:?}");

    // A link with a whole paragraph inside it is read for its words, and its
    // address stays out of the way.
    let text = to_text(
        "<p>Vor</p><a href=\"https://example.de/paket\"><p>Sendung verfolgen</p></a><p>Nach</p>",
    )
    .unwrap();
    assert!(text.contains("Sendung verfolgen"), "{text:?}");
    assert!(
        !text.contains("example.de"),
        "the address is not shown: {text:?}"
    );

    // And one wrapped round nothing but a block that empties itself still
    // counts as empty, so its address is what is left to show.
    let text = to_text("<a href=\"https://example.de/x\"><p>  </p></a>").unwrap();
    assert_eq!(text.trim(), "https://example.de/x");
}
