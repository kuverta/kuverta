//! Is there a page under the camera, and has it stopped moving?
//!
//! Arithmetic over a luma plane, and nothing else. `rpicam-still --encoding
//! yuv420` hands back raw YUV, whose first plane *is* the greyscale image, so
//! there is no JPEG to decode and no image library in this binary at all. That
//! is not a trick to save a dependency — it is what makes this file testable
//! on a laptop with no camera, which is the difference between a detector that
//! is reasoned about and one that is guessed at.
//!
//! Two questions, both answered by comparing frames:
//!
//! **Is something there?** Against a baseline of the empty surface. A sheet of
//! paper is large and pale, so it changes a great many pixels; a passing
//! shadow changes them slightly, and a threshold separates the two.
//!
//! **Has it stopped?** Against the previous frame. A hand placing the page is
//! still in shot for a moment after the page is, and capturing then gets a
//! photograph of a thumb.

/// A greyscale frame: one byte per pixel, row-major.
pub type Luma<'a> = &'a [u8];

#[derive(Debug, Clone)]
pub struct Thresholds {
    /// How different one pixel must be to count as changed, 0–255.
    ///
    /// Low enough to see paper against a wooden desk, high enough to ignore
    /// the sensor noise that makes every pixel wobble by a few counts.
    pub pixel: u8,
    /// What fraction of the frame must have changed for a page to be there.
    pub present: f32,
    /// Below this fraction of change between consecutive frames, nothing is
    /// moving any more.
    pub still: f32,
    /// Below this fraction against the baseline, the surface is clear again.
    pub clear: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            pixel: 24,
            present: 0.18,
            still: 0.01,
            clear: 0.05,
        }
    }
}

/// The fraction of pixels that differ by more than `pixel`.
///
/// Returns 0.0 for frames of different sizes rather than panicking: a camera
/// that changes resolution mid-run should stall the state machine, not take
/// the daemon down with it.
pub fn changed_fraction(a: Luma<'_>, b: Luma<'_>, pixel: u8) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let threshold = pixel as i16;
    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| (**x as i16 - **y as i16).abs() > threshold)
        .count();
    changed as f32 / a.len() as f32
}

/// Where the capture loop has got to.
///
/// Written as a state machine rather than as sleeps and flags because the one
/// behaviour that matters — never capture twice without the surface being
/// cleared in between — is a property of the transitions, and a property is
/// much easier to test than a sequence of delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing on the surface; waiting for a page.
    Waiting,
    /// Something is there but it is still moving.
    Settling { frames_still: u8 },
    /// A page has been captured; waiting for the surface to be cleared before
    /// arming again, so one letter does not become forty.
    Spent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Keep watching.
    Wait,
    /// Take the photograph now.
    Capture,
}

pub struct Detector {
    thresholds: Thresholds,
    /// How many consecutive still frames before capturing.
    settle_frames: u8,
    baseline: Option<Vec<u8>>,
    previous: Option<Vec<u8>>,
    state: State,
}

impl Detector {
    pub fn new(thresholds: Thresholds, settle_frames: u8) -> Self {
        Self {
            thresholds,
            settle_frames: settle_frames.max(1),
            baseline: None,
            previous: None,
            state: State::Waiting,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Takes the current frame as the empty surface.
    ///
    /// Re-learnt whenever the surface is seen to be clear, so the daylight
    /// moving across a desk over an afternoon does not slowly read as a page.
    pub fn learn_baseline(&mut self, frame: Luma<'_>) {
        self.baseline = Some(frame.to_vec());
    }

    pub fn has_baseline(&self) -> bool {
        self.baseline.is_some()
    }

    /// Feeds one preview frame in and says what to do.
    pub fn observe(&mut self, frame: Luma<'_>) -> Step {
        let Some(baseline) = self.baseline.as_deref() else {
            self.learn_baseline(frame);
            self.previous = Some(frame.to_vec());
            return Step::Wait;
        };

        let against_baseline = changed_fraction(baseline, frame, self.thresholds.pixel);
        let against_previous = self
            .previous
            .as_deref()
            .map(|previous| changed_fraction(previous, frame, self.thresholds.pixel))
            .unwrap_or(1.0);
        self.previous = Some(frame.to_vec());

        match self.state {
            State::Waiting => {
                if against_baseline >= self.thresholds.present {
                    self.state = State::Settling { frames_still: 0 };
                }
                Step::Wait
            }

            State::Settling { frames_still } => {
                if against_baseline < self.thresholds.present {
                    // Whatever it was has gone again — a hand passing over.
                    self.state = State::Waiting;
                    Step::Wait
                } else if against_previous < self.thresholds.still {
                    let frames_still = frames_still.saturating_add(1);
                    if frames_still >= self.settle_frames {
                        self.state = State::Spent;
                        Step::Capture
                    } else {
                        self.state = State::Settling { frames_still };
                        Step::Wait
                    }
                } else {
                    // Still moving. Start counting again rather than
                    // decrementing: a hand that pauses halfway must not add up
                    // to a still page.
                    self.state = State::Settling { frames_still: 0 };
                    Step::Wait
                }
            }

            State::Spent => {
                if against_baseline < self.thresholds.clear {
                    // The surface is empty again, and this is what the next
                    // page will be judged against.
                    self.learn_baseline(frame);
                    self.state = State::Waiting;
                }
                Step::Wait
            }
        }
    }
}
