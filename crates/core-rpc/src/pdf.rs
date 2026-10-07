//! PDFs kuverta writes, and the signature it puts on one.
//!
//! Two things, both plain: a document written from text — a letter, a
//! cancellation, a confirmation, in Helvetica on A4 — and a picture of the
//! person's signature placed on a page of an existing PDF, with the place and
//! the date beneath it when asked. The signature is a picture, as it would be
//! on paper printed, signed and scanned: not a certificate, and the document
//! says nothing about who placed it.
//!
//! Text is written with the standard Helvetica in WinAnsi encoding, which
//! every reader has and which covers German, French and the other Latin
//! languages, the euro sign and typographic quotes. What it cannot encode — a
//! Greek or Cyrillic letter, an emoji — is written as `?` rather than lost.
//! Line widths come from Helvetica's own metrics, so lines break where they
//! would on the page.
//!
//! Stamping leaves the PDF as it was and appends: a new content stream on the
//! page, the picture as an image, and a graphics state that multiplies it
//! onto the page, so the white paper of a scanned signature does not cover
//! the line it is signed on. A page's rotation is honoured, so the signature
//! reads upright on a scan that is stored sideways.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// A point is 1/72 inch; this many of them make a millimetre.
pub const MM: f32 = 72.0 / 25.4;

/// A4, in points.
const PAGE: (f32, f32) = (595.276, 841.89);
const MARGIN: f32 = 20.0 * MM;
const BODY_SIZE: f32 = 11.0;
const BODY_LEADING: f32 = 15.0;
const TITLE_SIZE: f32 = 17.0;
const HEADING_SIZE: f32 = 13.0;
const LIST_INDENT: f32 = 14.0;
/// How wide a signature is drawn in a written document, and at most how tall.
const SIGNATURE_WIDTH: f32 = 45.0 * MM;
const SIGNATURE_HEIGHT: f32 = 25.0 * MM;
/// The gap left for a signature by hand, where the text marks one and none
/// is placed.
const SIGNATURE_GAP: f32 = 14.0 * MM;
const CAPTION_SIZE: f32 = 9.0;

/// The marker in a document's text where the signature goes.
pub const SIGNATURE_MARK: &str = "[signature]";

/// The signature as pixels: 8-bit RGB rows, top row first, and the alpha
/// channel when the picture has one that matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
}

/// Where a signature goes on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// 1-based; `None` is the last page.
    pub page: Option<usize>,
    pub anchor: Anchor,
    /// From the page's lower edge to the signature's lower edge.
    pub above_bottom_mm: f32,
    /// From the page's left edge; overrides the anchor's own x.
    pub x_mm: Option<f32>,
    pub width_mm: f32,
    /// Written beneath, in small type: the place and the date.
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().replace('-', "_").as_str() {
            "bottom_left" | "left" => Some(Self::BottomLeft),
            "bottom_center" | "bottom_centre" | "center" | "centre" | "middle" => {
                Some(Self::BottomCenter)
            }
            "bottom_right" | "right" => Some(Self::BottomRight),
            _ => None,
        }
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            page: None,
            anchor: Anchor::BottomLeft,
            above_bottom_mm: 30.0,
            x_mm: None,
            width_mm: 50.0,
            caption: None,
        }
    }
}

/// A written document.
#[derive(Debug, Clone)]
pub struct Written {
    pub pdf: Vec<u8>,
    pub pages: usize,
    /// The signature was drawn into it.
    pub signed: bool,
}

// -- text --------------------------------------------------------------------

/// One character as WinAnsi, or `None` where the encoding has no place for it.
fn winansi(c: char) -> Option<u8> {
    let code = c as u32;
    match code {
        0x20..=0x7e | 0xa0..=0xff => Some(code as u8),
        _ => Some(match c {
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8a,
            '‹' => 0x8b,
            'Œ' => 0x8c,
            'Ž' => 0x8e,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9a,
            '›' => 0x9b,
            'œ' => 0x9c,
            'ž' => 0x9e,
            'Ÿ' => 0x9f,
            '\u{a0}' | '\u{2009}' | '\u{202f}' => 0x20,
            '\u{2010}' | '\u{2011}' | '\u{2212}' => b'-',
            '\u{2032}' => b'\'',
            '\u{2033}' => b'"',
            _ => return None,
        }),
    }
}

/// A PDF string literal: the text in WinAnsi, with what the syntax reserves
/// escaped and everything above ASCII written as an octal escape, so the
/// content stream stays plain ASCII whatever the text was.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('(');
    for c in text.chars() {
        let byte = winansi(c).unwrap_or(b'?');
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out.push(')');
    out
}

/// Helvetica's advance width for a character, in thousandths of the size.
/// The standard metrics for ASCII; accented letters take their base letter's
/// width, which is what the font does too.
fn helvetica_width(c: char) -> u32 {
    const ASCII: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278,
        278, // ' '..'/'
        556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // digits
        278, 278, 584, 584, 584, 556, 1015, // : ; < = > ? @
        667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722,
        667, 611, 722, 667, 944, 667, 667, 611, // A..Z
        278, 278, 278, 469, 556, 333, // [ \ ] ^ _ `
        556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333,
        500, 278, 556, 500, 722, 500, 500, 500, // a..z
        334, 260, 334, 584, // { | } ~
    ];
    if (' '..='~').contains(&c) {
        return u32::from(ASCII[c as usize - 32]);
    }
    let base = match c {
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' | 'Ø' => 'O',
        'Ù'..='Ü' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        'Š' => 'S',
        'Ž' => 'Z',
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' | 'ø' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'š' => 's',
        'ž' => 'z',
        _ => {
            return match c {
                'ß' => 611,
                '€' | '§' | '£' | '¥' | '«' | '»' | 'µ' => 556,
                '–' => 556,
                '—' | '…' | '™' | 'Æ' | 'Œ' => 1000,
                'æ' => 889,
                'œ' => 944,
                '“' | '”' | '„' | '¡' => 333,
                '‘' | '’' | '‚' => 222,
                '•' => 350,
                '°' => 400,
                '·' | '\u{a0}' => 278,
                '©' | '®' => 737,
                '±' | '×' | '÷' => 584,
                '¿' => 611,
                _ => 556,
            }
        }
    };
    helvetica_width(base)
}

/// How wide `text` is at `size`, in points. Bold is somewhat wider than
/// regular; the headings are measured generously rather than exactly.
fn width(text: &str, size: f32, bold: bool) -> f32 {
    let units: u32 = text.chars().map(helvetica_width).sum();
    units as f32 * size / 1000.0 * if bold { 1.08 } else { 1.0 }
}

/// `text` broken into lines no wider than `most`. A word that is wider on
/// its own is broken where it has to be.
fn wrap(text: &str, size: f32, bold: bool, most: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if width(&candidate, size, bold) <= most {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if width(word, size, bold) <= most {
            line = word.to_string();
            continue;
        }
        for c in word.chars() {
            let longer = format!("{line}{c}");
            if width(&longer, size, bold) > most && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// One line of the text as it is to be set.
#[derive(Debug, PartialEq)]
enum Block<'a> {
    Title(&'a str),
    Heading(&'a str),
    Item(&'a str),
    Line(&'a str),
    Blank,
    Signature,
}

/// Lines are kept as lines: an address block stays one, as does a line of
/// a poem. Only what Markdown marks is read as such — a title, a heading, a
/// list item — and the marker where the signature goes.
fn blocks(text: &str) -> Vec<Block<'_>> {
    text.lines()
        .map(|raw| {
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                Block::Blank
            } else if trimmed.eq_ignore_ascii_case(SIGNATURE_MARK)
                || trimmed.eq_ignore_ascii_case("[unterschrift]")
            {
                Block::Signature
            } else if let Some(rest) = trimmed.strip_prefix("# ") {
                Block::Title(rest.trim())
            } else if let Some(rest) = trimmed
                .strip_prefix("## ")
                .or_else(|| trimmed.strip_prefix("### "))
            {
                Block::Heading(rest.trim())
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("• "))
            {
                Block::Item(rest.trim())
            } else if trimmed == "---" || trimmed == "***" {
                Block::Blank
            } else {
                Block::Line(trimmed)
            }
        })
        .collect()
}

/// The pages as they are set: a content stream each, written top down.
struct Setter<'a> {
    pages: Vec<String>,
    current: String,
    y: f32,
    signature: Option<&'a Pixels>,
    /// Written beneath the signature, in small type: the place and the date.
    caption: Option<&'a str>,
    signed: bool,
}

impl<'a> Setter<'a> {
    fn new(signature: Option<&'a Pixels>, caption: Option<&'a str>) -> Self {
        Self {
            pages: Vec::new(),
            current: String::new(),
            y: PAGE.1 - MARGIN,
            signature,
            caption,
            signed: false,
        }
    }

    fn page_break(&mut self) {
        self.pages.push(std::mem::take(&mut self.current));
        self.y = PAGE.1 - MARGIN;
    }

    /// Makes room for `height` more, on this page or the next.
    fn need(&mut self, height: f32) {
        if self.y - height < MARGIN && self.y < PAGE.1 - MARGIN - 0.5 {
            self.page_break();
        }
    }

    fn text(&mut self, line: &str, size: f32, bold: bool, x: f32, leading: f32) {
        self.need(leading);
        self.y -= leading;
        let font = if bold { "F2" } else { "F1" };
        self.current.push_str(&format!(
            "BT /{font} {size} Tf {x:.2} {y:.2} Td {literal} Tj ET\n",
            y = self.y,
            literal = literal(line)
        ));
    }

    fn paragraph(&mut self, text: &str, size: f32, bold: bool, x: f32, leading: f32) {
        for line in wrap(text, size, bold, PAGE.0 - MARGIN - x) {
            self.text(&line, size, bold, x, leading);
        }
    }

    fn space(&mut self, height: f32) {
        if self.y < PAGE.1 - MARGIN - 0.5 {
            self.y -= height;
        }
    }

    fn signature(&mut self) {
        let Some(pixels) = self.signature else {
            self.need(SIGNATURE_GAP);
            self.y -= SIGNATURE_GAP;
            return;
        };
        let (w, h) = fit(pixels, SIGNATURE_WIDTH, SIGNATURE_HEIGHT);
        let caption = self.caption.map(str::trim).filter(|c| !c.is_empty());
        let below = if caption.is_some() {
            CAPTION_SIZE + 4.0
        } else {
            0.0
        };
        self.need(h + 6.0 + below);
        self.y -= h;
        self.current.push_str(&format!(
            "q /GS1 gs {w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm /Sig Do Q\n",
            x = MARGIN,
            y = self.y
        ));
        if let Some(caption) = caption {
            self.y -= CAPTION_SIZE + 2.0;
            self.current.push_str(&format!(
                "BT /F1 {CAPTION_SIZE} Tf {x:.2} {y:.2} Td {text} Tj ET\n",
                x = MARGIN,
                y = self.y,
                text = literal(caption)
            ));
            self.y -= 2.0;
        }
        self.y -= 6.0;
        self.signed = true;
    }

    fn finish(mut self) -> Vec<String> {
        self.pages.push(self.current);
        self.pages
    }
}

/// The size a picture is drawn at: `width` wide, unless that makes it taller
/// than `most_height`.
fn fit(pixels: &Pixels, width: f32, most_height: f32) -> (f32, f32) {
    let ratio = pixels.height as f32 / pixels.width.max(1) as f32;
    let height = width * ratio;
    if height <= most_height {
        (width, height)
    } else {
        (most_height / ratio, most_height)
    }
}

/// Writes a document. `sign` draws the signature where the text marks it, or
/// after the text when it does not, with `caption` — the place and the date
/// — beneath it; without `sign`, a marker leaves a gap to sign by hand.
pub fn write(
    title: &str,
    text: &str,
    signature: Option<&Pixels>,
    sign: bool,
    caption: Option<&str>,
) -> Result<Written, String> {
    if sign && signature.is_none() {
        return Err("there is no signature to sign with".into());
    }
    let drawn = if sign { signature } else { None };
    let mut setter = Setter::new(drawn, caption);
    let blocks = blocks(text);
    let mut marked = false;
    for (at, block) in blocks.iter().enumerate() {
        match block {
            Block::Title(line) => {
                setter.paragraph(line, TITLE_SIZE, true, MARGIN, TITLE_SIZE * 1.3);
                setter.space(8.0);
            }
            Block::Heading(line) => {
                setter.space(6.0);
                setter.paragraph(line, HEADING_SIZE, true, MARGIN, HEADING_SIZE * 1.3);
                setter.space(2.0);
            }
            Block::Item(line) => {
                let before = setter.y;
                let page = setter.pages.len();
                setter.paragraph(line, BODY_SIZE, false, MARGIN + LIST_INDENT, BODY_LEADING);
                // The bullet, beside the first line — wherever that ended up.
                let y = if setter.pages.len() == page {
                    before - BODY_LEADING
                } else {
                    PAGE.1 - MARGIN - BODY_LEADING
                };
                setter.current.push_str(&format!(
                    "BT /F1 {BODY_SIZE} Tf {x:.2} {y:.2} Td {bullet} Tj ET\n",
                    x = MARGIN + 3.0,
                    bullet = literal("•")
                ));
            }
            Block::Line(line) => setter.paragraph(line, BODY_SIZE, false, MARGIN, BODY_LEADING),
            Block::Blank => {
                // Several blank lines are one break, except that the gap
                // above a signature is kept: it is the room for it.
                let previous = at.checked_sub(1).map(|i| &blocks[i]);
                if !matches!(previous, Some(Block::Blank)) {
                    setter.space(BODY_LEADING * 0.6);
                }
            }
            Block::Signature => {
                marked = true;
                setter.space(4.0);
                setter.signature();
            }
        }
    }
    if sign && !marked {
        setter.space(BODY_LEADING);
        setter.signature();
    }
    let signed = setter.signed;
    let pages = setter.finish();
    let count = pages.len();
    let pdf = assemble(title, pages, drawn)?;
    Ok(Written {
        pdf,
        pages: count,
        signed,
    })
}

/// The document's objects: fonts, the signature when drawn, a page per
/// content stream, the page tree, the catalog.
fn assemble(
    title: &str,
    pages: Vec<String>,
    signature: Option<&Pixels>,
) -> Result<Vec<u8>, String> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let regular = doc.add_object(font("Helvetica"));
    let bold = doc.add_object(font("Helvetica-Bold"));
    let mut fonts = Dictionary::new();
    fonts.set("F1", regular);
    fonts.set("F2", bold);
    let mut resources = Dictionary::new();
    resources.set("Font", fonts);
    if let Some(pixels) = signature {
        let image_id = add_image(&mut doc, pixels);
        let state_id = doc.add_object(multiply_state());
        let mut xobjects = Dictionary::new();
        xobjects.set("Sig", image_id);
        resources.set("XObject", xobjects);
        let mut states = Dictionary::new();
        states.set("GS1", state_id);
        resources.set("ExtGState", states);
    }
    let resources_id = doc.add_object(resources);

    let mut kids = Vec::new();
    for content in pages {
        let mut stream = Stream::new(Dictionary::new(), content.into_bytes());
        let _ = stream.compress();
        let content_id = doc.add_object(stream);
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", pages_id);
        page.set(
            "MediaBox",
            vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Real(PAGE.0),
                Object::Real(PAGE.1),
            ],
        );
        page.set("Contents", content_id);
        page.set("Resources", resources_id);
        kids.push(Object::Reference(doc.add_object(page)));
    }
    let count = kids.len() as i64;
    let mut tree = Dictionary::new();
    tree.set("Type", Object::Name(b"Pages".to_vec()));
    tree.set("Kids", kids);
    tree.set("Count", count);
    doc.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(catalog);
    let mut info = Dictionary::new();
    info.set("Producer", Object::string_literal("kuverta"));
    if !title.trim().is_empty() {
        info.set("Title", Object::string_literal(pdf_doc_string(title)));
    }
    let info_id = doc.add_object(info);
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);
    save(&mut doc)
}

/// A string for the document's own dictionaries, in PDFDocEncoding, which is
/// Latin-1 where it matters; the rest is `?`.
fn pdf_doc_string(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' })
        .collect()
}

fn font(base: &str) -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(base.as_bytes().to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    font
}

/// The graphics state that multiplies the signature onto the page: ink
/// darkens what is under it, and the white of a scanned signature's paper
/// leaves it as it was.
fn multiply_state() -> Dictionary {
    let mut state = Dictionary::new();
    state.set("Type", Object::Name(b"ExtGState".to_vec()));
    state.set("BM", Object::Name(b"Multiply".to_vec()));
    state
}

/// The signature as an image object, with its alpha as a soft mask when it
/// has one.
fn add_image(doc: &mut Document, pixels: &Pixels) -> ObjectId {
    let mask = pixels.alpha.as_ref().map(|alpha| {
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Width", i64::from(pixels.width));
        dict.set("Height", i64::from(pixels.height));
        dict.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
        dict.set("BitsPerComponent", 8i64);
        let mut stream = Stream::new(dict, alpha.clone());
        let _ = stream.compress();
        doc.add_object(stream)
    });
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    dict.set("Width", i64::from(pixels.width));
    dict.set("Height", i64::from(pixels.height));
    dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    dict.set("BitsPerComponent", 8i64);
    if let Some(mask) = mask {
        dict.set("SMask", mask);
    }
    let mut stream = Stream::new(dict, pixels.rgb.clone());
    let _ = stream.compress();
    doc.add_object(stream)
}

fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|err| format!("the PDF could not be written: {err}"))?;
    Ok(out)
}

// -- stamping ----------------------------------------------------------------

/// Loads a PDF someone else wrote. lopdf can panic on a malformed one; that
/// is this file refused, not the thread gone.
pub(crate) fn load(pdf: &[u8]) -> Result<Document, String> {
    let loaded = std::panic::catch_unwind(|| Document::load_mem(pdf));
    let doc = match loaded {
        Ok(Ok(doc)) => doc,
        Ok(Err(err)) => return Err(format!("this is not a PDF kuverta can read: {err}")),
        Err(_) => return Err("this is not a PDF kuverta can read".into()),
    };
    if doc.is_encrypted() {
        return Err("this PDF is protected with a password, so nothing can be put on it".into());
    }
    Ok(doc)
}

/// Every page's size in millimetres, as a reader shows it: a page stored
/// sideways is measured the way it is read.
pub fn page_sizes_mm(pdf: &[u8]) -> Vec<(f32, f32)> {
    let Ok(doc) = load(pdf) else {
        return Vec::new();
    };
    doc.get_pages()
        .into_values()
        .map(|page_id| {
            let (boxed, rotate) = page_geometry(&doc, page_id);
            let (_, (w, h)) = display_to_page(boxed, rotate);
            (w / MM, h / MM)
        })
        .collect()
}

/// How many pages a PDF has, when it can be read.
pub fn page_count(pdf: &[u8]) -> Option<usize> {
    load(pdf).ok().map(|doc| doc.get_pages().len())
}

/// A page's own value for `key`, or the one it inherits from the page tree.
fn inherited<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = page_id;
    for _ in 0..64 {
        let dict = doc.get_dictionary(node).ok()?;
        if let Ok(value) = dict.get(key) {
            return doc.dereference(value).ok().map(|(_, object)| object);
        }
        node = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn rectangle(object: Option<&Object>) -> Option<[f32; 4]> {
    let array = object?.as_array().ok()?;
    if array.len() != 4 {
        return None;
    }
    let mut out = [0.0; 4];
    for (slot, value) in out.iter_mut().zip(array) {
        *slot = value.as_float().ok()?;
    }
    // Normalised: a box may be given from any two corners.
    Some([
        out[0].min(out[2]),
        out[1].min(out[3]),
        out[0].max(out[2]),
        out[1].max(out[3]),
    ])
}

/// The page's visible box and its rotation, as a reader shows it.
pub(crate) fn page_geometry(doc: &Document, page_id: ObjectId) -> ([f32; 4], i64) {
    let media =
        rectangle(inherited(doc, page_id, b"MediaBox")).unwrap_or([0.0, 0.0, PAGE.0, PAGE.1]);
    let boxed = rectangle(inherited(doc, page_id, b"CropBox"))
        .map(|crop| {
            [
                crop[0].max(media[0]),
                crop[1].max(media[1]),
                crop[2].min(media[2]),
                crop[3].min(media[3]),
            ]
        })
        .filter(|crop| crop[2] > crop[0] && crop[3] > crop[1])
        .unwrap_or(media);
    let rotate = inherited(doc, page_id, b"Rotate")
        .and_then(|r| r.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360);
    (boxed, rotate)
}

/// The page's resources as a dictionary of its own — a copy of what it
/// inherited or shared, so that adding to it changes no other page.
fn own_resources(doc: &mut Document, page_id: ObjectId) -> Result<Dictionary, String> {
    let found = inherited(doc, page_id, b"Resources")
        .and_then(|r| r.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    // Sub-dictionaries may themselves be references shared between pages.
    let mut resources = Dictionary::new();
    for (key, value) in found.iter() {
        let value = match doc.dereference(value) {
            Ok((_, object)) => object.clone(),
            Err(_) => value.clone(),
        };
        resources.set(key.clone(), value);
    }
    Ok(resources)
}

fn add_to(resources: &mut Dictionary, kind: &str, name: &str, id: ObjectId) {
    let mut entries = resources
        .get(kind.as_bytes())
        .ok()
        .and_then(|e| e.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    entries.set(name, id);
    resources.set(kind, entries);
}

/// The transform from the page as shown — x right, y up, origin at its lower
/// left corner — to the page's own coordinates, honouring its rotation.
pub(crate) fn display_to_page(boxed: [f32; 4], rotate: i64) -> ([f32; 6], (f32, f32)) {
    let (x0, y0, x1, y1) = (boxed[0], boxed[1], boxed[2], boxed[3]);
    let (w, h) = (x1 - x0, y1 - y0);
    match rotate {
        90 => ([0.0, 1.0, -1.0, 0.0, x1, y0], (h, w)),
        180 => ([-1.0, 0.0, 0.0, -1.0, x1, y1], (w, h)),
        270 => ([0.0, -1.0, 1.0, 0.0, x0, y1], (h, w)),
        _ => ([1.0, 0.0, 0.0, 1.0, x0, y0], (w, h)),
    }
}

/// The other way about: a point in the page's own coordinates, as the page
/// shows it. What [`display_to_page`] builds is a turn and a shift, so this
/// is its inverse — which is what reading a place off a page needs.
pub(crate) fn to_display(matrix: [f32; 6], point: (f32, f32)) -> (f32, f32) {
    let [a, b, c, d, e, f] = matrix;
    let det = a * d - b * c;
    if det == 0.0 {
        return point;
    }
    let (x, y) = (point.0 - e, point.1 - f);
    ((x * d - y * c) / det, (y * a - x * b) / det)
}

/// Puts the signature on a page of `pdf`, and returns the new PDF and the
/// page it went on, 1-based.
pub fn stamp(pdf: &[u8], pixels: &Pixels, at: &Placement) -> Result<(Vec<u8>, usize), String> {
    let mut doc = load(pdf)?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err("this PDF has no pages".into());
    }
    let number = match at.page {
        Some(n) if n >= 1 && n <= pages.len() => n,
        Some(n) => {
            return Err(format!(
                "there is no page {n}: the document has {}",
                pages.len()
            ))
        }
        None => pages.len(),
    };
    let page_id = pages[&(number as u32)];
    let (boxed, rotate) = page_geometry(&doc, page_id);
    let (matrix, (shown_w, shown_h)) = display_to_page(boxed, rotate);

    let width = (at.width_mm.clamp(10.0, 150.0) * MM).min(shown_w * 0.9);
    let (w, h) = fit(pixels, width, shown_h * 0.4);
    let x = match at.x_mm {
        Some(x) => x * MM,
        None => match at.anchor {
            Anchor::BottomLeft => MARGIN,
            Anchor::BottomCenter => (shown_w - w) / 2.0,
            Anchor::BottomRight => shown_w - MARGIN - w,
        },
    }
    .clamp(0.0, (shown_w - w).max(0.0));
    let y = (at.above_bottom_mm * MM).clamp(0.0, (shown_h - h).max(0.0));

    let image_id = add_image(&mut doc, pixels);
    let state_id = doc.add_object(multiply_state());
    let stamp = format!("KuvSig{}", image_id.0);
    let state = format!("KuvGS{}", state_id.0);
    let font_name = format!("KuvF{}", image_id.0);
    let mut resources = own_resources(&mut doc, page_id)?;
    add_to(&mut resources, "XObject", &stamp, image_id);
    add_to(&mut resources, "ExtGState", &state, state_id);
    let mut content = format!(
        "q {a} {b} {c} {d} {e:.3} {f:.3} cm\nq /{state} gs {w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm /{stamp} Do Q\n",
        a = matrix[0],
        b = matrix[1],
        c = matrix[2],
        d = matrix[3],
        e = matrix[4],
        f = matrix[5],
    );
    if let Some(caption) = at.caption.as_deref().filter(|c| !c.trim().is_empty()) {
        let font_id = doc.add_object(font("Helvetica"));
        add_to(&mut resources, "Font", &font_name, font_id);
        content.push_str(&format!(
            "BT /{font_name} {CAPTION_SIZE} Tf {x:.2} {cy:.2} Td {text} Tj ET\n",
            cy = (y - CAPTION_SIZE - 2.0).max(2.0),
            text = literal(caption.trim())
        ));
    }
    content.push_str("Q\n");
    let page = doc
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|err| format!("the page could not be changed: {err}"))?;
    page.set("Resources", resources);
    // Appended as a stream of its own, after the page's content, so the
    // page's own drawing is left exactly as it was — and wrapped in q/Q,
    // so a content stream that left its state unbalanced cannot move it.
    doc.add_page_contents(page_id, content.into_bytes())
        .map_err(|err| format!("the page could not be changed: {err}"))?;
    Ok((save(&mut doc)?, number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels() -> Pixels {
        // Two by two: black ink on white, with a transparent corner.
        Pixels {
            width: 2,
            height: 2,
            rgb: vec![0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0],
            alpha: Some(vec![255, 255, 0, 255]),
        }
    }

    fn text_of(pdf: &[u8]) -> String {
        pdf_extract::extract_text_from_mem(pdf).unwrap()
    }

    #[test]
    fn winansi_covers_german_and_the_typographic_marks() {
        assert_eq!(winansi('ä'), Some(0xe4));
        assert_eq!(winansi('ß'), Some(0xdf));
        assert_eq!(winansi('€'), Some(0x80));
        assert_eq!(winansi('„'), Some(0x84));
        assert_eq!(winansi('–'), Some(0x96));
        assert_eq!(winansi('→'), None);
        assert_eq!(literal("a(b)\\ ä→"), "(a\\(b\\)\\\\ \\344?)");
    }

    #[test]
    fn lines_wrap_at_the_width_and_long_words_are_broken() {
        let lines = wrap(
            "Sehr geehrte Damen und Herren, hiermit kündige ich",
            11.0,
            false,
            120.0,
        );
        assert!(lines.len() >= 3, "{lines:?}");
        for line in &lines {
            assert!(width(line, 11.0, false) <= 120.0, "{line}");
        }
        let broken = wrap(
            "Donaudampfschifffahrtsgesellschaftskapitän",
            11.0,
            false,
            60.0,
        );
        assert!(broken.len() > 1);
        assert_eq!(wrap("", 11.0, false, 100.0), vec![String::new()]);
    }

    #[test]
    fn the_text_is_read_as_lines_headings_items_and_the_mark() {
        let found = blocks("# Kündigung\n\nErika Mustermann\nMusterstraße 1\n- eins\n* zwei\n[Signature]\n[unterschrift]\n---");
        assert_eq!(found[0], Block::Title("Kündigung"));
        assert_eq!(found[1], Block::Blank);
        assert_eq!(found[2], Block::Line("Erika Mustermann"));
        assert_eq!(found[3], Block::Line("Musterstraße 1"));
        assert_eq!(found[4], Block::Item("eins"));
        assert_eq!(found[5], Block::Item("zwei"));
        assert_eq!(found[6], Block::Signature);
        assert_eq!(found[7], Block::Signature);
        assert_eq!(found[8], Block::Blank);
    }

    #[test]
    fn a_written_document_reads_back_and_runs_to_pages() {
        let written = write(
            "Kündigung",
            "# Kündigung\n\nSehr geehrte Damen und Herren,\n\nhiermit kündige ich meinen Vertrag Nr. 4711 zum nächstmöglichen Zeitpunkt.\n\nMit freundlichen Grüßen\n\n[signature]\nErika Mustermann",
            None,
            false,
            None,
        )
        .unwrap();
        assert_eq!(written.pages, 1);
        assert!(!written.signed);
        assert!(written.pdf.starts_with(b"%PDF-1.5"));
        let text = text_of(&written.pdf);
        assert!(text.contains("Kündigung"), "{text}");
        assert!(text.contains("4711"), "{text}");
        assert!(text.contains("Erika Mustermann"), "{text}");
        assert!(!text.contains("[signature]"), "{text}");

        let long = (1..=200)
            .map(|n| format!("Zeile {n}: ein Satz, der auf der Seite steht."))
            .collect::<Vec<_>>()
            .join("\n");
        let many = write("Lang", &long, None, false, None).unwrap();
        assert!(many.pages >= 4, "{}", many.pages);
        assert_eq!(page_count(&many.pdf), Some(many.pages));
        assert!(text_of(&many.pdf).contains("Zeile 200"));
    }

    #[test]
    fn signing_a_written_document_draws_the_picture_once() {
        let marked = write(
            "x",
            "Text\n\n[signature]\nName",
            Some(&pixels()),
            true,
            Some("Musterstadt, 07.10.2026"),
        )
        .unwrap();
        assert!(marked.signed);
        let text = text_of(&marked.pdf);
        assert!(text.contains("Musterstadt, 07.10.2026"), "{text}");
        // The caption sits between the signature and the printed name.
        assert!(
            text.find("Musterstadt").unwrap() < text.find("Name").unwrap(),
            "{text}"
        );
        let doc = Document::load_mem(&marked.pdf).unwrap();
        let page = doc.get_pages()[&1];
        assert_eq!(doc.get_page_images(page).unwrap().len(), 1);
        let content = String::from_utf8_lossy(&doc.get_page_content(page).unwrap()).to_string();
        assert_eq!(content.matches("/Sig Do").count(), 1);
        assert!(content.contains("/GS1 gs"));

        let unmarked = write("x", "Text only", Some(&pixels()), true, None).unwrap();
        assert!(unmarked.signed);
        assert!(write("x", "Text", None, true, None).is_err());
    }

    #[test]
    fn a_signature_is_stamped_on_the_page_asked_for_with_its_caption() {
        let original = crate::readable::tests_support::pdf_saying("Mietvertrag Seite 1");
        let (signed, page) = stamp(
            &original,
            &pixels(),
            &Placement {
                caption: Some("Musterstadt, 06.10.2026".into()),
                ..Placement::default()
            },
        )
        .unwrap();
        assert_eq!(page, 1);
        let text = text_of(&signed);
        assert!(text.contains("Mietvertrag Seite 1"), "{text}");
        assert!(text.contains("Musterstadt, 06.10.2026"), "{text}");
        let doc = Document::load_mem(&signed).unwrap();
        let page_id = doc.get_pages()[&1];
        assert_eq!(doc.get_page_images(page_id).unwrap().len(), 1);
        // The original's own fonts are still there beside the new one.
        let fonts = doc.get_page_fonts(page_id).unwrap();
        assert!(fonts.contains_key(&b"F1"[..]), "{:?}", fonts.keys());
        assert!(fonts.keys().any(|k| k.starts_with(b"KuvF")));
        assert!(stamp(
            &original,
            &pixels(),
            &Placement {
                page: Some(2),
                ..Placement::default()
            }
        )
        .is_err());
        assert!(stamp(b"not a pdf", &pixels(), &Placement::default()).is_err());
    }

    #[test]
    fn a_rotated_page_is_signed_as_it_is_shown() {
        let mut doc = Document::load_mem(&crate::readable::tests_support::pdf_saying("x")).unwrap();
        let page_id = doc.get_pages()[&1];
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Rotate", 90i64);
        let mut rotated = Vec::new();
        doc.save_to(&mut rotated).unwrap();
        let (signed, _) = stamp(&rotated, &pixels(), &Placement::default()).unwrap();
        let doc = Document::load_mem(&signed).unwrap();
        let page_id = doc.get_pages()[&1];
        let content = String::from_utf8_lossy(&doc.get_page_content(page_id).unwrap()).to_string();
        // The display-to-page transform for a quarter turn.
        assert!(content.contains("q 0 1 -1 0 612.000 0.000 cm"), "{content}");
    }

    #[test]
    fn the_display_transform_maps_the_shown_corners_onto_the_page() {
        let boxed = [0.0, 0.0, 612.0, 792.0];
        let apply =
            |m: [f32; 6], x: f32, y: f32| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        let (m, (w, h)) = display_to_page(boxed, 90);
        assert_eq!((w, h), (792.0, 612.0));
        // Shown lower-left is the page's lower-right; shown lower-right is
        // the page's upper-right.
        assert_eq!(apply(m, 0.0, 0.0), (612.0, 0.0));
        assert_eq!(apply(m, 792.0, 0.0), (612.0, 792.0));
        let (m, _) = display_to_page(boxed, 270);
        assert_eq!(apply(m, 0.0, 0.0), (0.0, 792.0));
        assert_eq!(apply(m, 792.0, 0.0), (0.0, 0.0));
        let (m, _) = display_to_page(boxed, 180);
        assert_eq!(apply(m, 0.0, 0.0), (612.0, 792.0));
    }
}
