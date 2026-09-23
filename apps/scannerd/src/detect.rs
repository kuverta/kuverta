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
//!
//! **Is it another page?** After a photograph the detector waits for the
//! surface to be cleared — but a letter's pages are often laid one on top of
//! the other, or turned over where they lie, and the surface is never clear.
//! So once a hand has been over it and it is still again, what lies there is
//! compared with what was photographed: in blocks of 8×8 pixels, allowing a
//! block's shift either way, so a page nudged or turned a few degrees is the
//! same page. On the rig a nudge changed 12–18% of the blocks, the blank back
//! of a page 32%, another letter 37%.

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
    /// A frame changed this much from the one before has a hand in it.
    pub handled: f32,
    /// This fraction of blocks changed from the photographed page, and what
    /// lies there now is another page.
    pub new_page: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            pixel: 24,
            present: 0.18,
            still: 0.01,
            clear: 0.05,
            handled: 0.05,
            new_page: 0.22,
        }
    }
}

/// The fraction of pixels that differ by more than `pixel`, once the two
/// frames' overall brightness is taken out.
///
/// Taken out because the camera sets its exposure for every frame. A white
/// page in view darkens everything around it, and the table left behind comes
/// back a shade lighter or darker than the one learnt; compared raw, a shade
/// of more than `pixel` is every pixel "changed", and the page never seems to
/// have gone — the likeliest reason each page on the Pi needed the table
/// learning again before the next. A page, a hand or a shadow changes some
/// pixels far more than others, which one offset for the whole frame does not
/// hide.
///
/// The side of a block [`page_changed`] compares.
const BLOCK: usize = 8;
/// A block's mean changed by more than this, once the frames' overall
/// brightness is taken out, has changed.
const BLOCK_CHANGE: f32 = 10.0;

/// How far either way, in pixels, a page may have moved and still compare as
/// itself; tried in steps of [`SHIFT_STEP`].
const SHIFT: usize = 8;
const SHIFT_STEP: usize = 2;

/// The fraction of 8×8 blocks that differ between two frames `width` pixels
/// wide, at the best of the shifts of up to [`SHIFT`] pixels either way — so a
/// page moved a little, or turned a few degrees, compares as itself, and
/// another page, or the other side of this one, does not. On the rig's frames:
/// a page nudged 5 pixels 1%, nudged 10 and turned 4° 17%, the blank back of
/// the page 31%, another letter 34%.
pub fn page_changed(a: Luma<'_>, b: Luma<'_>, width: usize) -> f32 {
    if a.len() != b.len() || width <= 2 * SHIFT + BLOCK * 4 || !a.len().is_multiple_of(width) {
        return 0.0;
    }
    let height = a.len() / width;
    if height <= 2 * SHIFT + BLOCK * 4 {
        return 0.0;
    }
    let (bw, bh) = ((width - 2 * SHIFT) / BLOCK, (height - 2 * SHIFT) / BLOCK);
    // Block means of `frame` over the middle, its window moved by (dx, dy).
    let means = |frame: Luma<'_>, dx: isize, dy: isize| {
        let mut out = Vec::with_capacity(bw * bh);
        for by in 0..bh {
            for bx in 0..bw {
                let (x0, y0) = (
                    (SHIFT + bx * BLOCK).wrapping_add_signed(dx),
                    (SHIFT + by * BLOCK).wrapping_add_signed(dy),
                );
                let mut sum = 0u32;
                for y in y0..y0 + BLOCK {
                    sum += frame[y * width + x0..y * width + x0 + BLOCK]
                        .iter()
                        .map(|&v| v as u32)
                        .sum::<u32>();
                }
                out.push(sum as f32 / (BLOCK * BLOCK) as f32);
            }
        }
        out
    };
    let now = means(b, 0, 0);
    let changed_at = |dx: isize, dy: isize| {
        let then = means(a, dx, dy);
        // The frames' overall brightness taken out, as the camera sets its
        // exposure for every frame.
        let offset = now.iter().zip(&then).map(|(n, t)| n - t).sum::<f32>() / now.len() as f32;
        let changed = now
            .iter()
            .zip(&then)
            .filter(|(n, t)| (**n - **t - offset).abs() > BLOCK_CHANGE)
            .count();
        changed as f32 / now.len() as f32
    };
    // Every other pixel first, then the pixels round the best of those.
    let reach = SHIFT as isize;
    let mut best = (1.0f32, 0isize, 0isize);
    for dy in (-reach..=reach).step_by(SHIFT_STEP) {
        for dx in (-reach..=reach).step_by(SHIFT_STEP) {
            let changed = changed_at(dx, dy);
            if changed < best.0 {
                best = (changed, dx, dy);
            }
        }
    }
    let (mut least, cx, cy) = best;
    for dy in cy - 1..=cy + 1 {
        for dx in cx - 1..=cx + 1 {
            if dx.abs() <= reach && dy.abs() <= reach {
                least = least.min(changed_at(dx, dy));
            }
        }
    }
    least
}

/// Returns 0.0 for frames of different sizes rather than panicking: a camera
/// that changes resolution mid-run should stall the state machine, not take
/// the daemon down with it.
pub fn changed_fraction(a: Luma<'_>, b: Luma<'_>, pixel: u8) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mean = |frame: Luma<'_>| {
        (frame.iter().map(|&value| value as u64).sum::<u64>() / frame.len() as u64) as i16
    };
    let offset = mean(b) - mean(a);
    let threshold = pixel as i16;
    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| (**y as i16 - **x as i16 - offset).abs() > threshold)
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
    /// Counts baselines learnt, so whoever keeps one on disk can tell when to
    /// write it again.
    generation: u64,
    previous: Option<Vec<u8>>,
    state: State,
    /// Consecutive frames, after a photograph, with no page in view.
    clear_frames: u8,
    /// The frame a photograph was taken at: what the next page is told
    /// apart from.
    photographed: Option<Vec<u8>>,
    /// Whether a hand has been over the page since it was photographed.
    handled: bool,
    /// Still frames, since the hand, with a page there.
    still_since_handled: u8,
    /// How wide the frames are, for comparing them in blocks.
    width: usize,
    /// The last frame's change from the empty surface and from the frame
    /// before, for a person tuning the rig to look at.
    last_measure: Option<(f32, f32)>,
}

impl Detector {
    pub fn new(thresholds: Thresholds, settle_frames: u8) -> Self {
        Self {
            thresholds,
            settle_frames: settle_frames.max(1),
            baseline: None,
            generation: 0,
            previous: None,
            state: State::Waiting,
            clear_frames: 0,
            photographed: None,
            handled: false,
            still_since_handled: 0,
            width: 320,
            last_measure: None,
        }
    }

    /// How wide the frames are: 320, unless the camera says otherwise.
    pub fn frame_width(&mut self, width: usize) {
        self.width = width;
    }

    /// A photograph is being taken of `frame`: from now on the next page is
    /// looked for.
    fn photographed(&mut self, frame: Luma<'_>) -> Step {
        self.state = State::Spent;
        self.clear_frames = 0;
        self.photographed = Some(frame.to_vec());
        self.handled = false;
        self.still_since_handled = 0;
        Step::Capture
    }

    /// The last frame's change from the empty surface, then from the frame
    /// before it, as fractions of the frame.
    pub fn last_measure(&self) -> Option<(f32, f32)> {
        self.last_measure
    }

    /// Takes `frame` as the surface and waits for the next page.
    fn rearm(&mut self, frame: Luma<'_>) {
        self.learn_baseline(frame);
        self.state = State::Waiting;
        self.clear_frames = 0;
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn settle_frames(&self) -> u8 {
        self.settle_frames
    }

    /// Takes the current frame as the empty surface.
    ///
    /// Re-learnt whenever the surface is seen to be clear, so the daylight
    /// moving across a desk over an afternoon does not slowly read as a page.
    pub fn learn_baseline(&mut self, frame: Luma<'_>) {
        self.baseline = Some(frame.to_vec());
        self.generation += 1;
    }

    /// Takes this frame as the empty surface *and* arms again, whatever state
    /// the detector was in. What a person means by "the table is empty now":
    /// it gets a detector out of `Spent` when the surface it was waiting to see
    /// again was never really the empty one.
    pub fn learn_empty(&mut self, frame: Luma<'_>) {
        self.rearm(frame);
        self.previous = Some(frame.to_vec());
    }

    /// Forgets the empty surface, as when the crop changes and it no longer
    /// shows the same part of the table.
    pub fn forget_baseline(&mut self) {
        self.baseline = None;
        self.previous = None;
        self.state = State::Waiting;
        self.clear_frames = 0;
    }

    pub fn has_baseline(&self) -> bool {
        self.baseline.is_some()
    }

    pub fn baseline(&self) -> Option<&[u8]> {
        self.baseline.as_deref()
    }

    pub fn baseline_generation(&self) -> u64 {
        self.generation
    }

    /// Feeds one preview frame in and says what to do.
    pub fn observe(&mut self, frame: Luma<'_>) -> Step {
        // A baseline of another size — one kept on disk from a different
        // camera setting — compares as "nothing changed" forever, because
        // `changed_fraction` answers 0 for frames of different sizes. That
        // would be a detector that silently never sees a page.
        if self
            .baseline
            .as_ref()
            .is_some_and(|baseline| baseline.len() != frame.len())
        {
            self.baseline = None;
        }

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
        self.last_measure = Some((against_baseline, against_previous));

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
                        self.photographed(frame)
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
                    self.rearm(frame);
                } else if against_baseline < self.thresholds.present {
                    // Not quite the surface that was learnt — a cup moved, a
                    // shadow, a hand at the edge — but no page either. Seen
                    // that way for as long as a page takes to settle, it is
                    // the surface now: waiting for the old picture of it is how
                    // a detector stops photographing anything at all.
                    self.clear_frames = self.clear_frames.saturating_add(1);
                    if self.clear_frames >= self.settle_frames {
                        self.rearm(frame);
                    }
                } else {
                    self.clear_frames = 0;
                    // A page is there. Another one, or this one turned over,
                    // is photographed once a hand has been and gone and it
                    // lies still.
                    if against_previous >= self.thresholds.handled {
                        self.handled = true;
                        self.still_since_handled = 0;
                    } else if self.handled && against_previous < self.thresholds.still {
                        self.still_since_handled = self.still_since_handled.saturating_add(1);
                        if self.still_since_handled >= self.settle_frames {
                            let changed = self
                                .photographed
                                .as_deref()
                                .map(|photographed| page_changed(photographed, frame, self.width))
                                .unwrap_or(0.0);
                            if changed >= self.thresholds.new_page {
                                return self.photographed(frame);
                            }
                            // Touched, straightened, but the same page.
                            self.handled = false;
                            self.still_since_handled = 0;
                        }
                    }
                }
                Step::Wait
            }
        }
    }
}
