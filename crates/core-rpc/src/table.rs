//! Tables the assistant writes: a spreadsheet file, not prose.
//!
//! "A table of everything I owe, by how much and how pressing it is." "A list
//! of my investments." The answer to that is not a paragraph and not a PDF —
//! it is a file the person opens in Excel or Numbers and sorts and sums for
//! themselves, because the whole point of asking for a table is to go on
//! working with it.
//!
//! Two formats. **xlsx** is the one to write: a figure stays a figure, so the
//! spreadsheet adds it up and shows it in the person's own currency format,
//! the heading row is bold and stays put while they scroll, and the columns
//! are wide enough to read. **csv** is for feeding something else, and it is
//! written with a byte-order mark, because without one Excel reads a file of
//! Mahngebühren as mojibake.
//!
//! The xlsx is written here rather than with a spreadsheet crate because the
//! zip crate is already in this tree for *reading* the xlsx people send as
//! attachments, and what a one-sheet workbook needs is five small XML files.
//! Nothing here reads a spreadsheet; [`crate::readable`] does that.

use std::io::Write;

use serde::{Deserialize, Serialize};

/// What goes in one cell.
///
/// A figure is kept apart from text all the way to the file: written as a
/// number, a spreadsheet will add it up, and written as text it will not, and
/// a column of amounts that cannot be totalled is not much of a table. The
/// assistant hands over whatever JSON it wrote, and [`Cell::from_json`] sorts
/// them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Number(f64),
    Text(String),
}

impl Cell {
    /// A cell from the JSON the assistant wrote.
    ///
    /// A number stays a number. A string that *is* a number is one too: a
    /// model writing a table of amounts will sometimes quote them, and a
    /// column that is all figures but written as text sums to nothing. But
    /// only when the whole of it is a number and nothing is lost by it — not
    /// "1.234,56 €", which is a figure with a currency and a thousands
    /// separator and belongs to a locale this does not know, and not "0049",
    /// where the nought in front is the point.
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Text(String::new()),
            serde_json::Value::Bool(yes) => {
                Self::Text(if *yes { "yes".into() } else { "no".into() })
            }
            serde_json::Value::Number(number) => number
                .as_f64()
                .filter(|number| number.is_finite())
                .map_or_else(|| Self::Text(number.to_string()), Self::Number),
            serde_json::Value::String(text) => Self::from_text(text),
            other => Self::Text(other.to_string()),
        }
    }

    fn from_text(text: &str) -> Self {
        let trimmed = text.trim();
        let plain = trimmed.strip_prefix('-').unwrap_or(trimmed);
        let digits_only = !plain.is_empty()
            && plain.chars().all(|c| c.is_ascii_digit() || c == '.')
            && plain.matches('.').count() <= 1;
        let keeps_its_shape = !plain.starts_with('0') || plain.starts_with("0.");
        if digits_only && keeps_its_shape {
            if let Ok(number) = trimmed.parse::<f64>() {
                if number.is_finite() {
                    return Self::Number(number);
                }
            }
        }
        Self::Text(trimmed.to_string())
    }

    fn text(&self) -> String {
        match self {
            Self::Number(number) => {
                if number.fract() == 0.0 && number.abs() < 1e15 {
                    format!("{number:.0}")
                } else {
                    number.to_string()
                }
            }
            Self::Text(text) => text.clone(),
        }
    }
}

/// A table: one heading row, then the rows in the order they are to be read.
///
/// The order is the assistant's, not ours. "Sorted by how much and how
/// pressing" is a judgement about letters, made where the letters are; sorting
/// the rows again here could only undo it.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

/// Which file to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Xlsx,
    Csv,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Xlsx => "xlsx",
            Self::Csv => "csv",
        }
    }
}

/// Columns of a table, and rows no longer than there are columns.
///
/// A row with a cell too many would land outside the table, and one short is
/// padded: a model that left the last column of one row empty has written a
/// table with a gap in it, which is a table, not an error.
pub fn squared(table: &Table) -> Table {
    let width = table.columns.len();
    Table {
        columns: table.columns.clone(),
        rows: table
            .rows
            .iter()
            .map(|row| {
                let mut row: Vec<Cell> = row.iter().take(width).cloned().collect();
                row.resize(width, Cell::Text(String::new()));
                row
            })
            .collect(),
    }
}

/// The table as CSV: RFC 4180, with a byte-order mark in front.
///
/// Commas and quotes, not semicolons, because that is what every other
/// program means by a CSV. The mark is there for Excel alone, which without
/// one reads a UTF-8 file as the local code page and turns every umlaut in a
/// creditor's name to rubbish.
pub fn csv(table: &Table) -> Vec<u8> {
    let table = squared(table);
    let mut out = String::from("\u{feff}");
    let line = |out: &mut String, cells: Vec<String>| {
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    };
    line(
        &mut out,
        table.columns.iter().map(|name| quoted(name)).collect(),
    );
    for row in &table.rows {
        line(
            &mut out,
            row.iter()
                .map(|cell| match cell {
                    // A figure unquoted, so whatever reads it next reads a
                    // figure.
                    Cell::Number(_) => cell.text(),
                    Cell::Text(text) => quoted(text),
                })
                .collect(),
        );
    }
    out.into_bytes()
}

/// A field with its quotes doubled and the whole wrapped, where it has to be.
fn quoted(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// How wide a column is made: the longest thing in it, within reason.
///
/// Excel's unit is roughly a character of the default font. A column narrower
/// than its heading cannot be read at all, and one as wide as the longest note
/// in it pushes the figures off the screen, so this is bounded at both ends.
fn width_of(heading: &str, cells: impl Iterator<Item = usize>) -> f32 {
    let longest = cells.chain([heading.chars().count()]).max().unwrap_or(8);
    (longest as f32 + 2.0).clamp(10.0, 48.0)
}

/// The table as a one-sheet xlsx workbook.
///
/// Figures as figures, the heading row bold and frozen, the columns wide
/// enough to read. Written by hand because a one-sheet workbook is five small
/// XML files in a zip, and the zip crate is already here.
pub fn xlsx(table: &Table) -> std::io::Result<Vec<u8>> {
    let table = squared(table);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // Stored, not deflated, would work too; deflate because a table of a
    // thousand rows is mostly repeated markup.
    let options: zip::write::FileOptions<'_, '_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, body: String| -> std::io::Result<()> {
        zip.start_file(name, options)?;
        zip.write_all(body.as_bytes())
    };

    put(
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#
            .into(),
    )?;
    put(
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            .into(),
    )?;
    put(
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Tabelle" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            .into(),
    )?;
    put(
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
            .into(),
    )?;
    // Two formats: the heading row bold, everything else plain. No number
    // format of our own — a figure left alone is shown the way the person's
    // own spreadsheet shows figures, which is the way they want to read it.
    // The "Normal" cell style is named because a workbook without one is a
    // workbook some readers warn about and Excel offers to repair.
    put(
        "xl/styles.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
            .into(),
    )?;
    put("xl/worksheets/sheet1.xml", sheet(&table))?;

    Ok(zip.finish()?.into_inner())
}

/// The sheet itself, its parts in the order the schema insists on: the view,
/// the column widths, then the rows.
fn sheet(table: &Table) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
    );

    if !table.columns.is_empty() {
        // The heading row stays put while the table scrolls. A table of debts
        // is read by sorting it another way, which is the whole reason it is a
        // spreadsheet and not a page.
        out.push_str(
            r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#,
        );
        out.push_str("<cols>");
        for (index, heading) in table.columns.iter().enumerate() {
            let width = width_of(
                heading,
                table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(index))
                    .map(|cell| cell.text().chars().count()),
            );
            let at = index + 1;
            out.push_str(&format!(
                r#"<col min="{at}" max="{at}" width="{width:.1}" customWidth="1"/>"#
            ));
        }
        out.push_str("</cols>");
    }

    out.push_str("<sheetData>");
    if !table.columns.is_empty() {
        out.push_str(r#"<row r="1">"#);
        for (index, heading) in table.columns.iter().enumerate() {
            out.push_str(&cell_xml(
                &reference(index, 1),
                &Cell::Text(heading.clone()),
                1,
            ));
        }
        out.push_str("</row>");
    }
    for (number, row) in table.rows.iter().enumerate() {
        let at = number + 2;
        out.push_str(&format!(r#"<row r="{at}">"#));
        for (index, cell) in row.iter().enumerate() {
            out.push_str(&cell_xml(&reference(index, at), cell, 0));
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData></worksheet>");
    out
}

/// A1, B1, … AA1, for a column index and a row number.
fn reference(column: usize, row: usize) -> String {
    let mut letters = Vec::new();
    let mut at = column;
    loop {
        letters.push(b'A' + (at % 26) as u8);
        if at < 26 {
            break;
        }
        at = at / 26 - 1;
    }
    letters.reverse();
    format!("{}{row}", String::from_utf8_lossy(&letters))
}

/// One cell. A figure as a number; text as an inline string, so there is no
/// shared-strings table to keep in step.
fn cell_xml(at: &str, cell: &Cell, style: u8) -> String {
    match cell {
        Cell::Number(number) => format!(r#"<c r="{at}" s="{style}"><v>{number}</v></c>"#),
        Cell::Text(text) if text.is_empty() => format!(r#"<c r="{at}" s="{style}"/>"#),
        Cell::Text(text) => format!(
            r#"<c r="{at}" s="{style}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
            escaped(text)
        ),
    }
}

/// XML's five, and the characters XML has no way to carry at all.
///
/// A control character in a cell makes a workbook Excel refuses to open, and
/// what put it there is a PDF somebody else wrote. Dropped, not escaped:
/// there is no escape for them.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}
