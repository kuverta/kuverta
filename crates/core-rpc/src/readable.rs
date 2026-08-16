//! Files as text a model can read.
//!
//! The assistant writes about files: the ones a person attaches to what they
//! are writing, and the ones a message it answers came with. A model reads
//! text, so each file is turned into some — Markdown where the file had
//! structure worth keeping, headings, lists and tables, because that is what
//! a model reads best and what reads back cleanly if it quotes it.
//!
//! What is read, and how:
//!
//! - **text** of any kind — plain, CSV, Markdown, calendar, contact — as it is;
//!   HTML through the same converter the reading pane uses;
//! - **PDF** by its text layer. A scan has none, and its pages are the
//!   photographs inside it, which go to the vision model as a letter's do;
//! - **Word, LibreOffice, Excel and PowerPoint** files by the XML inside them;
//! - **photographs** (JPEG, PNG) by the vision model, when one is set up;
//! - **a forwarded message** by its headers and text.
//!
//! Everything else is said to be unreadable, by name, rather than guessed at.
//!
//! These files come from anywhere — a PDF someone else sent is still someone
//! else's PDF — so everything here is bounded: how much a zip entry may
//! inflate to, how many rows of a sheet, how many pages go to the vision
//! model. A PDF reader that panics on a malformed file takes only that file's
//! text with it.

use std::io::Read;

use crate::ai::AiChoice;

/// What a file holds, as far as it can be read without a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contents {
    /// Text, ready to read.
    Text(String),
    /// Pictures of pages, JPEG or PNG, for a vision model to read.
    Pages(Vec<Vec<u8>>),
    /// Nothing that can be read, and why, in words.
    Unreadable(String),
}

/// How many pages of one file go to the vision model: each takes it a while.
pub const MOST_PAGES: usize = 5;

/// The most one entry of a zip may inflate to. A Word document's text is
/// kilobytes; a zip that inflates past this is a zip built to.
const MOST_INFLATED: u64 = 32 * 1024 * 1024;

/// Rows of one sheet, and sheets of one workbook, that are read.
const MOST_ROWS: usize = 300;
const MOST_SHEETS: usize = 10;
const MOST_COLUMNS: usize = 40;

/// What kind of file this is, from its type, its name, and its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Html,
    Pdf,
    Image,
    Docx,
    Odf,
    Xlsx,
    Pptx,
    Message,
    Other,
}

fn kind(name: &str, content_type: &str, bytes: &[u8]) -> Kind {
    let content_type = content_type.trim().to_ascii_lowercase();
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if bytes.starts_with(b"%PDF") || content_type == "application/pdf" || extension == "pdf" {
        return Kind::Pdf;
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) || bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Kind::Image;
    }
    match extension.as_str() {
        "docx" | "docm" | "dotx" => return Kind::Docx,
        "odt" | "ott" | "ods" | "odp" => return Kind::Odf,
        "xlsx" | "xlsm" => return Kind::Xlsx,
        "pptx" => return Kind::Pptx,
        "eml" => return Kind::Message,
        "html" | "htm" => return Kind::Html,
        "txt" | "text" | "md" | "markdown" | "csv" | "tsv" | "log" | "json" | "xml" | "ics"
        | "vcf" | "yaml" | "yml" | "toml" | "ini" => return Kind::Text,
        _ => {}
    }
    match content_type.as_str() {
        "text/html" | "application/xhtml+xml" => Kind::Html,
        "message/rfc822" => Kind::Message,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Kind::Docx,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Kind::Xlsx,
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Kind::Pptx,
        ct if ct.starts_with("application/vnd.oasis.opendocument.") => Kind::Odf,
        ct if ct.starts_with("text/") => Kind::Text,
        "application/json" | "application/xml" | "application/csv" => Kind::Text,
        _ => Kind::Other,
    }
}

/// What a file holds, read without a model.
pub fn contents(name: &str, content_type: &str, bytes: &[u8]) -> Contents {
    let unreadable = |why: &str| Contents::Unreadable(why.to_string());
    match kind(name, content_type, bytes) {
        Kind::Text => Contents::Text(decode(bytes)),
        Kind::Html => match crate::html::to_text(&decode(bytes)) {
            Ok(text) => Contents::Text(text),
            Err(_) => unreadable("a web page with no text kuverta could make out"),
        },
        Kind::Pdf => pdf(bytes),
        Kind::Image => Contents::Pages(vec![bytes.to_vec()]),
        Kind::Docx => office(bytes, docx, "a Word document"),
        Kind::Odf => office(bytes, odf, "an OpenDocument file"),
        Kind::Xlsx => office(bytes, xlsx, "an Excel workbook"),
        Kind::Pptx => office(bytes, pptx, "a PowerPoint presentation"),
        Kind::Message => message(bytes),
        Kind::Other => Contents::Unreadable(format!(
            "{} — a kind of file kuverta cannot read",
            if content_type.trim().is_empty() {
                "unknown type"
            } else {
                content_type.trim()
            }
        )),
    }
}

/// A file's text for a model: [`contents`], with pictures read by the vision
/// model when there is one. Never fails: a file that could not be read says
/// so, in brackets, which is as much as the model needs to know.
///
/// `on_page` hears `(page, pages)` before each page goes to the vision model,
/// which is the slow part.
pub async fn read(
    vision: Option<&AiChoice>,
    name: &str,
    content_type: &str,
    bytes: &[u8],
    mut on_page: impl FnMut(usize, usize),
) -> String {
    // In the preview worker when there is one: the file is someone else's,
    // and its parsing belongs where safe preview does it.
    let read = match crate::preview::worker() {
        Some(worker) => crate::preview::contents(worker, name, content_type, bytes),
        None => contents(name, content_type, bytes),
    };
    match read {
        Contents::Text(text) if text.trim().is_empty() => "[the file has no text in it]".into(),
        Contents::Text(text) => text,
        Contents::Unreadable(why) => format!("[not readable: {why}]"),
        Contents::Pages(pages) => {
            let Some(vision) = vision else {
                return "[a picture or a scan: no model that can see is set up to read it]".into();
            };
            let total = pages.len().min(MOST_PAGES);
            let mut out = String::new();
            for (at, page) in pages.iter().take(MOST_PAGES).enumerate() {
                on_page(at + 1, total);
                let text = match crate::ai::read_page(&vision.provider, &vision.model, page).await {
                    Ok(text) if text.trim().is_empty() => "[no text on this page]".to_string(),
                    Ok(text) => text,
                    Err(err) => format!("[this page could not be read: {err}]"),
                };
                if total > 1 {
                    out.push_str(&format!("## Page {}\n\n", at + 1));
                }
                out.push_str(text.trim());
                out.push_str("\n\n");
            }
            if pages.len() > MOST_PAGES {
                out.push_str(&format!(
                    "[{} more pages were not read]\n",
                    pages.len() - MOST_PAGES
                ));
            }
            out.trim_end().to_string()
        }
    }
}

/// `text` cut to at most `most` characters, and saying so when it was.
pub fn clip(text: &str, most: usize) -> String {
    let count = text.chars().count();
    if count <= most {
        return text.to_string();
    }
    let kept: String = text.chars().take(most).collect();
    format!(
        "{kept}\n[… cut here: {} more characters not shown]",
        count - most
    )
}

/// Text in whatever encoding it came in, as well as it can be guessed: UTF-8
/// when it is, Windows-1252 — what an old German CSV is — when it is not.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x80 => '€',
                0x84 => '„',
                0x93 => '“',
                0x94 => '”',
                0x96 => '–',
                0x97 => '—',
                b => b as char,
            })
            .collect(),
    }
}

// -- PDF ------------------------------------------------------------------------

/// Fewer letters than this in a PDF's text layer, and it is a scan.
const SCANNED_BELOW: usize = 40;

fn pdf(bytes: &[u8]) -> Contents {
    // pdf-extract panics on some malformed files rather than failing. That is
    // one file unread, not a thread gone.
    let extracted = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    let text = match extracted {
        Ok(Ok(text)) => tidy(&text),
        _ => String::new(),
    };
    if text.chars().filter(|c| c.is_alphanumeric()).count() >= SCANNED_BELOW {
        return Contents::Text(text);
    }
    let pages = core_paper::scan::jpeg_pages(bytes);
    if !pages.is_empty() {
        return Contents::Pages(pages);
    }
    if text.trim().is_empty() {
        Contents::Unreadable(
            "a PDF with no text in it, whose pages kuverta cannot take apart".into(),
        )
    } else {
        Contents::Text(text)
    }
}

/// Trailing spaces off every line, and no more than one empty line in a row.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut empty = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            empty += 1;
            if empty > 1 {
                continue;
            }
        } else {
            empty = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

// -- zipped XML: Word, OpenDocument, Excel, PowerPoint ----------------------------

type Archive<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

fn office(bytes: &[u8], read: fn(&mut Archive<'_>) -> Option<String>, what: &str) -> Contents {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return Contents::Unreadable(format!("{what} that is not a valid file"));
    };
    match read(&mut archive) {
        Some(text) => Contents::Text(tidy(&text)),
        None => Contents::Unreadable(format!("{what} kuverta could not make sense of")),
    }
}

/// One entry of the archive as text, inflated no further than [`MOST_INFLATED`].
fn entry(archive: &mut Archive<'_>, name: &str) -> Option<String> {
    let file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.take(MOST_INFLATED).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Entries named `{prefix}{n}.xml`, in the order of `n`.
fn numbered(archive: &Archive<'_>, prefix: &str) -> Vec<String> {
    let mut names: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|name| {
            let n = name
                .strip_prefix(prefix)?
                .strip_suffix(".xml")?
                .parse()
                .ok()?;
            Some((n, name.to_string()))
        })
        .collect();
    names.sort();
    names.into_iter().map(|(_, name)| name).collect()
}

fn parse(xml: &str) -> Option<roxmltree::Document<'_>> {
    // DTDs stay refused, as they are by default: an entity that expands to a
    // gigabyte is the classic way to make an XML reader fall over.
    roxmltree::Document::parse(xml).ok()
}

fn named<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn attribute<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name)
        .map(|a| a.value())
}

/// A Markdown table from rows of cells, the first row its head.
fn table(rows: &[Vec<String>]) -> String {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return String::new();
    }
    let cell = |text: &str| {
        text.replace('|', "\\|")
            .replace('\n', " ")
            .trim()
            .to_string()
    };
    let line = |row: &Vec<String>| {
        let mut cells: Vec<String> = row.iter().map(|c| cell(c)).collect();
        cells.resize(width, String::new());
        format!("| {} |\n", cells.join(" | "))
    };
    let mut out = line(&rows[0]);
    out.push_str(&format!("|{}\n", " --- |".repeat(width)));
    for row in &rows[1..] {
        out.push_str(&line(row));
    }
    out
}

/// A Word document: its paragraphs, headings as headings, list items as
/// items, and tables as tables.
fn docx(archive: &mut Archive<'_>) -> Option<String> {
    let xml = entry(archive, "word/document.xml")?;
    let document = parse(&xml)?;
    let body = document.descendants().find(|n| named(*n, "body"))?;
    let mut out = String::new();
    for block in body.children() {
        if named(block, "p") {
            out.push_str(&docx_paragraph(block));
            out.push('\n');
        } else if named(block, "tbl") {
            let rows: Vec<Vec<String>> = block
                .children()
                .filter(|n| named(*n, "tr"))
                .map(|row| {
                    row.children()
                        .filter(|n| named(*n, "tc"))
                        .map(|cell| {
                            cell.descendants()
                                .filter(|n| named(*n, "p"))
                                .map(docx_runs)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect()
                })
                .collect();
            out.push('\n');
            out.push_str(&table(&rows));
            out.push('\n');
        }
    }
    Some(out)
}

/// One Word paragraph as a Markdown line.
fn docx_paragraph(paragraph: roxmltree::Node<'_, '_>) -> String {
    let text = docx_runs(paragraph);
    let properties = paragraph.children().find(|n| named(*n, "pPr"));
    let style = properties
        .and_then(|p| p.children().find(|n| named(*n, "pStyle")))
        .and_then(|s| attribute(s, "val"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let listed = properties.is_some_and(|p| p.children().any(|n| named(n, "numPr")));
    if text.trim().is_empty() {
        return String::new();
    }
    // English Word names its heading styles Heading1…; German Word, which is
    // most of what arrives here, Überschrift1 — stored as "berschrift1".
    let level = ["heading", "berschrift"]
        .iter()
        .find_map(|prefix| style.strip_prefix(prefix)?.trim().parse::<usize>().ok());
    match level {
        Some(level) if (1..=6).contains(&level) => format!("\n{} {text}\n", "#".repeat(level)),
        _ if style == "title" || style == "titel" => format!("\n# {text}\n"),
        _ if listed || style.contains("list") || style.contains("liste") => format!("- {text}"),
        _ => format!("{text}\n"),
    }
}

/// The text of a Word paragraph's runs, tabs and breaks included.
fn docx_runs(paragraph: roxmltree::Node<'_, '_>) -> String {
    let mut text = String::new();
    for node in paragraph.descendants() {
        if named(node, "t") {
            text.push_str(node.text().unwrap_or_default());
        } else if named(node, "tab") {
            text.push('\t');
        } else if named(node, "br") || named(node, "cr") {
            text.push('\n');
        }
    }
    text
}

/// An OpenDocument file — text, spreadsheet or presentation — which keeps
/// all three in one `content.xml` with the same paragraph elements.
fn odf(archive: &mut Archive<'_>) -> Option<String> {
    let xml = entry(archive, "content.xml")?;
    let document = parse(&xml)?;
    let body = document.descendants().find(|n| named(*n, "body"))?;
    let mut out = String::new();
    odf_blocks(body, &mut out, 0);
    Some(out)
}

fn odf_blocks(node: roxmltree::Node<'_, '_>, out: &mut String, depth: usize) {
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "h" => {
                let level = attribute(child, "outline-level")
                    .and_then(|l| l.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 6);
                out.push_str(&format!("\n{} {}\n\n", "#".repeat(level), odf_text(child)));
            }
            "p" => {
                let text = odf_text(child);
                if !text.trim().is_empty() {
                    out.push_str(&text);
                    out.push_str("\n\n");
                }
            }
            "list-item" => {
                let text: Vec<String> = child
                    .children()
                    .filter(|n| named(*n, "p") || named(*n, "h"))
                    .map(odf_text)
                    .collect();
                out.push_str(&format!("{}- {}\n", "  ".repeat(depth), text.join(" ")));
                for nested in child.children().filter(|n| named(*n, "list")) {
                    odf_blocks(nested, out, depth + 1);
                }
            }
            "table" => {
                let rows: Vec<Vec<String>> = child
                    .descendants()
                    .filter(|n| named(*n, "table-row"))
                    .take(MOST_ROWS)
                    .map(|row| {
                        row.children()
                            .filter(|n| named(*n, "table-cell"))
                            .take(MOST_COLUMNS)
                            .map(|cell| {
                                cell.children()
                                    .filter(|n| named(*n, "p"))
                                    .map(odf_text)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .collect()
                    })
                    .filter(|row: &Vec<String>| row.iter().any(|c| !c.trim().is_empty()))
                    .collect();
                if let Some(name) = attribute(child, "name") {
                    out.push_str(&format!("\n## {name}\n\n"));
                }
                out.push_str(&table(&rows));
                out.push('\n');
            }
            // Pages of a presentation, frames, sections: what is inside them.
            _ => odf_blocks(child, out, depth),
        }
    }
}

fn odf_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut text = String::new();
    for part in node.descendants() {
        if part.is_text() {
            text.push_str(part.text().unwrap_or_default());
        } else if named(part, "s") {
            let count = attribute(part, "c")
                .and_then(|c| c.parse().ok())
                .unwrap_or(1);
            text.push_str(&" ".repeat(count.min(100)));
        } else if named(part, "tab") {
            text.push('\t');
        } else if named(part, "line-break") {
            text.push('\n');
        }
    }
    text
}

/// An Excel workbook: each sheet as a table, cells where they stand.
fn xlsx(archive: &mut Archive<'_>) -> Option<String> {
    // Text cells hold an index into this list rather than the text.
    let shared: Vec<String> = entry(archive, "xl/sharedStrings.xml")
        .and_then(|xml| {
            let document = parse(&xml)?;
            Some(
                document
                    .root_element()
                    .children()
                    .filter(|n| named(*n, "si"))
                    .map(|si| {
                        si.descendants()
                            .filter(|n| named(*n, "t"))
                            .filter_map(|t| t.text())
                            .collect()
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    // Sheet names, in the workbook's order — which is the order sheetN.xml
    // are numbered in, for a workbook Excel wrote.
    let names: Vec<String> = entry(archive, "xl/workbook.xml")
        .and_then(|xml| {
            let document = parse(&xml)?;
            Some(
                document
                    .descendants()
                    .filter(|n| named(*n, "sheet"))
                    .filter_map(|n| attribute(n, "name").map(str::to_string))
                    .collect(),
            )
        })
        .unwrap_or_default();

    let sheets = numbered(archive, "xl/worksheets/sheet");
    if sheets.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (at, sheet) in sheets.iter().take(MOST_SHEETS).enumerate() {
        let Some(xml) = entry(archive, sheet) else {
            continue;
        };
        let Some(document) = parse(&xml) else {
            continue;
        };
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut more = 0;
        for row in document.descendants().filter(|n| named(*n, "row")) {
            if rows.len() >= MOST_ROWS {
                more += 1;
                continue;
            }
            let mut cells: Vec<String> = Vec::new();
            for cell in row.children().filter(|n| named(*n, "c")) {
                let column = attribute(cell, "r").map(column_of).unwrap_or(cells.len());
                if column >= MOST_COLUMNS {
                    continue;
                }
                let value = match attribute(cell, "t") {
                    Some("s") => cell
                        .children()
                        .find(|n| named(*n, "v"))
                        .and_then(|v| v.text()?.trim().parse::<usize>().ok())
                        .and_then(|i| shared.get(i).cloned())
                        .unwrap_or_default(),
                    Some("inlineStr") => cell
                        .descendants()
                        .filter(|n| named(*n, "t"))
                        .filter_map(|t| t.text())
                        .collect(),
                    _ => cell
                        .children()
                        .find(|n| named(*n, "v"))
                        .and_then(|v| v.text())
                        .unwrap_or_default()
                        .to_string(),
                };
                if cells.len() <= column {
                    cells.resize(column + 1, String::new());
                }
                cells[column] = value;
            }
            if cells.iter().any(|c| !c.trim().is_empty()) {
                rows.push(cells);
            }
        }
        let name = names
            .get(at)
            .cloned()
            .unwrap_or_else(|| format!("Sheet {}", at + 1));
        out.push_str(&format!("## {name}\n\n"));
        out.push_str(&table(&rows));
        if more > 0 {
            out.push_str(&format!("[{more} more rows not shown]\n"));
        }
        out.push('\n');
    }
    if sheets.len() > MOST_SHEETS {
        out.push_str(&format!(
            "[{} more sheets not shown]\n",
            sheets.len() - MOST_SHEETS
        ));
    }
    Some(out)
}

/// The column of a cell reference: `A1` is 0, `AB12` is 27.
fn column_of(reference: &str) -> usize {
    reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .fold(0, |n, c| {
            n * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)
        })
        .saturating_sub(1)
}

/// A PowerPoint presentation: each slide's text, paragraph by paragraph.
fn pptx(archive: &mut Archive<'_>) -> Option<String> {
    let slides = numbered(archive, "ppt/slides/slide");
    if slides.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (at, slide) in slides.iter().enumerate() {
        let Some(xml) = entry(archive, slide) else {
            continue;
        };
        let Some(document) = parse(&xml) else {
            continue;
        };
        out.push_str(&format!("## Slide {}\n\n", at + 1));
        for paragraph in document.descendants().filter(|n| named(*n, "p")) {
            let text: String = paragraph
                .descendants()
                .filter(|n| named(*n, "t"))
                .filter_map(|t| t.text())
                .collect();
            if !text.trim().is_empty() {
                out.push_str(&text);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    Some(out)
}

// -- a forwarded message ------------------------------------------------------------

fn message(bytes: &[u8]) -> Contents {
    let Some(parsed) = mail_parser::MessageParser::default().parse(bytes) else {
        return Contents::Unreadable("a message that could not be read".into());
    };
    let address = |address: Option<&mail_parser::Address<'_>>| {
        address
            .map(|a| {
                a.iter()
                    .map(|addr| match (&addr.name, &addr.address) {
                        (Some(name), Some(address)) => format!("{name} <{address}>"),
                        (_, Some(address)) => address.to_string(),
                        (Some(name), None) => name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    let body = parsed
        .body_text(0)
        .map(|text| text.to_string())
        .or_else(|| {
            let html = parsed.body_html(0)?;
            crate::html::to_text(&html).ok()
        })
        .unwrap_or_default();
    Contents::Text(format!(
        "From: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}",
        address(parsed.from()),
        address(parsed.to()),
        parsed.date().map(|d| d.to_rfc3339()).unwrap_or_default(),
        parsed.subject().unwrap_or_default(),
        body.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zipped(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut out);
            for (name, content) in entries {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        out.into_inner()
    }

    fn text(contents: Contents) -> String {
        match contents {
            Contents::Text(text) => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn text_is_read_as_it_is_whatever_its_encoding() {
        assert_eq!(
            text(contents("a.csv", "", "Name;Betrag\nMiete;850 €".as_bytes())),
            "Name;Betrag\nMiete;850 €"
        );
        // A CSV from an old German Excel: Windows-1252, not UTF-8.
        assert_eq!(
            text(contents("b.csv", "text/csv", b"Gr\xf6\xdfe;\x80")),
            "Größe;€"
        );
    }

    #[test]
    fn a_word_document_keeps_its_headings_lists_and_tables() {
        let document = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="berschrift1"/></w:pPr><w:r><w:t>Mietvertrag</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Zwischen Erika </w:t></w:r><w:r><w:t>Mustermann und</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>Kaution</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Posten</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Betrag</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Miete</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>850 | warm</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        let read = text(contents(
            "Vertrag.docx",
            "application/octet-stream",
            &zipped(&[("word/document.xml", document)]),
        ));
        assert!(read.starts_with("# Mietvertrag"), "{read}");
        assert!(read.contains("Zwischen Erika Mustermann und"), "{read}");
        assert!(read.contains("- Kaution"), "{read}");
        assert!(read.contains("| Posten | Betrag |"), "{read}");
        assert!(read.contains("| Miete | 850 \\| warm |"), "{read}");
    }

    #[test]
    fn an_excel_sheet_is_a_table_with_cells_where_they_stand() {
        let shared = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<si><t>Monat</t></si><si><t>Summe</t></si><si><t>Januar</t></si></sst>"#;
        let workbook = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets><sheet name="Ausgaben" sheetId="1"/></sheets></workbook>"#;
        let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2" t="s"><v>2</v></c><c r="C2"><v>1234.5</v></c></row>
</sheetData></worksheet>"#;
        let read = text(contents(
            "Haushalt.xlsx",
            "",
            &zipped(&[
                ("xl/sharedStrings.xml", shared),
                ("xl/workbook.xml", workbook),
                ("xl/worksheets/sheet1.xml", sheet),
            ]),
        ));
        assert!(read.contains("## Ausgaben"), "{read}");
        assert!(read.contains("| Monat |  | Summe |"), "{read}");
        assert!(read.contains("| Januar |  | 1234.5 |"), "{read}");
    }

    #[test]
    fn column_references_count_like_a_spreadsheet() {
        assert_eq!(column_of("A1"), 0);
        assert_eq!(column_of("Z9"), 25);
        assert_eq!(column_of("AB12"), 27);
    }

    #[test]
    fn an_opendocument_text_keeps_its_headings_and_lists() {
        let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text>
<text:h text:outline-level="2">Termine</text:h>
<text:p>Bitte<text:s text:c="2"/>bestätigen</text:p>
<text:list><text:list-item><text:p>Montag</text:p></text:list-item></text:list>
</office:text></office:body></office:document-content>"#;
        let read = text(contents("a.odt", "", &zipped(&[("content.xml", content)])));
        assert!(read.contains("## Termine"), "{read}");
        assert!(read.contains("Bitte  bestätigen"), "{read}");
        assert!(read.contains("- Montag"), "{read}");
    }

    #[test]
    fn a_photograph_is_a_page_for_the_vision_model() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 0];
        assert_eq!(
            contents("IMG_1.JPG", "image/jpeg", &jpeg),
            Contents::Pages(vec![jpeg.to_vec()])
        );
    }

    #[test]
    fn a_pdf_is_read_by_its_text() {
        let read = text(contents(
            "Rechnung.pdf",
            "application/pdf",
            &tests_support::pdf_saying(
                "Rechnung Nummer 4711 fuer Erika Mustermann, faellig am 1. Oktober",
            ),
        ));
        assert!(read.contains("Rechnung Nummer 4711"), "{read}");
    }

    #[test]
    fn a_broken_file_says_so_rather_than_failing() {
        assert!(matches!(
            contents("x.pdf", "application/pdf", b"%PDF-1.4 and then nothing"),
            Contents::Unreadable(_)
        ));
        assert!(matches!(
            contents("x.docx", "", b"PK not a zip"),
            Contents::Unreadable(_)
        ));
        assert!(matches!(
            contents("x.bin", "application/x-thing", &[0, 1, 2]),
            Contents::Unreadable(_)
        ));
    }

    #[test]
    fn a_forwarded_message_is_its_headers_and_text() {
        let eml = b"From: Erika Mustermann <erika@example.com>\r\nTo: max@example.com\r\nSubject: Termin\r\n\r\nMontag passt.\r\n";
        let read = text(contents("fwd.eml", "message/rfc822", eml));
        assert!(
            read.contains("From: Erika Mustermann <erika@example.com>"),
            "{read}"
        );
        assert!(read.contains("Subject: Termin"), "{read}");
        assert!(read.ends_with("Montag passt."), "{read}");
    }

    #[test]
    fn clipping_says_how_much_is_missing() {
        assert_eq!(clip("kurz", 10), "kurz");
        let clipped = clip("äöüäöü", 3);
        assert!(clipped.starts_with("äöü\n"), "{clipped}");
        assert!(clipped.contains("3 more characters"), "{clipped}");
    }
}

/// What the tests of this module, of [`crate::preview`] and of
/// [`crate::spots`] make files from.
#[cfg(test)]
pub(crate) mod tests_support {
    /// A one-page PDF with `line` on it, in Helvetica, cross-reference table
    /// and all.
    pub(crate) fn pdf_saying(line: &str) -> Vec<u8> {
        pdf_of(&[format!("BT /F1 12 Tf 72 720 Td ({line}) Tj ET")])
    }

    /// A PDF of one page per content stream, letter-sized, with Helvetica as /F1.
    pub(crate) fn pdf_of(streams: &[String]) -> Vec<u8> {
        // 1 catalog, 2 pages, 3 the font, then a page and its stream each.
        let first = 4;
        let kids: Vec<String> = (0..streams.len())
            .map(|at| format!("{} 0 R", first + at * 2))
            .collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                streams.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_string(),
        ];
        for (at, stream) in streams.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
                first + at * 2 + 1
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (at, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", at + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}
