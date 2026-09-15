//! The photographs inside a scan's PDF.
//!
//! A vision model reads images, not PDFs. scannerd's PDFs carry each page's
//! photograph unchanged as a JPEG (`/DCTDecode`), so taking them back out is
//! finding those streams — no PDF renderer, no image library. A PDF from
//! anywhere else may compress its images another way or give a stream's length
//! only by reference; those yield nothing here, and a caller falls back to the
//! text Paperless already has, which for a PDF that was never a photograph is
//! usually good.

/// Each JPEG embedded in `pdf`, in the order they appear — for scannerd's
/// letters, one per page.
pub fn jpeg_pages(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut pages = Vec::new();
    let mut at = 0;
    while let Some(found) = find(&pdf[at..], b"/DCTDecode") {
        let filter = at + found;
        // The image's dictionary, around the filter.
        let start = rfind(&pdf[..filter], b"<<").unwrap_or(0);
        let Some(end) = find(&pdf[filter..], b">>").map(|end| filter + end) else {
            break;
        };
        at = end + 2;
        let Some(length) = direct_length(&pdf[start..end]) else {
            continue;
        };

        // `stream`, one end of line, then exactly `length` bytes.
        let Some(keyword) = find(&pdf[at..], b"stream").map(|found| at + found) else {
            break;
        };
        let mut data = keyword + b"stream".len();
        if pdf.get(data) == Some(&b'\r') {
            data += 1;
        }
        if pdf.get(data) == Some(&b'\n') {
            data += 1;
        }
        let Some(bytes) = pdf.get(data..data + length) else {
            break;
        };
        if bytes.starts_with(&[0xFF, 0xD8]) {
            pages.push(bytes.to_vec());
        }
        at = data + length;
    }
    pages
}

/// `/Length` when it is a number in the dictionary itself. `/Length 12 0 R`
/// names another object, which this does not follow.
fn direct_length(dictionary: &[u8]) -> Option<usize> {
    let after = find(dictionary, b"/Length")? + b"/Length".len();
    let rest = std::str::from_utf8(&dictionary[after..]).ok()?;
    let mut words = rest.split_whitespace();
    let length = words.next()?.parse().ok()?;
    let generation = words.next();
    if generation.is_some_and(|word| word.chars().all(|c| c.is_ascii_digit()))
        && words.next() == Some("R")
    {
        return None;
    }
    Some(length)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}
