//! The camera, which is a program rather than a library.
//!
//! `rpicam-still` is what Raspberry Pi OS ships and supports; the libcamera
//! stack underneath it is C++ and its Rust bindings are not something to bet a
//! device daemon on. Shelling out costs a process per frame and buys a camera
//! that will still work after the next OS upgrade.
//!
//! Two things fall out of that choice, and both are why this file is short:
//!
//! **Previews are raw YUV.** `--encoding yuv420` means the first plane is the
//! greyscale image, so detection needs no JPEG decoder and this binary has no
//! image library in it at all.
//!
//! **Cropping is a sensor setting.** A capture rig has the camera fixed above
//! a fixed spot, so the page is always in the same part of the frame.
//! `--roi` crops on the sensor, which makes cropping a line of configuration
//! rather than a page of geometry — and geometry that cannot be tested against
//! real photographs is geometry that is guessed at.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// What the daemon needs of a camera. A trait so the state machine can be
/// driven by a directory of images on a laptop.
pub trait Camera {
    /// A small greyscale frame, for detection.
    fn preview(&self) -> Result<Vec<u8>>;
    /// A full-resolution JPEG, written to `path`.
    fn capture(&self, path: &Path) -> Result<()>;
    /// A small greyscale frame of the whole view, ignoring the crop — what
    /// setup finds the page in. The same as a preview for a camera with no
    /// crop.
    fn full_view(&self) -> Result<Vec<u8>> {
        self.preview()
    }
    /// A colour JPEG of the whole view, ignoring the crop, big enough to see
    /// a table on a phone — what setup shows to choose the crop on.
    fn snapshot(&self) -> Result<Vec<u8>> {
        bail!("this camera takes no snapshots")
    }
}

#[derive(Debug, Clone)]
pub struct RpiCamera {
    pub program: String,
    pub preview_width: u32,
    pub preview_height: u32,
    /// `x,y,w,h`, each 0.0–1.0 of the sensor. The crop.
    pub roi: Option<String>,
    /// Milliseconds the sensor is given to settle before a frame is taken.
    pub settle_ms: u32,
    /// The sensor's full size in pixels, which a crop's photograph is a share
    /// of. The Pi camera v1 (OV5647) is 2592×1944.
    pub sensor_width: u32,
    pub sensor_height: u32,
}

impl Default for RpiCamera {
    fn default() -> Self {
        Self {
            program: "rpicam-still".to_string(),
            // Small on purpose: detection is a fraction of changed pixels, and
            // 320×240 is ample for "is there a sheet of paper there" while
            // being cheap enough to run several times a second.
            preview_width: 320,
            preview_height: 240,
            roi: None,
            // The shortest the sensor will accept and still expose sanely.
            settle_ms: 200,
            sensor_width: 2592,
            sensor_height: 1944,
        }
    }
}

impl RpiCamera {
    /// The photograph's size for the crop: the crop's own share of the
    /// sensor's pixels.
    ///
    /// Without it `rpicam-still` scales whatever the crop is to its full 4:3
    /// output, so a crop the shape of a page came out stretched — which is why
    /// the first setup page could only draw 4:3 boxes, and why that was no use
    /// for a page. `None` without a crop, where the camera's own size applies.
    pub fn capture_size(&self) -> Option<(u32, u32)> {
        let parts: Vec<f64> = self
            .roi
            .as_deref()?
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [_, _, width, height] = parts[..] else {
            return None;
        };
        // Even sizes, which every encoder takes, and never absurdly small.
        let even = |pixels: f64| ((pixels.round() as u32) / 2 * 2).max(64);
        Some((
            even(width * self.sensor_width as f64),
            even(height * self.sensor_height as f64),
        ))
    }

    fn frame(&self, roi: Option<&str>) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.program);
        command
            .arg("--nopreview")
            .args(["--immediate", "--encoding", "yuv420"])
            .args(["--width", &self.preview_width.to_string()])
            .args(["--height", &self.preview_height.to_string()])
            .args(["--timeout", &self.settle_ms.to_string()])
            .args(["--output", "-"]);

        if let Some(roi) = roi {
            command.args(["--roi", roi]);
        }

        let output = run(&self.program, &mut command)?;

        // The Y plane comes first and is one byte per pixel; U and V follow at
        // quarter resolution and are not wanted.
        let luma = (self.preview_width * self.preview_height) as usize;
        if output.len() < luma {
            bail!(
                "{} returned {} bytes, short of the {luma} a {}×{} frame needs",
                self.program,
                output.len(),
                self.preview_width,
                self.preview_height
            );
        }
        let mut frame = output;
        frame.truncate(luma);
        Ok(frame)
    }
}

impl Camera for RpiCamera {
    fn preview(&self) -> Result<Vec<u8>> {
        // The same crop as the photograph. Detection's thresholds are fractions
        // of the frame, so without it they were fractions of the whole desk: a
        // page filling a tight crop could change too little of the full view
        // to count as present, and a hand beside the crop could count as one.
        self.frame(self.roi.as_deref())
    }

    fn full_view(&self) -> Result<Vec<u8>> {
        self.frame(None)
    }

    fn snapshot(&self) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.program);
        command
            .arg("--nopreview")
            .args(["--immediate", "--encoding", "jpg", "--quality", "85"])
            .args(["--width", "1024", "--height", "768"])
            .args(["--timeout", &self.settle_ms.to_string()])
            .args(["--output", "-"]);
        let jpeg = run(&self.program, &mut command)?;
        if !jpeg.starts_with(&[0xFF, 0xD8]) {
            bail!("{} did not return a JPEG", self.program);
        }
        Ok(jpeg)
    }

    fn capture(&self, path: &Path) -> Result<()> {
        let mut command = Command::new(&self.program);
        command
            .arg("--nopreview")
            // Autofocus where the sensor has it; harmless where it does not.
            .args(["--autofocus-mode", "auto"])
            .args(["--timeout", "800"])
            .args(["--output", &path.to_string_lossy()]);

        if let Some(roi) = &self.roi {
            command.args(["--roi", roi]);
        }
        if let Some((width, height)) = self.capture_size() {
            command.args([
                "--width",
                &width.to_string(),
                "--height",
                &height.to_string(),
            ]);
        }

        run(&self.program, &mut command)?;
        if !path.exists() {
            bail!("{} reported success but wrote no file", self.program);
        }
        Ok(())
    }
}

/// Runs the camera program and returns what it wrote to standard output.
fn run(program: &str, command: &mut Command) -> Result<Vec<u8>> {
    let output = command
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}
