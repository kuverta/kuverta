//! HTML mail as a page, made safe to look at.
//!
//! [`html::to_text`] reads a message's HTML as text, and text is what the
//! reading pane shows by default. This is the other tab: the message as the
//! sender laid it out, in a frame of the window that can do nothing — no
//! scripts, no forms, no reaching the network — and whose page is written
//! here from an allow-list, element by element, attribute by attribute.
//! Nothing the sender wrote goes to the frame as it was written; what the
//! frame gets is a new document that mentions only what this file chose to
//! keep.
//!
//! ## What is kept and what is not
//!
//! Text, the elements that lay it out (tables, paragraphs, lists, headings,
//! fonts), their presentation attributes, and style — inline and in `<style>`
//! — with everything that could fetch or run taken out of the CSS. Scripts,
//! frames, objects, forms, inputs, `<meta>` (a refresh is a navigation),
//! `<link>`, `<base>`, SVG and every `on…` attribute are not kept; the
//! elements are dropped with their content where the content is not text,
//! and dropped around their content where it is.
//!
//! ## Pictures
//!
//! A picture inside the message (`cid:`) is fetched from the app itself, by
//! the index the attachments list gives it. A picture written into the
//! message (`data:image/…`) is kept as it is — it cannot reach anywhere. A
//! picture on the internet is the sender's way of learning that the message
//! was opened, and when, from where: it is not shown until the person says
//! so for this message ([`View::remote`]), and until then it is counted so
//! the window can say how many there are.
//!
//! ## Links
//!
//! A link never goes where it says. Its address becomes the app's own
//! ([`View::open`]) with the real address and the link's words behind it, so
//! that a click reaches the app and the app can ask — showing the real
//! address beside what the link said — before anything is opened. A link to
//! anything but a web page, a mail address or a phone number is not a link.
//! The real address is also the link's `title`, so hovering shows it.
//!
//! ## Refusing rather than guessing
//!
//! The same rule as the text reader: too big, too deep, or unterminated is
//! refused whole, never shown half. A half-sanitised page is worse than a
//! half-read one, because what was dropped is exactly what mattered.

use crate::html::{
    find, find_close, memchr, tag_end, tag_name, unescape, without_tags, Refused, MOST_DEPTH,
    MOST_HTML,
};

/// How the page is to be made: what it may show and where its links and
/// pictures lead.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    /// Whether pictures on the internet are shown. Off until the person says
    /// so for the message in front of them.
    pub remote: bool,
    /// Where a click on a link goes: the app's own address, to which `?to=`
    /// and `&text=` are appended. Must be one no message could guess — it
    /// carries a secret for that reason.
    pub open: &'a str,
    /// Where the message's own pictures are fetched from, by index: this
    /// followed by the index, as [`Core::attachment`](crate::Core::attachment)
    /// numbers them.
    pub parts: &'a str,
    /// The message's own pictures: each Content-ID, without its angle
    /// brackets, and the index it is fetched by.
    pub pictures: &'a [(String, usize)],
}

/// The page, ready to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// A whole document, `<!doctype html>` to `</html>`.
    pub html: String,
    /// The Content-Security-Policy the document is to be served under. Also
    /// written into the document as a `<meta>`, so a frame that lost the
    /// header still has most of it; the header is the one that counts, since
    /// `sandbox` cannot be set from a `<meta>`.
    pub csp: String,
    /// How many pictures from the internet were left out, so the window can
    /// say so and offer to show them.
    pub remote_blocked: usize,
    /// How many links the page has.
    pub links: usize,
}

/// The most page that will be written. The input is bounded by
/// [`MOST_HTML`]; this bounds what escaping and link rewriting can make of
/// it.
const MOST_PAGE: usize = 4 << 20;

/// The most of a link's own words that travel with it to the question
/// "open this?", in characters.
const MOST_LINK_TEXT: usize = 120;

/// The longest address a link may have. Tracking links run to a thousand
/// characters; nothing a person is asked to read runs to four.
const MOST_ADDRESS: usize = 4096;

/// Elements whose content is not text and is dropped unread, to their own
/// end tag.
const SILENT: [&str; 15] = [
    "script", "noscript", "template", "title", "iframe", "frameset", "object", "applet", "svg",
    "math", "textarea", "select", "xmp", "noembed", "noframes",
];

/// Elements kept, with their content. Everything not here and not in
/// [`SILENT`] is dropped around its content.
const KEPT: [&str; 73] = [
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "label",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
    "body",
    "style",
];

/// Elements with no end tag.
const VOID: [&str; 5] = ["br", "hr", "img", "wbr", "col"];

/// Elements that may carry a `background` picture.
const BACKGROUNDED: [&str; 7] = ["table", "tbody", "thead", "tfoot", "tr", "td", "th"];

/// The style the page starts with, before the sender's. Little: the sender
/// designed the message, and this is their design, not the window's.
const BASE_STYLE: &str = "html,body{margin:0;padding:0;background:#fff}\
.k-body{padding:12px 14px;box-sizing:border-box;min-height:100%;color:#1a1a1a;\
font:14px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",system-ui,sans-serif;\
overflow-wrap:break-word}\
img[data-blocked]{display:inline-block;box-sizing:border-box;background:#f1f1f1;\
outline:1px dashed #c4c4c4;min-width:12px;min-height:12px;color:#777;font-size:11px}";

/// HTML mail as a page, or why not.
pub fn to_page(html: &str, view: &View<'_>) -> Result<Page, Refused> {
    if html.len() > MOST_HTML {
        return Err(Refused::TooBig);
    }
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() + 512);
    let mut at = 0usize;
    let mut depth = 0usize;
    let mut blocked = 0usize;
    let mut links = 0usize;
    // Whether anything worth looking at was written: text, or a picture.
    let mut something = false;
    let mut link: Option<Link> = None;

    while at < bytes.len() {
        if out.len() > MOST_PAGE {
            return Err(Refused::TooBig);
        }
        if bytes[at] != b'<' {
            let next = memchr(bytes, b'<', at).unwrap_or(bytes.len());
            let text = &html[at..next];
            if !text.trim().is_empty() {
                something = true;
            }
            // As written: text is text in the new document too, and an
            // entity the sender escaped stays escaped. The only character
            // that could start markup is the one this run stops at.
            out.push_str(text);
            at = next;
            continue;
        }
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

        if name.is_empty() {
            // `<` with nothing after it that names an element, as in
            // `a < b`: a browser shows it as text. Here it is simply not
            // there — one character, and one that starts markup.
            continue;
        }
        if SILENT.contains(&name.as_str()) && !ending {
            at = find_close(html, &name, at).ok_or(Refused::Unterminated)?;
            continue;
        }
        if name == "style" {
            if ending {
                continue;
            }
            let end = find_close(html, "style", at).ok_or(Refused::Unterminated)?;
            // Up to the `</style`, which is where the browser would stop too.
            let inner_end = html[at..end].rfind("</").map(|i| at + i).unwrap_or(end);
            let (css, lost) = safe_css(&html[at..inner_end], view.remote);
            blocked += lost;
            out.push_str("<style>");
            out.push_str(&css);
            out.push_str("</style>");
            at = end;
            continue;
        }
        let void = VOID.contains(&name.as_str());
        if !ending && !tag.ends_with('/') && !void {
            depth += 1;
            if depth > MOST_DEPTH {
                return Err(Refused::TooDeep);
            }
        } else if ending {
            depth = depth.saturating_sub(1);
        }
        if !KEPT.contains(&name.as_str()) {
            continue;
        }

        if ending {
            if void {
                continue;
            }
            if name == "a" {
                if let Some(open) = link.take() {
                    open.finish(&mut out);
                }
            }
            out.push_str("</");
            out.push_str(if name == "body" { "div" } else { &name });
            out.push('>');
            continue;
        }

        let attrs = attributes(tag);
        match name.as_str() {
            "a" => {
                if let Some(open) = link.take() {
                    open.finish(&mut out);
                }
                out.push_str("<a");
                let target = attr(&attrs, "href").and_then(link_target);
                if let Some(target) = target {
                    links += 1;
                    out.push_str(" href=\"");
                    escape_into(&mut out, view.open);
                    out.push_str("?to=");
                    escape_into(&mut out, &percent_encode(&target));
                    out.push_str("&amp;text=");
                    let insert_at = out.len();
                    out.push_str("\" title=\"");
                    escape_into(&mut out, &target);
                    out.push_str("\" rel=\"noreferrer noopener\"");
                    push_common(&mut out, &attrs, &name, view.remote, &mut blocked);
                    out.push('>');
                    link = Some(Link {
                        insert_at,
                        text_from: out.len(),
                    });
                } else {
                    push_common(&mut out, &attrs, &name, view.remote, &mut blocked);
                    out.push('>');
                }
            }
            "img" => {
                out.push_str("<img");
                let src = attr(&attrs, "src").filter(|src| !src.trim().is_empty());
                // A picture is something to look at — and one left out is
                // something to offer: a message that is one picture from
                // the internet is a page with one thing to say.
                something |= src.is_some();
                match src.and_then(|src| picture(src, view, &mut blocked)) {
                    Some(src) => push_attr(&mut out, "src", &src),
                    None => out.push_str(" data-blocked=\"\""),
                }
                if let Some(alt) = attr(&attrs, "alt") {
                    push_attr(&mut out, "alt", alt);
                }
                push_common(&mut out, &attrs, &name, view.remote, &mut blocked);
                out.push('>');
            }
            "body" => {
                // The body's own look, on a block of its own: the page has a
                // body already, and the sender's colours go on theirs.
                out.push_str("<div");
                let mut style = attr(&attrs, "style")
                    .map(|css| safe_css(css, view.remote))
                    .map(|(css, lost)| {
                        blocked += lost;
                        css
                    })
                    .unwrap_or_default();
                if let Some(colour) = attr(&attrs, "bgcolor").filter(|c| is_colour(c)) {
                    if !style.is_empty() && !style.trim_end().ends_with(';') {
                        style.push(';');
                    }
                    style.push_str("background-color:");
                    style.push_str(colour);
                }
                if !style.is_empty() {
                    push_attr(&mut out, "style", &style);
                }
                for name in ["class", "id", "dir", "lang"] {
                    if let Some(value) = attr(&attrs, name).filter(|v| valid(name, v)) {
                        push_attr(&mut out, name, value);
                    }
                }
                out.push('>');
            }
            _ => {
                out.push('<');
                out.push_str(&name);
                push_common(&mut out, &attrs, &name, view.remote, &mut blocked);
                out.push('>');
            }
        }
    }
    if let Some(open) = link.take() {
        open.finish(&mut out);
    }
    if !something {
        return Err(Refused::Empty);
    }

    let csp = csp_for(view);
    let mut page = String::with_capacity(out.len() + 1024);
    page.push_str("<!doctype html><html><head><meta charset=\"utf-8\">");
    page.push_str("<meta name=\"referrer\" content=\"no-referrer\">");
    page.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    // `sandbox` is a header-only directive; in a `<meta>` it is ignored
    // with a console warning, so it is left out of this copy.
    escape_into(&mut page, csp.trim_end_matches("; sandbox"));
    page.push_str("\"><style>");
    page.push_str(BASE_STYLE);
    page.push_str("</style></head><body><div class=\"k-body\">");
    page.push_str(&out);
    page.push_str("</div></body></html>");
    Ok(Page {
        html: page,
        csp,
        remote_blocked: blocked,
        links,
    })
}

/// The policy the page is served under. Nothing by default; style, because
/// the page is all style; pictures from the app's own addresses, and from
/// the internet only when asked. `sandbox` with no flags: no scripts, no
/// forms, no popups, an origin of its own.
fn csp_for(view: &View<'_>) -> String {
    let mut img = format!("data: {}", view.parts);
    if view.remote {
        img.push_str(" https: http:");
    }
    format!(
        "default-src 'none'; style-src 'unsafe-inline'; img-src {img}; \
         form-action 'none'; base-uri 'none'; sandbox"
    )
}

/// A link being written: where its words go once they are known.
struct Link {
    /// Where in the output the `text=` value is inserted.
    insert_at: usize,
    /// Where the link's own content starts.
    text_from: usize,
}

impl Link {
    fn finish(self, out: &mut String) {
        let content = out.get(self.text_from..).unwrap_or("");
        let words: String = unescape(&without_tags(content))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MOST_LINK_TEXT)
            .collect();
        let mut encoded = String::new();
        escape_into(&mut encoded, &percent_encode(&words));
        out.insert_str(self.insert_at, &encoded);
    }
}

/// A link's real address, when it is one a person could follow: a web
/// page, a mail address, a phone number. Anything else — `javascript:`,
/// `data:`, `file:`, a path relative to nowhere — makes the link not one.
pub fn link_target(href: &str) -> Option<String> {
    // Tabs and newlines inside an address are dropped, as a browser drops
    // them, and whitespace around it as well.
    let href: String = href
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let href = href.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    if href.is_empty() || href.len() > MOST_ADDRESS {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    let allowed = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:");
    allowed.then(|| href.to_string())
}

/// Where a picture is fetched from, or `None` for one that is not shown.
/// Counts a picture on the internet that was not.
fn picture(src: &str, view: &View<'_>, blocked: &mut usize) -> Option<String> {
    let src: String = src
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let src = src.trim();
    if src.is_empty() || src.len() > MOST_HTML {
        return None;
    }
    let lower = src.to_ascii_lowercase();
    if let Some(cid) = lower.strip_prefix("cid:") {
        let cid = cid.trim_matches(|c| c == '<' || c == '>');
        let index = view
            .pictures
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(cid))
            .map(|(_, index)| *index)?;
        return Some(format!("{}{index}", view.parts));
    }
    if is_inline_picture(&lower) {
        return Some(src.to_string());
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        if view.remote {
            return Some(src.to_string());
        }
        *blocked += 1;
    }
    None
}

/// A picture written into the message: `data:image/png;base64,…` and the
/// other kinds mail carries. SVG is not one — it is a document, not a
/// picture, and goes where documents go.
fn is_inline_picture(lower: &str) -> bool {
    ["png", "jpeg", "jpg", "gif", "webp", "bmp"]
        .iter()
        .any(|kind| lower.starts_with(&format!("data:image/{kind};base64,")))
}

/// The attributes kept on any element, after what the element's own branch
/// wrote. `style` is cleaned; the rest are kept when they look like what
/// they are — a width is digits, a colour is a colour.
fn push_common(
    out: &mut String,
    attrs: &[(String, String)],
    element: &str,
    remote: bool,
    blocked: &mut usize,
) {
    const COMMON: [&str; 22] = [
        "style",
        "class",
        "id",
        "dir",
        "lang",
        "align",
        "valign",
        "width",
        "height",
        "bgcolor",
        "border",
        "cellpadding",
        "cellspacing",
        "colspan",
        "rowspan",
        "nowrap",
        "color",
        "face",
        "size",
        "type",
        "start",
        "hspace",
    ];
    for name in COMMON {
        // The link's title is its real address, and the picture's words are
        // written by the picture's branch.
        let Some(value) = attr(attrs, name) else {
            continue;
        };
        match name {
            "style" => {
                let (css, lost) = safe_css(value, remote);
                *blocked += lost;
                if !css.trim().is_empty() {
                    push_attr(out, "style", &css);
                }
            }
            "nowrap" => out.push_str(" nowrap=\"\""),
            _ if valid(name, value) => push_attr(out, name, value),
            _ => {}
        }
    }
    if let Some(value) = attr(attrs, "vspace").filter(|v| valid("vspace", v)) {
        push_attr(out, "vspace", value);
    }
    if element != "a" {
        if let Some(title) = attr(attrs, "title") {
            push_attr(out, "title", title);
        }
    }
    if BACKGROUNDED.contains(&element) {
        if let Some(found) = attr(attrs, "background") {
            let view = View {
                remote,
                open: "",
                parts: "",
                pictures: &[],
            };
            // A `cid:` here would need the parts address, which this does
            // not have; a background is a picture on the internet or
            // nothing, which is what it is in the mail that uses one.
            if let Some(src) = picture(found, &view, blocked).filter(|s| !s.is_empty()) {
                push_attr(out, "background", &src);
            }
        }
    }
}

/// Whether an attribute's value is the kind of thing that attribute holds.
fn valid(name: &str, value: &str) -> bool {
    let measure = |v: &str| {
        !v.is_empty()
            && (v.eq_ignore_ascii_case("auto")
                || v.chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | '%' | '+' | '-')))
    };
    let word = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    match name {
        "width" | "height" | "border" | "cellpadding" | "cellspacing" | "colspan" | "rowspan"
        | "size" | "start" | "hspace" | "vspace" => measure(value),
        "bgcolor" | "color" => is_colour(value),
        "align" | "valign" | "type" => word(value),
        "dir" => matches!(value.to_ascii_lowercase().as_str(), "ltr" | "rtl" | "auto"),
        "class" | "id" | "lang" | "face" => value.len() <= 512,
        _ => false,
    }
}

/// `#rrggbb`, `#rgb`, or a colour's name.
fn is_colour(value: &str) -> bool {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    !value.is_empty() && value.len() <= 24 && value.chars().all(|c| c.is_ascii_alphabetic())
}

/// CSS with nothing in it that fetches or runs, and how many pictures on
/// the internet were taken out of it.
///
/// `url(` is the one way a style sheet reaches out. It is kept for a
/// picture written into the sheet, and for one on the internet when those
/// are allowed; otherwise the function is renamed so the declaration is
/// invalid and dropped. Backslashes go first, because `u\72l(` is `url(`
/// to a CSS parser and would not be to this. `@import` fetches a sheet,
/// `expression(` ran script in one browser and `behavior`/`-moz-binding`
/// in two others; none is a thing mail needs.
fn safe_css(css: &str, remote: bool) -> (String, usize) {
    let css = unescape_css(css);
    let lower = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut blocked = 0;
    let mut at = 0;
    while at < css.len() {
        let rest = &lower[at..];
        if let Some(inside) = rest.strip_prefix("url(") {
            let argument = inside
                .trim_start()
                .trim_start_matches(['"', '\''])
                .trim_start();
            let keep = if is_inline_picture(argument) {
                true
            } else if argument.starts_with("https://") || argument.starts_with("http://") {
                if !remote {
                    blocked += 1;
                }
                remote
            } else {
                false
            };
            out.push_str(if keep { "url(" } else { "no-url(" });
            at += 4;
            continue;
        }
        let renamed = [
            ("@import", "@no-import"),
            ("expression(", "no-expression("),
            ("behavior", "no-behavior"),
            ("-moz-binding", "no-binding"),
        ]
        .iter()
        .find(|(from, _)| rest.starts_with(from));
        if let Some((from, to)) = renamed {
            out.push_str(to);
            at += from.len();
            continue;
        }
        let c = css[at..].chars().next().unwrap_or(' ');
        out.push(c);
        at += c.len_utf8();
    }
    (out, blocked)
}

/// CSS with its escapes written out: `\72` is `r`, `\(` is `(`, so that
/// `u\72l(` reads as the `url(` it is to a CSS parser. A backslash that
/// escapes nothing is dropped; none is needed to say anything mail says.
fn unescape_css(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(chars.next().unwrap_or(' '));
        }
        if hex.is_empty() {
            // `\(` is `(`; a lone backslash at the end is nothing.
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        // One whitespace after a hex escape belongs to the escape.
        if chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if let Some(decoded) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            out.push(decoded);
        }
    }
    out
}

/// A tag's attributes, in order, names lowercased and values unescaped —
/// the first of a name wins, as in a browser. A value is what it is: text.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let rest = tag.trim_start_matches('/');
    // Past the element's name.
    let body = rest
        .find(|c: char| c.is_whitespace() || c == '/')
        .map(|at| &rest[at..])
        .unwrap_or("");
    let bytes = body.as_bytes();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        if at >= bytes.len() {
            break;
        }
        let name_from = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'/' | b'=')
        {
            at += 1;
        }
        let name = body[name_from..at].to_ascii_lowercase();
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = String::new();
        if at < bytes.len() && bytes[at] == b'=' {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if at < bytes.len() && matches!(bytes[at], b'"' | b'\'') {
                let quote = bytes[at];
                at += 1;
                let from = at;
                while at < bytes.len() && bytes[at] != quote {
                    at += 1;
                }
                value = unescape(&body[from..at]);
                at += 1;
            } else {
                let from = at;
                while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                value = unescape(&body[from..at]);
            }
        }
        if name.is_empty() {
            continue;
        }
        if !out.iter().any(|(known, _)| *known == name) {
            out.push((name, value));
        }
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value.as_str())
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape_into(out, value);
    out.push('"');
}

/// Text into markup, as text: the four characters that mean anything in a
/// double-quoted attribute or between tags, and nothing that cannot be
/// written. Every attribute this file writes is double-quoted, so a single
/// quote is a character.
fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() && !matches!(c, '\n' | '\t') => {}
            c => out.push(c),
        }
    }
}

/// `text`, percent-encoded for a query string: everything but the
/// unreserved characters.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A percent-encoded query value, decoded. Bytes that are not UTF-8 are
/// shown as the replacement character rather than guessed at.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                at += 3;
                continue;
            }
        }
        out.push(if bytes[at] == b'+' { b' ' } else { bytes[at] });
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The values of a query string, by name, decoded.
pub fn query_values(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(name), percent_decode(value))
        })
        .collect()
}
