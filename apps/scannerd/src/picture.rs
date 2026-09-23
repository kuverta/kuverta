//! Small greyscale pictures for the LCD: what the camera sees, and the page
//! it last photographed.
//!
//! Greyscale because that is what scannerd has of both without decoding
//! colour — a preview frame is the luma plane, and the photograph check reads
//! only the luma of a JPEG — and small because the panes they go in are a few
//! hundred pixels across. The pixels are shared, so a screen that holds one
//! can be copied and compared cheaply.

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub luma: Arc<Vec<u8>>,
}

impl Picture {
    /// `luma`, `width`×`height`, made to fit inside `max_width`×`max_height`
    /// with its proportions kept. Each pixel is the average of the ones it
    /// stands for, so thin lines of text become grey rather than vanishing.
    /// Never made larger.
    pub fn scaled(
        luma: &[u8],
        width: usize,
        height: usize,
        max_width: u32,
        max_height: u32,
    ) -> Self {
        if width == 0 || height == 0 || luma.len() < width * height {
            return Self::empty();
        }
        let scale = (max_width as f64 / width as f64)
            .min(max_height as f64 / height as f64)
            .min(1.0);
        let (w, h) = (
            ((width as f64 * scale).round() as usize).max(1),
            ((height as f64 * scale).round() as usize).max(1),
        );
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let (top, bottom) = (
                y * height / h,
                ((y + 1) * height / h).max(y * height / h + 1),
            );
            for x in 0..w {
                let (left, right) = (x * width / w, ((x + 1) * width / w).max(x * width / w + 1));
                let mut sum = 0u64;
                for row in top..bottom {
                    sum += luma[row * width + left..row * width + right]
                        .iter()
                        .map(|&value| value as u64)
                        .sum::<u64>();
                }
                out.push((sum / ((bottom - top) * (right - left)) as u64) as u8);
            }
        }
        Self {
            width: w as u32,
            height: h as u32,
            luma: Arc::new(out),
        }
    }

    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            luma: Arc::new(Vec::new()),
        }
    }

    /// Turned `degrees` clockwise, in quarter turns: the live view is turned
    /// as the photographs will be.
    pub fn turned(&self, degrees: u16) -> Self {
        let quarters = (degrees / 90) % 4;
        if quarters == 0 || self.luma.is_empty() {
            return self.clone();
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let (out_w, out_h) = if quarters % 2 == 1 { (h, w) } else { (w, h) };
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (ox, oy) = match quarters {
                    1 => (h - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                out[oy * out_w + ox] = self.luma[y * w + x];
            }
        }
        Self {
            width: out_w as u32,
            height: out_h as u32,
            luma: Arc::new(out),
        }
    }
}
