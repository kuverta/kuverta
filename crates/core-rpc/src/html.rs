//! HTML mail as text, when the sender's own plain text is not worth reading.
//!
//! Most senders put a `text/plain` beside the HTML and most of those are
//! fine. Some are not: a DHL parcel notice arrives with a plain part its own
//! converter gave up on halfway, raw `<table>` markup and Outlook conditional
//! comments and all, and kuverta was showing it faithfully. This reads the
//! HTML part instead, and only where the plain one is like that — see
//! [`looks_converted_badly`].
//!
//! ## What this is not
//!
//! It is not a browser and never becomes one. Nothing here executes, fetches,
//! or follows anything: `<script>` and `<style>` are thrown away unread,
//! `src` and `href` are treated as text and never opened, and no input can
//! make it reach the network or the disk. It takes a string and returns a
//! string.
//!
//! ## Refusing rather than guessing
//!
//! Every limit below is a refusal, not a truncation: [`to_text`] returns
//! [`Refused`] and the caller keeps what the sender sent. A half-parsed body
//! is the one outcome worth avoiding — it is indistinguishable from a
//! well-parsed one to whoever is reading it, and a message that can silently
//! drop the second half of a sentence is worse than one that admits it could
//! not be read. There is no recursion, so no input can exhaust the stack; the
//! nesting depth is counted instead and refused at [`MOST_DEPTH`].

/// Why a message was not converted. The caller shows the sender's own text
/// instead; none of these is worth an error in front of somebody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Longer than [`MOST_HTML`]. Mail this size is a mail-out, and its
    /// plain part is as good as anything this would make of it.
    TooBig,
    /// Nested deeper than [`MOST_DEPTH`]. Honest mail does not do this;
    /// something trying to make a parser fall over does.
    TooDeep,
    /// A `<script>`, `<style>` or comment that never ends. Everything after
    /// it would be a guess about where the text starts again.
    Unterminated,
    /// It converted, but to nothing worth showing.
    Empty,
}

/// The most HTML that will be read. A megabyte of markup is a newsletter with
/// every image inlined, not a letter.
const MOST_HTML: usize = 1 << 20;
/// How deeply elements may nest. Mail templates reach perhaps thirty.
const MOST_DEPTH: usize = 100;
/// The most text that will come out, so a small input cannot make a large
/// output.
const MOST_TEXT: usize = 1 << 19;

/// Elements whose content is not text and is thrown away unread.
const SILENT: [&str; 6] = ["script", "style", "head", "title", "noscript", "template"];

/// Elements that start and end a line of their own.
const BLOCK: [&str; 24] = [
    "p",
    "div",
    "br",
    "tr",
    "table",
    "ul",
    "ol",
    "li",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "hr",
    "pre",
    "form",
];

/// HTML as text, or why it was not.
pub fn to_text(html: &str) -> Result<String, Refused> {
    if html.len() > MOST_HTML {
        return Err(Refused::TooBig);
    }
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 4);
    let mut at = 0usize;
    let mut depth = 0usize;
    // The text of the link being read, so an empty one can show its address
    // rather than vanishing.
    let mut link: Option<(String, usize)> = None;

    while at < bytes.len() {
        if out.len() > MOST_TEXT {
            return Err(Refused::TooBig);
        }
        if bytes[at] != b'<' {
            let next = memchr(bytes, b'<', at).unwrap_or(bytes.len());
            push_text(&mut out, &html[at..next]);
            at = next;
            continue;
        }
        // A comment, which includes every Outlook conditional: `<!--[if mso]>`
        // opens one and the `-->` that follows closes it, so what such a block
        // holds is skipped and what the "downlevel revealed" form leaves
        // outside the comment is kept — which is what a browser does too.
        if html[at..].starts_with("<!--") {
            let end = find(html, "-->", at + 4).ok_or(Refused::Unterminated)?;
            at = end + 3;
            continue;
        }
        if html[at..].starts_with("<!") || html[at..].starts_with("<?") {
            at = memchr(bytes, b'>', at).ok_or(Refused::Unterminated)? + 1;
            continue;
        }
        let close = tag_end(bytes, at).ok_or(Refused::Unterminated)?;
        let tag = &html[at + 1..close];
        let ending = tag.starts_with('/');
        let name = tag_name(tag);
        at = close + 1;

        if SILENT.contains(&name.as_str()) && !ending {
            // Unread to its own end tag. Not to the next `>`: a stylesheet is
            // full of them.
            let end = find_close(html, &name, at).ok_or(Refused::Unterminated)?;
            at = end;
            continue;
        }
        if !ending && !tag.ends_with('/') {
            depth += 1;
            if depth > MOST_DEPTH {
                return Err(Refused::TooDeep);
            }
        } else if ending {
            depth = depth.saturating_sub(1);
        }

        match name.as_str() {
            // A link is shown as its own words. The address is kept only
            // for a link that has none — an image or a bare button — because
            // a page of addresses beside every word is what made this mail
            // unreadable in the first place.
            "a" if !ending => link = Some((attribute(tag, "href").unwrap_or_default(), out.len())),
            "a" if ending => {
                if let Some((href, from)) = link.take() {
                    // `get`, not a slice. The text is not only ever appended
                    // to: a block element inside the link runs `newline`,
                    // which pops the trailing spaces — so by the `</a>` the
                    // text can be *shorter* than it was at the `<a>`, and
                    // `out[from..]` panics. `<p><a ,href=…</h2><p></a` did
                    // it, from a fuzzer, with index 104 into 101 bytes.
                    //
                    // Nothing there to read means the link had nothing but
                    // the whitespace that has since been trimmed, which is
                    // an empty link — the same answer the slice would have
                    // given if it had lived to give one.
                    if out.get(from..).unwrap_or("").trim().is_empty() {
                        if let Some(address) = worth_showing(&href) {
                            push_text(&mut out, address);
                        }
                    }
                }
            }
            "img" => {
                if let Some(alt) = attribute(tag, "alt").filter(|alt| !alt.trim().is_empty()) {
                    push_text(&mut out, &format!("[{}]", alt.trim()));
                }
            }
            "li" if !ending => newline(&mut out, "- "),
            "td" | "th" if ending => out.push(' '),
            name if BLOCK.contains(&name) => newline(&mut out, ""),
            _ => {}
        }
    }

    let text = tidy(&out);
    if text.trim().is_empty() {
        return Err(Refused::Empty);
    }
    Ok(text)
}

/// Whether a sender's own plain text is one of the bad conversions — markup
/// it failed to take out, left in the middle of the words.
///
/// Deliberately narrow: an angle bracket is not enough, and neither is a
/// stray `<b>`. What is looked for is the wreckage of a converter — table
/// markup, Outlook's conditionals, a style attribute — because taking the
/// HTML instead of a plain part the sender wrote on purpose is a step down
/// far more often than it is a step up.
pub fn looks_converted_badly(plain: &str) -> bool {
    const WRECKAGE: [&str; 6] = [
        "<table",
        "<td",
        "<!--[if",
        "<div",
        "style=\"",
        "cellpadding",
    ];
    let found = WRECKAGE.iter().filter(|mark| plain.contains(*mark)).count();
    found >= 2
}

// -- the small parts ---------------------------------------------------------

/// Where a tag ends, which is not simply the next `>`.
///
/// An attribute may hold one: DHL's own alt text is `alt="Ablageort<br>
/// buchen"`, and stopping at that `>` ends the tag in the middle of a quoted
/// value and spills the rest of it into the message as words. Quotes are
/// tracked so the tag ends where the tag ends.
fn tag_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (at, byte) in bytes.iter().enumerate().skip(from) {
        match (quote, *byte) {
            (Some(open), byte) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*byte),
            (None, b'>') => return Some(at),
            (None, _) => {}
        }
    }
    None
}

fn memchr(bytes: &[u8], looking_for: u8, from: usize) -> Option<usize> {
    // Past the end is "not found", not a panic. `find` below has had this
    // guard all along and this did not, which is the sort of difference
    // between two neighbouring four-line functions that nobody reads twice.
    if from >= bytes.len() {
        return None;
    }
    bytes[from..]
        .iter()
        .position(|byte| *byte == looking_for)
        .map(|at| at + from)
}

fn find(text: &str, looking_for: &str, from: usize) -> Option<usize> {
    if from > text.len() {
        return None;
    }
    text[from..].find(looking_for).map(|at| at + from)
}

/// Just past the end tag of `name`, whatever is between.
fn find_close(html: &str, name: &str, from: usize) -> Option<usize> {
    let lower = html[from..].to_ascii_lowercase();
    let end = lower.find(&format!("</{name}"))? + from;
    memchr(html.as_bytes(), b'>', end).map(|at| at + 1)
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// One attribute's value, quoted or not. Never opened, never followed: this
/// is a string out of a string.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|at| at + from) {
        let before = lower[..at].chars().next_back();
        let after = lower[at + name.len()..].trim_start().chars().next();
        from = at + name.len();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '-') || after != Some('=') {
            continue;
        }
        let rest = tag[at + name.len()..].trim_start();
        let rest = rest.strip_prefix('=')?.trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or(""),
            _ => rest.split_whitespace().next().unwrap_or(""),
        };
        // An attribute can hold markup of its own — DHL writes its button
        // labels as `alt="Ablageort<br>buchen"` — and an attribute's value is
        // text, so the markup in it is taken out rather than shown.
        return Some(without_tags(&unescape(value)));
    }
    None
}

/// Ends the line, without stacking empty ones: a mail template is mostly
/// empty rows and every one of them asked for a newline.
/// Tags taken out of a string that is meant to be text. No nesting, no
/// state: an attribute value is not a document.
fn without_tags(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut inside = false;
    for c in value.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn newline(out: &mut String, then: &str) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(then);
}

/// Text into the output, with runs of space collapsed as a browser does.
fn push_text(out: &mut String, raw: &str) {
    for c in unescape(raw).chars() {
        if c.is_whitespace() {
            if !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// The most an entity may run to, `&` and `;` included. Longer than
/// `&hellip;` needs and shorter than a sentence: past this, an `&` is just an
/// `&` and the text after it is text.
const MOST_ENTITY: usize = 12;

/// Addresses a person could act on, and the only ones ever written into a
/// message as text.
const SCHEMES: [&str; 4] = ["http://", "https://", "mailto:", "tel:"];

/// A link's address, if it is one worth putting in front of somebody.
///
/// Only an empty link reaches here — an icon, a bare button — where showing
/// the address is better than showing nothing. "Better than nothing" is the
/// whole justification, and it does not hold for every address: a fuzzer
/// found `<a href="javascript:alert(1)"></a>`, which was printed into the
/// message verbatim. Nothing ran, because this produces text and the window
/// renders text, but it is a line that reads like a link and means nothing
/// to a person — and it would become a real hazard the day anything turns
/// the addresses in a message back into links.
///
/// So an address is shown when it is one somebody could follow or ring, and
/// otherwise the link is simply not there, which is what it was anyway.
/// Leading control characters and spaces go first: `\0java\tscript:` is not a
/// scheme this list has to know about, but `  https://x` is.
fn worth_showing(href: &str) -> Option<&str> {
    let href = href.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    let lowered = href.to_lowercase();
    SCHEMES
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
        .then_some(href)
}

/// The entities mail actually uses, and numeric ones. An entity this does not
/// know is left as it was written: in text, `&frac12;` is not dangerous, it
/// is only ugly.
fn unescape(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // Over the bytes, not over the string. `&mdash;` and every other
        // entity is ASCII, and an ASCII byte never appears inside a longer
        // character — but a *window* of a fixed number of bytes can end
        // inside one, and slicing a string there is a panic. A fuzzer found
        // it in two minutes with `&\n\0Ľ!Ľ&6\0Ľ<`: twelve bytes into that
        // is the middle of an `Ľ`.
        let window = &rest.as_bytes()[..rest.len().min(MOST_ENTITY)];
        let Some(end) = window.iter().position(|byte| *byte == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let put = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" | "#160" => Some(' '),
            "shy" => Some('\u{ad}'),
            "euro" => Some('€'),
            "hellip" => Some('…'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "auml" => Some('ä'),
            "ouml" => Some('ö'),
            "uuml" => Some('ü'),
            "Auml" => Some('Ä'),
            "Ouml" => Some('Ö'),
            "Uuml" => Some('Ü'),
            "szlig" => Some('ß'),
            other => other
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match put {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Blank lines collapsed and the ends trimmed: a mail template is mostly
/// empty rows, and each of them was a newline.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    say_it_once(out.trim())
}

/// A picture's own words, where the words beside it say the same thing, said
/// once.
///
/// Every button in a parcel notice is an icon and a label inside one link,
/// and the icon's `alt` is the label — so the reading came out as
/// "[Ablageort buchen] Ablageort buchen". This is a reading of the text
/// rather than of the markup on purpose: the same duplication arrives in a
/// dozen shapes and they all end up looking like this one.
fn say_it_once(text: &str) -> String {
    /// Words alone, for comparing two ways of saying the same thing.
    fn words(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < lines.len() {
        // `[alt] label`, where the label says what the alt says — over as
        // many lines as the label took, because a button's own words are
        // often written with a line break in the middle of them.
        let repeated = lines[at]
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
            .filter(|(alt, _)| !alt.trim().is_empty())
            .and_then(|(alt, after)| {
                let mut said = after.trim().to_string();
                for ahead in 0..LOOK_AHEAD {
                    if words(&said) == words(alt) {
                        return Some((said, ahead));
                    }
                    let next = lines.get(at + ahead + 1)?;
                    if next.is_empty() {
                        return None;
                    }
                    said = format!("{said} {next}");
                }
                None
            });
        match repeated {
            Some((said, skipped)) => {
                out.push_str(said.trim());
                at += skipped + 1;
            }
            None => {
                out.push_str(lines[at]);
                at += 1;
            }
        }
        out.push('\n');
    }
    out.trim().to_string()
}

/// How many lines after a picture its own words may be spread over before
/// they stop counting as the same words.
const LOOK_AHEAD: usize = 3;
