//! Pages into one PDF, with no image library.
//!
//! A letter of several pages has to reach Paperless as one document, and
//! Paperless takes a PDF. A JPEG goes into a PDF as it is — PDF has carried
//! JPEG data natively since 1.2 (`/DCTDecode`) — so assembling one needs only
//! each image's size and colour components, which sit in its frame header,
//! and a few hundred bytes of objects around it. Nothing is re-encoded, so
//! nothing is lost before the OCR sees it.

use anyhow::{bail, Context, Result};

/// What a PDF needs to know about a JPEG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
}

/// Reads a JPEG's frame header, skipping whatever segments come before it.
pub fn jpeg_info(jpeg: &[u8]) -> Result<JpegInfo> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        bail!("not a JPEG");
    }

    let mut at = 2;
    while at + 4 <= jpeg.len() {
        if jpeg[at] != 0xFF {
            bail!("a JPEG marker was expected at byte {at}");
        }
        let marker = jpeg[at + 1];
        // Any number of 0xFF fill bytes may come before a marker.
        if marker == 0xFF {
            at += 1;
            continue;
        }
        // Markers that stand alone, with no length.
        if marker == 0x01 || marker == 0xD8 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }
        // End of image, or the scan itself: no frame header came first.
        if marker == 0xD9 || marker == 0xDA {
            break;
        }

        let length = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if length < 2 {
            bail!("a JPEG segment claims a length of {length}");
        }

        // SOF0–SOF15 are frame headers; C4, C8 and CC share the range and are not.
        let frame = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if frame {
            // precision (1), height (2), width (2), components (1), ...
            let header = jpeg
                .get(at + 4..at + 2 + length)
                .filter(|header| header.len() >= 6)
                .context("a truncated JPEG frame header")?;
            let height = u16::from_be_bytes([header[1], header[2]]) as u32;
            let width = u16::from_be_bytes([header[3], header[4]]) as u32;
            if width == 0 || height == 0 {
                bail!("a JPEG with no size ({width}×{height})");
            }
            return Ok(JpegInfo {
                width,
                height,
                components: header[5],
            });
        }

        at += 2 + length;
    }

    bail!("a JPEG with no frame header")
}

/// Every page is A4 wide, in points. The height follows the photograph's
/// proportions, so nothing is stretched; what the OCR reads is the pixels,
/// which are kept exactly as the camera took them.
const PAGE_WIDTH: f64 = 595.0;

/// One PDF, a page per JPEG, in order.
pub fn from_jpegs(pages: &[Vec<u8>]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        bail!("a PDF needs at least one page");
    }
    let infos = pages
        .iter()
        .enumerate()
        .map(|(index, jpeg)| {
            let info = jpeg_info(jpeg).with_context(|| format!("page {}", index + 1))?;
            if !matches!(info.components, 1 | 3) {
                // Four is CMYK, whose inversion conventions differ between
                // encoders. No camera produces it; refusing is better than
                // a letter in negative.
                bail!(
                    "page {} has {} colour components",
                    index + 1,
                    info.components
                );
            }
            Ok(info)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut pdf = Writer::default();
    // The binary comment tells tools the file is not text.
    pdf.out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // Objects: 1 is the catalogue, 2 the page tree, then three per page.
    let page_object = |index: usize| 3 + index * 3;
    let kids = (0..pages.len())
        .map(|index| format!("{} 0 R", page_object(index)))
        .collect::<Vec<_>>()
        .join(" ");

    pdf.object(1, &[b"<< /Type /Catalog /Pages 2 0 R >>"]);
    pdf.object(
        2,
        &[format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).as_bytes()],
    );

    for (index, (jpeg, info)) in pages.iter().zip(&infos).enumerate() {
        let page = page_object(index);
        let (contents, image) = (page + 1, page + 2);
        let height = PAGE_WIDTH * info.height as f64 / info.width as f64;

        pdf.object(
            page,
            &[format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH:.2} {height:.2}] \
                 /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"
            )
            .as_bytes()],
        );

        let draw = format!("q {PAGE_WIDTH:.2} 0 0 {height:.2} 0 0 cm /Im0 Do Q");
        pdf.object(
            contents,
            &[
                format!("<< /Length {} >>\nstream\n", draw.len()).as_bytes(),
                draw.as_bytes(),
                b"\nendstream",
            ],
        );

        let space = if info.components == 1 {
            "/DeviceGray"
        } else {
            "/DeviceRGB"
        };
        pdf.object(
            image,
            &[
                format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {space} \
                     /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                    info.width,
                    info.height,
                    jpeg.len()
                )
                .as_bytes(),
                jpeg,
                b"\nendstream",
            ],
        );
    }

    Ok(pdf.finish())
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    /// Where each object starts, by object number minus one.
    offsets: Vec<usize>,
}

impl Writer {
    fn object(&mut self, number: usize, parts: &[&[u8]]) {
        debug_assert_eq!(
            number,
            self.offsets.len() + 1,
            "objects are written in order"
        );
        self.offsets.push(self.out.len());
        self.out
            .extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        for part in parts {
            self.out.extend_from_slice(part);
        }
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// The cross-reference table and trailer. Every entry is exactly twenty
    /// bytes, which is what lets a reader seek straight to an object.
    fn finish(mut self) -> Vec<u8> {
        let xref_at = self.out.len();
        let size = self.offsets.len() + 1;
        self.out
            .extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for offset in &self.offsets {
            self.out
                .extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        self.out.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
                .as_bytes(),
        );
        self.out
    }
}
