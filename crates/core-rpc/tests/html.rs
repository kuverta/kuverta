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
