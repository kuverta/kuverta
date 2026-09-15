//! Taking the photographs back out of a scan's PDF, for a vision model.

use core_paper::scan::jpeg_pages;

/// An image object as scannerd writes one: a flat dictionary, a direct
/// `/Length`, and the JPEG unchanged between `stream` and `endstream`.
fn image_object(number: usize, jpeg: &[u8]) -> Vec<u8> {
    let mut object = format!(
        "{number} 0 obj\n<< /Type /XObject /Subtype /Image /Width 1392 /Height 1820 /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        jpeg.len()
    )
    .into_bytes();
    object.extend_from_slice(jpeg);
    object.extend_from_slice(b"\nendstream\nendobj\n");
    object
}

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec();
    for object in objects {
        pdf.extend_from_slice(object);
    }
    pdf.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    pdf
}

#[test]
fn each_page_photograph_comes_out_whole_and_in_order() {
    // Including the bytes a careless search would stop at: `>>`, `stream`
    // and `endstream` inside the JPEG data itself.
    let first = b"\xff\xd8page one >> stream endstream \x00\xff".to_vec();
    let second = b"\xff\xd8page two".to_vec();
    let document = pdf(&[image_object(3, &first), image_object(6, &second)]);

    assert_eq!(jpeg_pages(&document), vec![first, second]);
}

#[test]
fn a_length_given_by_reference_is_not_followed() {
    let mut object =
        b"3 0 obj\n<< /Subtype /Image /Filter /DCTDecode /Length 9 0 R >>\nstream\n\xff\xd8x\nendstream\nendobj\n"
            .to_vec();
    object.extend_from_slice(b"9 0 obj\n3\nendobj\n");
    assert!(jpeg_pages(&pdf(&[object])).is_empty());
}

#[test]
fn what_is_not_a_jpeg_is_left_alone() {
    let flate = b"3 0 obj\n<< /Filter /FlateDecode /Length 4 >>\nstream\nabcd\nendstream\nendobj\n".to_vec();
    let not_really = image_object(4, b"GIF89a....");
    assert!(jpeg_pages(&pdf(&[flate, not_really])).is_empty());
}

#[test]
fn a_truncated_scan_yields_what_is_whole_and_does_not_panic() {
    let whole = b"\xff\xd8whole".to_vec();
    let mut document = pdf(&[image_object(3, &whole)]);
    let mut cut = image_object(4, b"\xff\xd8this page was cut off halfway");
    cut.truncate(cut.len() - 30);
    document.extend_from_slice(&cut);

    assert_eq!(jpeg_pages(&document), vec![whole]);
    assert!(jpeg_pages(b"").is_empty());
    assert!(jpeg_pages(b"/DCTDecode").is_empty());
}
