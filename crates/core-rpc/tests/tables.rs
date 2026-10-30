//! Tables the assistant writes: the file has to be one a spreadsheet opens,
//! and a figure in it has to still be a figure.

use core_rpc::table::{csv, squared, xlsx, Cell, Format, Table};

fn debts() -> Table {
    Table {
        columns: vec![
            "Gläubiger".into(),
            "Betrag (EUR)".into(),
            "Fällig".into(),
            "Quelle".into(),
        ],
        rows: vec![
            vec![
                Cell::Text("Stadtwerke".into()),
                Cell::Number(1234.56),
                Cell::Text("2026-11-15".into()),
                Cell::Text("Mahnung, 2026-10-02".into()),
            ],
            vec![
                Cell::Text("Erika Mustermann".into()),
                Cell::Number(80.0),
                Cell::Text("2026-12-01".into()),
                Cell::Text("Rechnung \"Nr. 7\", 2026-09-30".into()),
            ],
        ],
    }
}

#[test]
fn a_figure_written_as_text_is_still_a_figure() {
    // A model writing a table of amounts will sometimes quote them, and a
    // column of figures written as text sums to nothing.
    assert_eq!(
        Cell::from_json(&serde_json::json!("1234.56")),
        Cell::Number(1234.56)
    );
    assert_eq!(
        Cell::from_json(&serde_json::json!(1234.56)),
        Cell::Number(1234.56)
    );
    assert_eq!(
        Cell::from_json(&serde_json::json!("-80")),
        Cell::Number(-80.0)
    );
}

#[test]
fn what_only_looks_like_a_figure_stays_text() {
    // Each of these loses something if it is made a number.
    for text in ["1.234,56 €", "0049", "2026-11-15", "12 Raten", "", "1.2.3"] {
        assert_eq!(
            Cell::from_json(&serde_json::json!(text)),
            Cell::Text(text.trim().to_string()),
            "{text} should have stayed text"
        );
    }
}

#[test]
fn rows_are_as_wide_as_the_columns() {
    // A row with a cell too many would land outside the table; one short is a
    // table with a gap in it, which is a table.
    let table = squared(&Table {
        columns: vec!["a".into(), "b".into()],
        rows: vec![
            vec![Cell::Text("one".into())],
            vec![
                Cell::Text("x".into()),
                Cell::Text("y".into()),
                Cell::Text("z".into()),
            ],
        ],
    });
    assert_eq!(table.rows[0].len(), 2);
    assert_eq!(table.rows[0][1], Cell::Text(String::new()));
    assert_eq!(table.rows[1].len(), 2);
}

#[test]
fn the_csv_is_rfc_4180_with_a_mark_in_front() {
    let out = csv(&debts());
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.starts_with('\u{feff}'),
        "Excel needs the mark to read UTF-8"
    );
    let lines: Vec<&str> = text.trim_start_matches('\u{feff}').split("\r\n").collect();
    assert_eq!(lines[0], "Gläubiger,Betrag (EUR),Fällig,Quelle");
    // The figure unquoted, so whatever reads it next reads a figure; the
    // quotes inside a field doubled.
    assert_eq!(
        lines[1],
        "Stadtwerke,1234.56,2026-11-15,\"Mahnung, 2026-10-02\""
    );
    assert_eq!(
        lines[2],
        "Erika Mustermann,80,2026-12-01,\"Rechnung \"\"Nr. 7\"\", 2026-09-30\""
    );
}

/// One entry of the workbook, as text.
fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut out = String::new();
    std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut out).unwrap();
    out
}

#[test]
fn the_xlsx_is_a_workbook_with_the_parts_a_spreadsheet_looks_for() {
    let bytes = xlsx(&debts()).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|at| zip.by_index(at).unwrap().name().unwrap().to_string())
        .collect();
    for wanted in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
    ] {
        assert!(names.contains(&wanted.to_string()), "{wanted} is missing");
    }

    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    // A figure as a number, so the spreadsheet adds it up.
    assert!(
        sheet.contains(r#"<c r="B2" s="0"><v>1234.56</v></c>"#),
        "{sheet}"
    );
    // Text as an inline string, and XML's characters escaped.
    assert!(sheet.contains("Mahnung, 2026-10-02"));
    assert!(sheet.contains("Rechnung &quot;Nr. 7&quot;, 2026-09-30"));
    // The heading row bold, and frozen above the rows.
    assert!(sheet.contains(r#"<c r="A1" s="1""#), "{sheet}");
    assert!(sheet.contains(r#"state="frozen""#));
    // The view before the columns before the rows, as the schema insists.
    let at = |needle: &str| sheet.find(needle).unwrap();
    assert!(at("<sheetViews>") < at("<cols>"));
    assert!(at("<cols>") < at("<sheetData>"));
}

#[test]
fn what_xml_cannot_carry_is_dropped_rather_than_escaped() {
    // A control character comes out of somebody else's PDF, and a workbook
    // with one in it is a workbook Excel refuses to open.
    let table = Table {
        columns: vec!["a".into()],
        rows: vec![vec![Cell::Text("Stadt\u{7}werke\tGmbH".into())]],
    };
    let sheet = part(&xlsx(&table).unwrap(), "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("Stadtwerke\tGmbH"), "{sheet}");
    assert!(!sheet.contains('\u{7}'));
}

#[test]
fn a_table_of_many_columns_is_still_lettered_right() {
    // Z to AA is where a hand-rolled column reference goes wrong.
    let table = Table {
        columns: (0..30).map(|at| format!("c{at}")).collect(),
        rows: vec![(0..30).map(|at| Cell::Number(at as f64)).collect()],
    };
    let sheet = part(&xlsx(&table).unwrap(), "xl/worksheets/sheet1.xml");
    for wanted in ["Z1", "AA1", "AD1", "AD2"] {
        assert!(
            sheet.contains(&format!(r#"r="{wanted}""#)),
            "{wanted} missing"
        );
    }
}

#[test]
fn the_format_names_the_file() {
    assert_eq!(Format::Xlsx.extension(), "xlsx");
    assert_eq!(Format::Csv.extension(), "csv");
}
