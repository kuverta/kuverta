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
        }
    }
}

impl Camera for RpiCamera {
    fn preview(&self) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.program);
        command
            .arg("--nopreview")
            .args(["--immediate", "--encoding", "yuv420"])
            .args(["--width", &self.preview_width.to_string()])
            .args(["--height", &self.preview_height.to_string()])
            .args(["--timeout", &self.settle_ms.to_string()])
            .args(["--output", "-"]);

        let output = command
            .output()
            .with_context(|| format!("could not run {}", self.program))?;
        if !output.status.success() {
            bail!(
                "{} failed: {}",
                self.program,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        // The Y plane comes first and is one byte per pixel; U and V follow at
        // quarter resolution and are not wanted.
        let luma = (self.preview_width * self.preview_height) as usize;
        if output.stdout.len() < luma {
            bail!(
                "{} returned {} bytes, short of the {luma} a {}×{} frame needs",
                self.program,
                output.stdout.len(),
                self.preview_width,
                self.preview_height
            );
        }
        let mut frame = output.stdout;
        frame.truncate(luma);
        Ok(frame)
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

        let output = command
            .output()
            .with_context(|| format!("could not run {}", self.program))?;
        if !output.status.success() {
            bail!(
                "{} failed: {}",
                self.program,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if !path.exists() {
            bail!("{} reported success but wrote no file", self.program);
        }
        Ok(())
    }
}
