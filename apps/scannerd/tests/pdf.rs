//! Pages into one PDF, checked by reading the file back the way a PDF reader
//! does: from the trailer, through the cross-reference table, to each object.

use scannerd::pdf::{from_jpegs, jpeg_info, JpegInfo};

/// The smallest JPEG `jpeg_info` has to read: start of image, an APP0 segment
/// to skip, a baseline frame header, end of image. `sof` picks the frame type.
fn jpeg_with(sof: u8, width: u16, height: u16, components: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8];
    bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x07, b'J', b'F', b'I', b'F', 0x00]);
    let length = 8 + 3 * components as u16;
    bytes.extend_from_slice(&[0xFF, sof]);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.push(8);
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.push(components);
    for id in 0..components {
        bytes.extend_from_slice(&[id + 1, 0x11, 0x00]);
    }
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    jpeg_with(0xC0, width, height, 3)
}

#[test]
fn a_jpegs_size_is_read_from_its_frame_header() {
    assert_eq!(
        jpeg_info(&jpeg(2480, 3508)).unwrap(),
        JpegInfo {
            width: 2480,
            height: 3508,
            components: 3
        }
    );
    // Progressive, as some encoders write by default.
    assert_eq!(
        jpeg_info(&jpeg_with(0xC2, 640, 480, 1)).unwrap().components,
        1
    );
}

#[test]
fn a_huffman_table_is_not_mistaken_for_a_frame_header() {
    // C4 sits in the frame-header range and is not one.
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x05, 0x00, 0x00, 0x00];
    bytes.extend_from_slice(&jpeg(100, 200)[2..]);
    assert_eq!(jpeg_info(&bytes).unwrap().width, 100);
}

#[test]
fn what_is_not_a_readable_jpeg_says_so() {
    assert!(jpeg_info(b"%PDF-1.4").is_err());
    assert!(
        jpeg_info(&[0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02]).is_err(),
        "scan before any frame"
    );
    assert!(jpeg_info(&jpeg(0, 10)).is_err(), "no size");
    let whole = jpeg(10, 10);
    assert!(jpeg_info(&whole[..16]).is_err(), "truncated");
}

#[test]
fn three_pages_make_one_pdf_of_three_pages() {
    let pdf = from_jpegs(&[jpeg(2480, 3508), jpeg(2480, 3508), jpeg(3508, 2480)]).unwrap();
    let text = String::from_utf8_lossy(&pdf);

    assert!(pdf.starts_with(b"%PDF-1.4\n"));
    assert!(text.ends_with("%%EOF\n"));
    assert_eq!(text.matches("/Type /Page ").count(), 3);
    assert!(text.contains("/Count 3"));
}

#[test]
fn every_cross_reference_points_at_its_object() {
    // A reader seeks to objects by these offsets; one that is off by a byte is
    // a PDF that opens in forgiving viewers and fails in ocrmypdf.
    let pdf = from_jpegs(&[jpeg(800, 1000), jpeg(800, 1000)]).unwrap();

    // Bytes, not text: the comment on line two is deliberately not UTF-8, and
    // a lossy conversion moves every offset after it. (The first version of
    // this test did exactly that, and blamed the writer.)
    let marker = b"startxref\n";
    let tail = &pdf[pdf
        .windows(marker.len())
        .rposition(|window| window == marker)
        .unwrap()
        + marker.len()..];
    let xref_at: usize =
        std::str::from_utf8(&tail[..tail.iter().position(|b| *b == b'\n').unwrap()])
            .unwrap()
            .parse()
            .unwrap();
    assert!(pdf[xref_at..].starts_with(b"xref\n0 9\n"));

    // From the table onward the file is ASCII.
    let table = std::str::from_utf8(&pdf[xref_at..]).unwrap();
    let entries: Vec<&str> = table.lines().skip(3).take(8).collect();
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(
            entry.len(),
            19,
            "entries are twenty bytes with the newline: {entry:?}"
        );
        let offset: usize = entry[..10].parse().unwrap();
        let number = index + 1;
        assert!(
            pdf[offset..].starts_with(format!("{number} 0 obj\n").as_bytes()),
            "object {number} is not at {offset}"
        );
    }
}

#[test]
fn the_photograph_goes_in_as_it_is_and_the_page_keeps_its_shape() {
    let page = jpeg(2480, 3508);
    let pdf = from_jpegs(std::slice::from_ref(&page)).unwrap();
    let text = String::from_utf8_lossy(&pdf);

    assert!(
        pdf.windows(page.len())
            .any(|window| window == page.as_slice()),
        "the JPEG bytes must be embedded unchanged"
    );
    assert!(text.contains(&format!("/Filter /DCTDecode /Length {}", page.len())));
    assert!(text.contains(&format!(
        "/MediaBox [0 0 595.00 {:.2}]",
        595.0 * 3508.0 / 2480.0
    )));
}

#[test]
fn nothing_to_put_in_it_or_a_page_it_cannot_show_is_refused() {
    assert!(from_jpegs(&[]).is_err());
    let err = from_jpegs(&[jpeg(10, 10), jpeg_with(0xC0, 10, 10, 4)]).unwrap_err();
    assert!(format!("{err:#}").contains("page 2"), "{err:#}");
}
