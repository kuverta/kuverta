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

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// What the daemon needs of a camera. A trait so the state machine can be
/// driven by a directory of images on a laptop.
pub trait Camera {
    /// A small greyscale frame, for detection.
    fn preview(&self) -> Result<Vec<u8>>;
    /// A full-resolution JPEG, written to `path`.
    fn capture(&self, path: &Path) -> Result<()>;
    /// The same, exposed `ev` stops brighter than the camera would choose —
    /// darker, for a negative `ev`. White paper fools a camera's exposure:
    /// the rig's OV5647 turned it pure white and the text pale grey until it
    /// was told two stops less. A camera that cannot be told takes it as it
    /// would.
    fn capture_at(&self, path: &Path, ev: f32) -> Result<()> {
        let _ = ev;
        self.capture(path)
    }
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

#[derive(Clone)]
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
    /// Previews from a video stream this program keeps running —
    /// `rpicam-vid` — at `stream_fps` frames a second. `None`: a still for
    /// each frame, as before.
    ///
    /// A still costs half a second of starting the camera every frame, which
    /// with three still frames to wait for made a page lying still wait three
    /// or four seconds to be photographed. A stream starts once, in under two
    /// seconds, and then has a frame every fifth of a second. It is stopped
    /// for the photograph, which needs the camera to itself, and started again
    /// at the next preview.
    pub video_program: Option<String>,
    pub stream_fps: u32,
    /// The running stream, shared by clones. Opaque: `..Default::default()`
    /// fills it.
    pub stream_state: StreamState,
}

/// Whatever stream an [`RpiCamera`] has running, and the last of its frames
/// handed out, so the next is a newer one.
#[derive(Clone, Default)]
pub struct StreamState {
    stream: Arc<Mutex<Option<Stream>>>,
    seen: Arc<Mutex<u64>>,
}

impl std::fmt::Debug for RpiCamera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpiCamera")
            .field("program", &self.program)
            .field("roi", &self.roi)
            .field("video_program", &self.video_program)
            .finish()
    }
}

/// The newest frame of a stream, and how many there have been.
#[derive(Default)]
struct Latest {
    count: u64,
    luma: Vec<u8>,
    ended: bool,
}

/// A running `rpicam-vid`, and what it has sent.
struct Stream {
    child: Child,
    roi: Option<String>,
    latest: Arc<(Mutex<Latest>, Condvar)>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How long a preview waits for the stream's next frame before giving up.
const FRAME_WAIT: Duration = Duration::from_secs(5);

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
            video_program: None,
            stream_fps: 5,
            stream_state: StreamState::default(),
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

    /// Stops the stream, so a still can have the camera — or because nothing
    /// is being watched. The next preview starts it again.
    pub fn stop_stream(&self) {
        *self.stream_state.stream.lock().unwrap() = None;
    }

    /// The next frame of the stream, starting it — or starting it again, for
    /// a new crop or after it stopped — when it is not running.
    fn streamed(&self, program: &str) -> Result<Vec<u8>> {
        let latest = {
            let mut stream = self.stream_state.stream.lock().unwrap();
            let stale = stream.as_ref().is_some_and(|stream| {
                stream.roi != self.roi || stream.latest.0.lock().unwrap().ended
            });
            if stale {
                *stream = None;
            }
            if stream.is_none() {
                *stream = Some(self.start_stream(program)?);
            }
            stream.as_ref().expect("started").latest.clone()
        };
        let (lock, arrived) = &*latest;
        let mut seen = self.stream_state.seen.lock().unwrap();
        let (guard, timeout) = arrived
            .wait_timeout_while(lock.lock().unwrap(), FRAME_WAIT, |latest| {
                latest.count <= *seen && !latest.ended
            })
            .unwrap();
        if guard.count <= *seen {
            let ended = guard.ended;
            drop(guard);
            self.stop_stream();
            if ended || !timeout.timed_out() {
                bail!("{program} stopped sending frames");
            }
            bail!("{program} sent no frame for {FRAME_WAIT:?}");
        }
        *seen = guard.count;
        Ok(guard.luma.clone())
    }

    fn start_stream(&self, program: &str) -> Result<Stream> {
        let (width, height) = (self.preview_width as usize, self.preview_height as usize);
        let mut command = Command::new(program);
        command
            .arg("--nopreview")
            .args(["--codec", "yuv420"])
            .args(["--width", &width.to_string()])
            .args(["--height", &height.to_string()])
            .args(["--framerate", &self.stream_fps.max(1).to_string()])
            .args(["--timeout", "0"])
            .args(["--output", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(roi) = &self.roi {
            command.args(["--roi", roi]);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("could not run {program}"))?;
        let mut stdout = child.stdout.take().context("no output from the stream")?;
        let latest: Arc<(Mutex<Latest>, Condvar)> = Arc::default();
        let feed = latest.clone();
        // A new stream counts its frames from nothing.
        *self.stream_state.seen.lock().unwrap() = 0;
        std::thread::spawn(move || {
            // YUV 4:2:0: the luma plane, then two quarter-size chroma planes.
            let mut frame = vec![0u8; width * height * 3 / 2];
            while stdout.read_exact(&mut frame).is_ok() {
                let (lock, arrived) = &*feed;
                let mut latest = lock.lock().unwrap();
                latest.count += 1;
                latest.luma.clear();
                latest.luma.extend_from_slice(&frame[..width * height]);
                arrived.notify_all();
            }
            let (lock, arrived) = &*feed;
            lock.lock().unwrap().ended = true;
            arrived.notify_all();
        });
        tracing::debug!(fps = self.stream_fps, "preview stream started");
        Ok(Stream {
            child,
            roi: self.roi.clone(),
            latest,
        })
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
        match &self.video_program {
            Some(program) => self.streamed(program),
            None => self.frame(self.roi.as_deref()),
        }
    }

    fn full_view(&self) -> Result<Vec<u8>> {
        self.stop_stream();
        self.frame(None)
    }

    fn snapshot(&self) -> Result<Vec<u8>> {
        self.stop_stream();
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
        self.capture_at(path, 0.0)
    }

    fn capture_at(&self, path: &Path, ev: f32) -> Result<()> {
        // The photograph needs the camera to itself.
        self.stop_stream();
        let mut command = Command::new(&self.program);
        command
            .arg("--nopreview")
            // Autofocus where the sensor has it; harmless where it does not.
            .args(["--autofocus-mode", "auto"])
            // The camera's colour denoising smooths exactly what the OCR
            // needs: the thin strokes of 8-point print. A photograph of paper
            // is not a photograph of a dim room, and there is little noise to
            // remove.
            .args(["--denoise", "cdn_off"])
            // Long enough for the exposure to settle after the camera starts;
            // 800 was a third of a second more than the rig's needed.
            .args(["--timeout", "500"])
            .args(["--output", &path.to_string_lossy()]);

        if let Some(roi) = &self.roi {
            command.args(["--roi", roi]);
        }
        if ev != 0.0 {
            command.args(["--ev", &format!("{ev:.1}")]);
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
