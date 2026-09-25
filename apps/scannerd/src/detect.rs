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
    /// What share of the frame must have changed *the way paper changes it* —
    /// paler than a dark table, darker than a pale one — for the change to be
    /// a page rather than the light shifting.
    pub paper_share: f32,
    /// And how densely those blocks must sit inside the box they span: paper
    /// is one lump, and an evening shadow is spread over everything.
    pub paper_fill: f32,
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
            // A DL envelope on the rig's corners covers about 0.4 of the
            // frame and an A4 page nearly all of it, so this is generous —
            // it is there to rule out the light, not to measure the paper.
            paper_share: 0.06,
            paper_fill: 0.45,
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

/// The side of a block [`paper_like`] compares. Larger than [`BLOCK`]: this
/// asks where the change *is*, not how much of it there is, and sensor noise
/// averages away over 256 pixels.
const PAPER_BLOCK: usize = 16;
/// A block's mean must move this far from the table's to count as something
/// lying there.
const PAPER_BY: f32 = 12.0;
/// And it must be this much paler than the table itself — not than the rest of
/// the frame. A shadow over half the table makes the other half *look* paler
/// than the average, which is how the sun going down came to be photographed
/// as a letter; against the table's own level a shadow is never paper.
const PAPER_ABOVE_TABLE: f32 = 35.0;
/// However dark the table, paper reads at least this. The rig's table is 81 in
/// the preview stream and its brightest pixel 121, so this is a floor under
/// [`PAPER_ABOVE_TABLE`] and not a second opinion.
const PAPER_LEVEL: f32 = 110.0;
/// A baseline this pale is a pale table, and paper on it reads *darker*: the
/// tests below and the rig both have a dark one, but a white desk is a desk.
const PALE_TABLE: f32 = 150.0;

/// Whether what changed looks like paper on the table or like the light
/// changing over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaperLike {
    /// The share of the frame's blocks that moved the way paper moves them.
    pub share: f32,
    /// How much of the box those blocks span they actually fill: 1 for one
    /// solid lump, and much less for change scattered over the whole frame.
    pub fill: f32,
}

impl PaperLike {
    pub const NOTHING: Self = Self {
        share: 0.0,
        fill: 0.0,
    };
}

/// Where the change between two frames is, and which way it went.
///
/// The sun going down was photographed as a letter: the light over the table
/// changes, [`changed_fraction`] climbs past `present` — one offset for the
/// whole frame cannot hide a shadow crossing it — and a photograph of the bare
/// table is taken. What tells the two apart is not how much changed but the
/// shape of it: paper is one lump, in one direction (paler than a dark table,
/// darker than a pale one), and an evening is a wash over everything, mostly
/// the other way.
///
/// So the frame is divided into blocks, each block's mean compared with the
/// baseline's after the overall brightness is taken out, and the blocks that
/// moved the way paper moves them are counted and their box measured.
pub fn paper_like(baseline: Luma<'_>, frame: Luma<'_>, width: usize) -> PaperLike {
    if baseline.len() != frame.len()
        || width < PAPER_BLOCK * 3
        || !frame.len().is_multiple_of(width)
    {
        return PaperLike::NOTHING;
    }
    let height = frame.len() / width;
    let (bw, bh) = (width / PAPER_BLOCK, height / PAPER_BLOCK);
    if bw < 3 || bh < 3 {
        return PaperLike::NOTHING;
    }
    let block_mean = |plane: Luma<'_>, bx: usize, by: usize| {
        let (x0, y0) = (bx * PAPER_BLOCK, by * PAPER_BLOCK);
        let mut sum = 0u32;
        for y in y0..y0 + PAPER_BLOCK {
            sum += plane[y * width + x0..y * width + x0 + PAPER_BLOCK]
                .iter()
                .map(|&v| v as u32)
                .sum::<u32>();
        }
        sum as f32 / (PAPER_BLOCK * PAPER_BLOCK) as f32
    };

    let mut table = 0.0f32;
    let mut residuals = Vec::with_capacity(bw * bh);
    for by in 0..bh {
        for bx in 0..bw {
            let was = block_mean(baseline, bx, by);
            let now = block_mean(frame, bx, by);
            table += was;
            residuals.push(now - was);
        }
    }
    let blocks = residuals.len() as f32;
    table /= blocks;
    // A page is paler than a dark table and darker than a pale one.
    let pale_table = table >= PALE_TABLE;
    let sign = if pale_table { -1.0 } else { 1.0 };
    // What a block must read in itself to be paper: pale on a dark table, and
    // clearly darker than the table when the table is the pale thing.
    let level = if pale_table {
        table - PAPER_ABOVE_TABLE
    } else {
        (table + PAPER_ABOVE_TABLE).max(PAPER_LEVEL)
    };

    let mut lit = 0usize;
    let (mut left, mut top, mut right, mut bottom) = (bw, bh, 0usize, 0usize);
    for (index, residual) in residuals.iter().enumerate() {
        if residual * sign <= PAPER_BY {
            continue;
        }
        let (bx, by) = (index % bw, index / bw);
        let now = block_mean(frame, bx, by);
        if (now - level) * sign <= 0.0 {
            continue;
        }
        lit += 1;
        left = left.min(bx);
        top = top.min(by);
        right = right.max(bx);
        bottom = bottom.max(by);
    }
    if lit == 0 {
        return PaperLike::NOTHING;
    }
    let box_blocks = ((right + 1 - left) * (bottom + 1 - top)) as f32;
    PaperLike {
        share: lit as f32 / blocks,
        fill: lit as f32 / box_blocks,
    }
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
    /// And whether that change looked like paper.
    last_paper: Option<PaperLike>,
    /// Consecutive frames whose change from the baseline did not look like
    /// paper: the light over the table has moved, and after a while what it
    /// looks like now is the table.
    light_frames: u8,
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
            last_paper: None,
            light_frames: 0,
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
    /// Whether the last frame's change from the table looked like paper.
    pub fn last_paper(&self) -> Option<PaperLike> {
        self.last_paper
    }

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
        // Only worth asking where the change is when there is enough of it to
        // be a page: this walks the frame twice more.
        let paper = if against_baseline >= self.thresholds.present {
            paper_like(baseline, frame, self.width)
        } else {
            PaperLike::NOTHING
        };
        let looks_like_paper =
            paper.share >= self.thresholds.paper_share && paper.fill >= self.thresholds.paper_fill;
        self.last_paper = Some(paper);
        let against_previous = self
            .previous
            .as_deref()
            .map(|previous| changed_fraction(previous, frame, self.thresholds.pixel))
            .unwrap_or(1.0);
        self.previous = Some(frame.to_vec());
        self.last_measure = Some((against_baseline, against_previous));

        match self.state {
            State::Waiting => {
                if against_baseline < self.thresholds.present {
                    self.light_frames = 0;
                } else if looks_like_paper {
                    self.light_frames = 0;
                    self.state = State::Settling { frames_still: 0 };
                } else {
                    // Changed, but not the way paper changes it: the sun has
                    // moved over the table. Seen that way for a while, that is
                    // what the table looks like now — otherwise every frame
                    // from here to nightfall is a page about to be
                    // photographed.
                    self.light_frames = self.light_frames.saturating_add(1);
                    if self.light_frames >= self.settle_frames.saturating_mul(4) {
                        tracing::info!(
                            share = paper.share,
                            fill = paper.fill,
                            change = against_baseline,
                            "the light over the table changed: learning the table again"
                        );
                        self.learn_baseline(frame);
                        self.light_frames = 0;
                    }
                }
                Step::Wait
            }

            State::Settling { frames_still } => {
                if against_baseline < self.thresholds.present || !looks_like_paper {
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
