//! One turn of the capture loop.
//!
//! Split out of the binary so the loop itself can be driven by a test: a
//! scripted camera, a spool in a temporary directory and a Paperless on
//! loopback, with the clock passed in rather than read. `main` is then only
//! arguments, the setup page's commands, and a sleep.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::button::Button;
use crate::camera::Camera;
use crate::detect::{Detector, State, Step, Thresholds};
use crate::folders::{folders_among, Filing, Filings, Folder, Outcome, Refiling};
use crate::hub::Event;
use crate::picture::Picture;
use crate::quality::{assess, better, exposure, read_luma, Exposure, Problem, Quality};
use crate::read::{PageText, Reader};
use crate::spool::Spool;
use crate::straighten::{straighten_trimmed, Corners};
use crate::upload::{Task, Uploader};

/// What one turn did.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    Watching,
    /// Not watching: scanning is stopped on the display. Letters still close
    /// and the spool is still sent.
    Paused,
    Captured(PathBuf),
    /// The camera gave no frame. The spool was still looked at.
    NoFrame,
}

/// Collecting pages into letters, closed by a button or by being left alone.
pub struct Letters {
    pub button: Button,
    /// A letter nobody closed is closed this long after its last page, so a
    /// forgotten press delays post rather than keeping it forever.
    pub idle: Duration,
    /// A letter whose last page was taken away is finished once the table has
    /// been empty this long: the next page is laid down sooner, or on top.
    /// `None`: only the button, the page or `idle` finish one.
    pub when_clear: Option<Duration>,
}

/// The last photograph taken, and what the check made of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Photographed {
    pub at: u64,
    /// Its page in the open letter, when pages are collected.
    pub page: Option<usize>,
    /// `None` when the photograph could not be read to judge it.
    pub quality: Option<Quality>,
    /// The page small, for the LCD. From the same reading as the judgement.
    pub thumbnail: Option<Picture>,
    /// It was an envelope: the start of a new letter.
    pub envelope: bool,
    /// Whether it went into the letter. A photograph with nothing on it is
    /// the table, and is put aside instead.
    pub kept: bool,
}

/// The largest a photograph's thumbnail is, either way.
const THUMBNAIL: u32 = 240;

/// How long after a letter is closed it can still be taken back from the
/// display.
pub const UNDO_LETTER_SECS: u64 = 600;

/// Photographs of one page taken again for their exposure, at most. Each is
/// a stop apart, so two go from the camera's choice to where the rig's white
/// paper needed it.
const RETAKES: u32 = 2;

/// Activity lines kept for the page. Capped, because without a page nobody
/// takes them.
const EVENTS: usize = 50;

/// How long a finish waits for something that keeps moving.
pub const FINISH_WAIT_SECS: u64 = 15;

pub struct Scanner {
    detector: Detector,
    /// How often the spool is looked at when nothing is being captured.
    drain_every: Duration,
    last_drain: Option<u64>,
    preview_failures: u32,
    /// `None`: every page is its own document.
    letters: Option<Letters>,
    /// When finishing was asked for. The letter closes as soon as no page is
    /// settling, or after [`FINISH_WAIT_SECS`] if something never stops moving.
    close_requested_at: Option<u64>,
    /// The camera's last frame: what the page shows, and what "learn empty
    /// table" learns.
    last_frame: Option<Vec<u8>>,
    /// Where the empty table is kept between runs.
    baseline_file: Option<PathBuf>,
    /// Which baseline the file holds.
    saved_generation: u64,
    /// Whether the baseline is known to be the empty table, rather than
    /// whatever the first frame happened to show.
    trusted_baseline: bool,
    /// Where the page's corners are in a photograph, when the camera looks at
    /// the table at an angle.
    straighten: Option<Corners>,
    last_photographed: Option<Photographed>,
    /// The folders on the shelf. None: letters are not followed after they
    /// are sent.
    folders: Vec<Folder>,
    /// Their tags' ids, once Paperless has them set up.
    folder_tags: Option<Vec<(u64, Folder)>>,
    /// Why the folders could not be set up last time, so the same failure is
    /// said once and not every thirty seconds — where it buried everything
    /// else on the page.
    folders_failed: Option<String>,
    filings: Filings,
    /// Whether the camera is watched at all. Off while scanning is stopped
    /// on the display, so nothing is photographed until someone asks.
    watching: bool,
    letters_closed: u64,
    /// Since when the table has been seen empty with a letter open.
    clear_since: Option<u64>,
    /// The open letter is so far only its envelope: the table is empty while
    /// the letter is taken out of it, which is not the letter being done.
    envelope_only: bool,
    /// The letter closed last, by file name, and when — what "Undo last
    /// letter" takes back. `None` once it has been.
    last_letter: Option<(String, u64)>,
    /// Where the letter just closed was decided to go, read off the pages by
    /// the Pi at the moment it was finished. Written beside the letter in the
    /// spool as it is queued, so it survives a restart.
    decided: Vec<Folder>,
    /// How much of the corners' area a page covers, from the pages seen:
    /// what an envelope is told apart by. Until the first, a page is taken
    /// to fill them.
    page_covers: Option<f64>,
    /// Stops from the camera's own exposure, learnt from the photographs.
    ev: f32,
    /// Reads each page here, for the preview, when there is something to read
    /// it with.
    reader: Option<Reader>,
    /// What the pages of the open letter say, as the preview shows them.
    page_texts: Vec<PageText>,
    /// And of the letter sent last, kept until the next letter starts: the
    /// preview still has something to show while Paperless reads it.
    sent_texts: Vec<PageText>,
    events: Vec<Event>,
}

impl Scanner {
    pub fn new(settle_frames: u8, drain_every: Duration) -> Self {
        Self {
            detector: Detector::new(Thresholds::default(), settle_frames),
            drain_every,
            last_drain: None,
            preview_failures: 0,
            letters: None,
            close_requested_at: None,
            last_frame: None,
            baseline_file: None,
            saved_generation: 0,
            trusted_baseline: false,
            straighten: None,
            last_photographed: None,
            folders: Vec::new(),
            folder_tags: None,
            folders_failed: None,
            filings: Filings::default(),
            watching: true,
            letters_closed: 0,
            clear_since: None,
            envelope_only: false,
            decided: Vec::new(),
            page_covers: None,
            last_letter: None,
            ev: 0.0,
            reader: None,
            page_texts: Vec::new(),
            sent_texts: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Reads every page as it is photographed, for the preview.
    pub fn reading_with(mut self, reader: Reader) -> Self {
        self.reader = Some(reader);
        self
    }

    /// What the pages of the letter in front of the camera say — the open
    /// one, or the one just sent while Paperless is still reading it.
    pub fn read_pages(&self) -> &[PageText] {
        if self.page_texts.is_empty() {
            &self.sent_texts
        } else {
            &self.page_texts
        }
    }

    /// Where that text says the letter goes, before Paperless has said: the
    /// one folder it fits best, and everybody it is for.
    ///
    /// One folder, not every folder it matches. A letter is a piece of paper
    /// and goes in one place on the shelf, so naming two is asking for a
    /// decision the rig is there to make. People are different — a letter can
    /// be for two of them — and are all kept.
    pub fn guessed(&self) -> Vec<Folder> {
        let text = self.read_text();
        let ranked = crate::folders::rank(&text, &self.folders);
        let best = ranked
            .iter()
            .find(|(folder, _)| !folder.person)
            .map(|(folder, _)| (*folder).clone());
        best.into_iter()
            .chain(
                ranked
                    .iter()
                    .filter(|(folder, _)| folder.person)
                    .map(|(folder, _)| (*folder).clone()),
            )
            .collect()
    }

    /// Paperless's answer put in the order the rig would have put it in: the
    /// folder the letter fits best first.
    ///
    /// Paperless tags a document with every folder whose words it matches,
    /// and it is right to — a document can carry two labels. The paper
    /// cannot: it goes in one folder, so the one named first had better be
    /// the one it belongs in. Ranked by the rig's own reading of the pages,
    /// which is the only measure it has; with nothing read, Paperless's order
    /// stands.
    fn best_first(&self, folders: Vec<Folder>) -> Vec<Folder> {
        let text = self.read_text();
        if text.trim().is_empty() {
            return folders;
        }
        let hits: Vec<(String, bool, usize)> = crate::folders::rank(&text, &folders)
            .into_iter()
            .map(|(folder, hits)| (folder.name.clone(), folder.person, hits))
            .collect();
        let score = |folder: &Folder| {
            hits.iter()
                .find(|(name, person, _)| *name == folder.name && *person == folder.person)
                .map_or(0, |(_, _, hits)| *hits)
        };
        let mut folders = folders;
        // People keep their place at the back, where the display expects them.
        folders.sort_by(|a, b| {
            a.person
                .cmp(&b.person)
                .then_with(|| score(b).cmp(&score(a)))
        });
        folders
    }

    /// Everything the Pi has read of the letter in front of the camera.
    fn read_text(&self) -> String {
        self.read_pages()
            .iter()
            .map(|page| page.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Takes in whatever the reader has finished since the last turn.
    fn collect_texts(&mut self) {
        let Some(reader) = &self.reader else {
            return;
        };
        for page in reader.texts() {
            match self
                .page_texts
                .iter_mut()
                .find(|known| known.name == page.name)
            {
                Some(known) => *known = page,
                None => self.page_texts.push(page),
            }
        }
    }

    /// Collects pages into letters instead of sending each on its own.
    pub fn collecting(mut self, letters: Letters) -> Self {
        self.letters = Some(letters);
        self
    }

    /// Keeps the empty table in `path` between runs, loading it now if there
    /// is one.
    ///
    /// Learnt from the first frame, "empty" is whatever lay under the camera at
    /// start. A page lying there becomes the table; lifting it reads as a page
    /// and is photographed, and the detector then waits for that page to come
    /// back before it will photograph anything else. Kept on disk, a restart
    /// with a letter on the table sees the letter.
    pub fn keep_baseline_in(mut self, path: PathBuf) -> Self {
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => {
                self.detector.learn_baseline(&bytes);
                self.trusted_baseline = true;
                tracing::info!(file = %path.display(), "remembered the empty table");
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(%err, "could not read the saved empty table"),
        }
        self.saved_generation = self.detector.baseline_generation();
        self.baseline_file = Some(path);
        self
    }

    /// Warps every photograph so that these corners, as fractions of it,
    /// become its own, in order — which is also how a photograph is turned.
    /// `None` keeps photographs as the camera takes them.
    pub fn straighten_with(&mut self, corners: Option<Corners>) {
        self.straighten = corners;
    }

    pub fn collecting_letters(&self) -> bool {
        self.letters.is_some()
    }

    pub fn state(&self) -> State {
        self.detector.state()
    }

    /// How wide the camera's preview frames are.
    pub fn frame_width(&mut self, width: u32) {
        self.detector.frame_width(width as usize);
    }

    pub fn settle_frames(&self) -> u8 {
        self.detector.settle_frames()
    }

    /// Whether the empty table has really been seen, as opposed to guessed.
    pub fn has_baseline(&self) -> bool {
        self.trusted_baseline && self.detector.has_baseline()
    }

    /// Says which of these folders each letter goes in, once Paperless has
    /// read it. Their tags are set up in Paperless before the next letter is
    /// sent — again, whenever this is called, because the words may have
    /// changed or the Paperless may be a different one.
    pub fn file_into(&mut self, folders: Vec<Folder>) {
        self.folders = folders;
        self.folder_tags = None;
    }

    /// Sets the folders' tags up in Paperless now, and says how many there
    /// are.
    pub async fn sync_folders(&mut self, uploader: &Uploader) -> Result<usize> {
        let tags = uploader.sync_folders(&self.folders).await?;
        let count = tags.len();
        self.folder_tags = Some(tags);
        Ok(count)
    }

    /// A letter was closed into `letter`.
    fn closed(&mut self, spool: &Spool, letter: &std::path::Path, now: u64) {
        // What its pages said stays in front of whoever is standing there
        // until the next letter starts.
        self.sent_texts = std::mem::take(&mut self.page_texts);
        // And where it goes is settled here, while the paper is still in
        // somebody's hand, rather than later by Paperless. The folder on the
        // panel is the drawer the paper goes in; it has to be the folder the
        // record ends up with, or kuverta cannot lead you back to the paper.
        self.decided = self.guessed();
        let names: Vec<String> = self
            .decided
            .iter()
            .map(|folder| folder.tag_name())
            .collect();
        spool.record_folders(letter, &names);
        self.letters_closed += 1;
        self.envelope_only = false;
        self.last_letter = letter
            .file_name()
            .map(|name| (name.to_string_lossy().into_owned(), now));
    }

    /// Whether "Undo last letter" has something to take back: a letter closed
    /// in the last ten minutes, not taken back already.
    pub fn can_undo_letter(&self, now: u64) -> bool {
        self.last_letter
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_sub(*at) < UNDO_LETTER_SECS)
    }

    /// "Undo last page": the newest page of the open letter goes to the
    /// spool's `discarded/`. False when there is none.
    pub fn undo_page(&mut self, spool: &Spool, now: u64) -> bool {
        match spool.discard_last_page() {
            Ok(Some(_)) => {
                let left = spool.open_pages().map(|pages| pages.len()).unwrap_or(0);
                self.event(
                    now,
                    true,
                    format!("took the last page back; {left} left in this letter"),
                );
                self.last_photographed = None;
                if left == 0 {
                    self.envelope_only = false;
                }
                true
            }
            Ok(None) => {
                self.event(now, false, "no page to take back");
                false
            }
            Err(err) => {
                self.event(now, false, format!("could not take the page back: {err:#}"));
                false
            }
        }
    }

    /// "Cancel letter": every page of the open letter goes to `discarded/`,
    /// and says how many.
    pub fn cancel_letter(&mut self, spool: &Spool, now: u64) -> usize {
        match spool.discard_open_letter() {
            Ok(pages) => {
                let plural = if pages == 1 { "" } else { "s" };
                self.event(
                    now,
                    true,
                    format!("cancelled the letter: {pages} page{plural} thrown away"),
                );
                self.close_requested_at = None;
                self.clear_since = None;
                self.envelope_only = false;
                self.last_photographed = None;
                pages
            }
            Err(err) => {
                self.event(now, false, format!("could not cancel the letter: {err:#}"));
                0
            }
        }
    }

    /// Throws one letter waiting to be sent away, into `discarded/`.
    pub fn discard_queued(&mut self, spool: &Spool, name: &str, now: u64) -> bool {
        match spool.discard_queued(name) {
            Ok(true) => {
                self.event(now, true, format!("threw {name} away before it was sent"));
                if self
                    .last_letter
                    .as_ref()
                    .is_some_and(|(last, _)| last == name)
                {
                    self.last_letter = None;
                }
                true
            }
            Ok(false) => {
                self.event(now, false, "that letter is not waiting any more");
                false
            }
            Err(err) => {
                self.event(now, false, format!("could not throw it away: {err:#}"));
                false
            }
        }
    }

    /// Whether the last letter can still be filed by hand: it was sent, and
    /// it is recent enough that the person standing there means this one.
    pub fn can_refile(&self, now: u64) -> bool {
        self.last_letter.as_ref().is_some_and(|(name, at)| {
            now.saturating_sub(*at) < UNDO_LETTER_SECS
                && self.filings.named_ref(name).is_some_and(|filing| {
                    !matches!(filing.outcome, Outcome::Deleted | Outcome::Failed(_))
                })
        })
    }

    /// "No folder" and "Throw away" on the display: the letter stays in
    /// Paperless, but where it belongs is said by the person holding the
    /// paper rather than by what Paperless made of the words on it.
    ///
    /// Paperless may still be reading it, and the answer is wanted before
    /// then — that is the point of the buttons. In that case the letter is
    /// marked, and filed this way the moment it is read.
    pub async fn refile_letter(&mut self, uploader: &Uploader, into: Refiling, now: u64) {
        if !self.can_refile(now) {
            self.event(now, false, "no letter to file");
            return;
        }
        let Some((name, _)) = self.last_letter.clone() else {
            return;
        };
        if into == Refiling::Bin && self.bin().is_none() {
            self.event(
                now,
                false,
                "no folder is marked as the bin: mark one on the setup page",
            );
            return;
        }
        if let Refiling::Into(name) = &into {
            if self.named_folder(name).is_none() {
                self.event(now, false, format!("there is no folder called {name:?}"));
                return;
            }
        }
        let Some(filing) = self.filings.named(&name) else {
            self.event(now, false, "that letter was not sent to Paperless");
            return;
        };
        let (task, document) = (filing.task.clone(), filing.document);
        let Some(document) = document else {
            filing.refile = Some(into.clone());
            let words = words_for(&into, true);
            self.event(now, true, format!("the last letter {words}"));
            return;
        };
        match self.refile_document(uploader, document, into.clone()).await {
            Ok(()) => {
                let outcome = self.outcome_for(&into);
                self.filings.decide(&task, outcome, now);
                let words = words_for(&into, false);
                self.event(now, true, format!("the last letter {words}"));
            }
            Err(err) => self.event(now, false, format!("could not file the letter: {err:#}")),
        }
    }

    /// The bin among the folders, if one is marked.
    fn bin(&self) -> Option<Folder> {
        self.folders.iter().find(|folder| folder.discard).cloned()
    }

    /// A folder by name, whatever its case.
    fn named_folder(&self, name: &str) -> Option<Folder> {
        self.folders
            .iter()
            .find(|folder| !folder.person && folder.name.eq_ignore_ascii_case(name.trim()))
            .cloned()
    }

    /// What a letter's outcome becomes once it is filed by hand.
    fn outcome_for(&self, into: &Refiling) -> Outcome {
        match into {
            Refiling::Nowhere => Outcome::Nowhere,
            Refiling::Bin => Outcome::Folders(self.bin().into_iter().collect()),
            Refiling::Into(name) => Outcome::Folders(self.named_folder(name).into_iter().collect()),
        }
    }

    /// Takes the folders' tags off a document in Paperless, and puts the
    /// bin's on when that is where it goes. Tags that are not folders — the
    /// address, who the post is for — are left alone.
    async fn refile_document(
        &self,
        uploader: &Uploader,
        document: u64,
        into: Refiling,
    ) -> anyhow::Result<()> {
        let known = self.folder_tags.as_deref().unwrap_or_default();
        let folder_tags: Vec<u64> = known
            .iter()
            .filter(|(_, folder)| !folder.person)
            .map(|(id, _)| *id)
            .collect();
        let bin_tag = known
            .iter()
            .find(|(_, folder)| folder.discard)
            .map(|(id, _)| *id);
        let mut tags: Vec<u64> = uploader
            .document_tags(document)
            .await?
            .into_iter()
            .filter(|tag| !folder_tags.contains(tag))
            .collect();
        let wanted = match &into {
            Refiling::Nowhere => None,
            Refiling::Bin => bin_tag,
            Refiling::Into(name) => known
                .iter()
                .find(|(_, folder)| !folder.person && folder.name.eq_ignore_ascii_case(name.trim()))
                .map(|(id, _)| *id),
        };
        if let Some(tag) = wanted {
            tags.push(tag);
        }
        uploader.set_document_tags(document, &tags).await
    }

    /// "Undo last letter": out of the queue if it is still waiting there;
    /// otherwise deleted from Paperless — into its trash — now, or as soon as
    /// Paperless has read it.
    pub async fn undo_letter(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        if !self.can_undo_letter(now) {
            self.event(now, false, "no letter to take back");
            return;
        }
        let Some((name, _)) = self.last_letter.take() else {
            return;
        };
        match spool.discard_queued(&name) {
            Ok(true) => {
                self.event(now, true, "took the last letter back before it was sent");
                return;
            }
            Ok(false) => {}
            Err(err) => {
                self.event(
                    now,
                    false,
                    format!("could not take the letter back: {err:#}"),
                );
                return;
            }
        }
        let Some(filing) = self.filings.named(&name) else {
            self.event(
                now,
                false,
                "the last letter was sent, but Paperless's answer is not known: delete it there",
            );
            return;
        };
        match filing.document {
            Some(document) => {
                let task = filing.task.clone();
                match uploader.delete_document(document).await {
                    Ok(()) => {
                        self.filings.decide(&task, Outcome::Deleted, now);
                        self.event(now, true, "took the last letter back: deleted from Paperless (its trash keeps it)");
                    }
                    Err(err) => self.event(
                        now,
                        false,
                        format!("could not delete the last letter from Paperless: {err:#}"),
                    ),
                }
            }
            None => {
                filing.undo = true;
                self.event(
                    now,
                    true,
                    "taking the last letter back: it is deleted as soon as Paperless has read it",
                );
            }
        }
    }

    /// Starts or stops watching the camera. Stopped, a turn takes no frame at
    /// all: the camera is left alone and nothing can be photographed.
    pub fn watch(&mut self, watching: bool) {
        self.watching = watching;
    }

    pub fn watching(&self) -> bool {
        self.watching
    }

    /// The exposure the next page is taken at, in stops from the camera's own
    /// choice. Learnt from each photograph; kept by `main` between runs.
    pub fn exposure(&self) -> f32 {
        self.ev
    }

    pub fn expose_at(&mut self, ev: f32) {
        self.ev = ev.clamp(crate::quality::EV_DARKEST, crate::quality::EV_BRIGHTEST);
    }

    /// How many letters have been closed since start — by the button, the
    /// page, or being left alone.
    pub fn letters_closed(&self) -> u64 {
        self.letters_closed
    }

    /// The letter sent last and where it goes, while it is worth showing.
    pub fn filing(&self) -> Option<&Filing> {
        self.filings.latest()
    }

    /// The last photograph and its verdict, for the display.
    pub fn last_photographed(&self) -> Option<&Photographed> {
        self.last_photographed.as_ref()
    }

    pub fn last_frame(&self) -> Option<&[u8]> {
        self.last_frame.as_deref()
    }

    /// What happened since this was last asked, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The page's "Finish letter". Goes through the same wait as the button,
    /// so a page still settling is not left out, and says so at once. False
    /// when pages are not being collected.
    pub fn request_close(&mut self, now: u64) -> bool {
        if self.letters.is_none() {
            return false;
        }
        if self.close_requested_at.is_none() {
            self.close_requested_at = Some(now);
            self.event(now, true, "finishing the letter");
        }
        true
    }

    /// Seconds until the open letter is finished for the table being clear,
    /// while that is counting down — for the display to say so.
    pub fn finishing_in(&self, now: u64) -> Option<u64> {
        let after = self.letters.as_ref()?.when_clear?.as_secs();
        let since = self.clear_since?;
        if self.close_requested_at.is_some() || !self.watching {
            return None;
        }
        Some(after.saturating_sub(now.saturating_sub(since)))
    }

    /// Whether finishing was asked for and the letter is not closed yet.
    pub fn finishing(&self) -> bool {
        self.close_requested_at.is_some()
    }

    /// The last frame's change from the empty table and from the frame before.
    pub fn measure(&self) -> Option<(f32, f32)> {
        self.detector.last_measure()
    }

    /// "The table is empty now": learns the last frame as the empty table and
    /// arms again. False when there is no frame yet to learn from.
    pub fn learn_empty(&mut self, now: u64) -> bool {
        let Some(frame) = self.last_frame.clone() else {
            self.event(
                now,
                false,
                "no picture from the camera yet to learn the table from",
            );
            return false;
        };
        self.detector.learn_empty(&frame);
        self.trusted_baseline = true;
        self.save_baseline();
        self.event(now, true, "learnt the empty table");
        true
    }

    /// "The table is empty now", from a frame taken for the purpose: the last
    /// one may be minutes old, or there may be none, when scanning is stopped.
    pub fn learn_empty_now(&mut self, camera: &dyn Camera, now: u64) -> bool {
        match camera.preview() {
            Ok(frame) => {
                self.last_frame = Some(frame);
                self.learn_empty(now)
            }
            Err(err) => {
                self.event(
                    now,
                    false,
                    format!("could not see the table to learn it: {err:#}"),
                );
                false
            }
        }
    }

    /// Forgets the empty table, as when the crop changes and the camera no
    /// longer shows the same part of it.
    pub fn forget_baseline(&mut self) {
        self.detector.forget_baseline();
        self.trusted_baseline = false;
        if let Some(path) = &self.baseline_file {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Looks at one frame, captures if a page has settled, closes the letter if
    /// it is finished, and sends whatever is due. `now` is seconds since the
    /// epoch.
    pub async fn turn(
        &mut self,
        camera: &dyn Camera,
        spool: &Spool,
        uploader: &Uploader,
        now: u64,
    ) -> Turn {
        // Whatever the reader finished while the last turn was taken.
        self.collect_texts();
        if self.close_if_due(spool, now) {
            // The letter someone just finished goes now, not on the timer.
            self.drain_now(spool, uploader, now).await;
        }

        let collecting = self.letters.is_some();
        let preview = if self.watching {
            camera.preview()
        } else {
            if self.drain_due(now) {
                self.drain_now(spool, uploader, now).await;
            }
            self.follow(uploader, now).await;
            return Turn::Paused;
        };
        let turn = match preview {
            Ok(frame) => {
                self.preview_failures = 0;
                let before = (self.detector.state(), self.detector.baseline_generation());
                let step = self.detector.observe(&frame);
                if self.detector.baseline_generation() != before.1 {
                    // Relearnt because the table was seen clear after a
                    // photograph: that is the empty table. Learnt from a first
                    // frame, or because the size changed: a guess, not kept.
                    self.trusted_baseline = before.0 == State::Spent;
                    self.save_baseline();
                }
                self.last_frame = Some(frame);

                if step == Step::Capture {
                    let taking =
                        capture(camera, spool, collecting, self.straighten.as_ref(), self.ev);
                    match taking {
                        Ok(Taken {
                            path,
                            warning,
                            quality,
                            thumbnail,
                            retakes,
                            ev,
                            next_ev,
                            covered,
                        }) => {
                            // Paper first: on the rig a photograph of the
                            // dark table covered 0.71 of the corners, which is
                            // an envelope's share — but there was no paper in
                            // it at all, and the check had already said so.
                            let is_paper = quality
                                .as_ref()
                                .is_none_or(|quality| !quality.problems.contains(&Problem::NoPage));
                            // Nothing is an envelope until a page has been
                            // seen. An envelope is told apart by being
                            // smaller than a page, so with no page to measure
                            // against there is nothing to tell — and guessing
                            // costs more than waiting: an envelope taken for
                            // a page is page one of the letter it holds,
                            // which is where it belongs anyway, while a page
                            // taken for an envelope cuts a letter in two.
                            // The corners are set generously round where
                            // letters land, so a page covers about 0.7 of
                            // them and sat right on the threshold.
                            // Nothing is an envelope until a page has been
                            // seen. An envelope is told apart by being
                            // smaller than a page, so with no page to measure
                            // against there is nothing to tell — and guessing
                            // costs more than waiting: an envelope taken for a
                            // page becomes page one of the letter it holds,
                            // which is where it belongs anyway, while a page
                            // taken for an envelope cuts a letter in two. It
                            // used to be measured against the whole of the
                            // corners, and the corners are set generously
                            // round where letters land — so a page covers
                            // about 0.7 of them and sat on the threshold.
                            let envelope = is_paper
                                && self.page_covers.is_some_and(|page| {
                                    crate::straighten::is_envelope(covered, page)
                                });
                            if is_paper && !envelope {
                                // A page: learnt, a little at a time, so one
                                // odd photograph does not move it far. Paper
                                // whose edge could not be measured is paper
                                // filling the corners, which is what corners
                                // set close round a page look like — and that
                                // is worth learning, because otherwise a rig
                                // set up that way never learns anything and
                                // can never tell an envelope at all.
                                let seen = covered.unwrap_or(1.0);
                                self.page_covers = Some(match self.page_covers {
                                    Some(known) => known * 0.7 + seen * 0.3,
                                    None => seen,
                                });
                            }
                            if let Some(warning) = warning {
                                self.event(now, false, warning);
                            }
                            // An envelope is a new letter: whatever was being
                            // collected is finished — and sent now, so its
                            // folder is known by the time this one's is asked —
                            // and the envelope is its first page.
                            let mut sent_before = false;
                            if envelope && collecting {
                                match spool.close_letter_except(&path) {
                                    Ok(Some(letter)) => {
                                        tracing::info!(letter = %letter.display(), "letter closed by the next envelope");
                                        self.event(now, true, "an envelope: the letter before it is finished, sending");
                                        self.close_requested_at = None;
                                        self.closed(spool, &letter, now);
                                        sent_before = true;
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        tracing::error!(%err, "could not finish the letter before the envelope");
                                        self.event(now, false, format!("could not finish the letter before the envelope: {err:#}"));
                                    }
                                }
                            }
                            self.envelope_only = envelope && collecting;
                            if retakes > 0 {
                                self.event(
                                    now,
                                    true,
                                    format!(
                                        "took the page {retakes} more time{} for its exposure, \
                                         keeping the one at {ev:+.1} EV",
                                        if retakes == 1 { "" } else { "s" }
                                    ),
                                );
                            }
                            if next_ev != self.ev {
                                tracing::info!(from = self.ev, to = next_ev, "exposure");
                            }
                            self.ev = next_ev;
                            let page = collecting
                                .then(|| spool.open_pages().map(|pages| pages.len()).unwrap_or(0));
                            let text = match page {
                                _ if envelope => {
                                    "photographed the envelope of a new letter".to_string()
                                }
                                Some(page) => format!("photographed page {page} of this letter"),
                                None => "photographed a page".to_string(),
                            };
                            // Nothing on it: the table, photographed because
                            // it no longer looks like the one that was learnt
                            // — a page taken away, the light changed, the
                            // paper under the camera moved. It is not a page
                            // of the letter, so it is not kept, and the view
                            // is learnt as the table so the next one is not
                            // taken either.
                            let nothing_on_it = quality.as_ref().is_some_and(|quality| {
                                quality.problems.iter().any(|problem| {
                                    matches!(problem, Problem::NoText | Problem::NoPage)
                                })
                            });
                            let kept = !nothing_on_it;
                            if nothing_on_it {
                                if let Err(err) = spool.discard_capture(&path) {
                                    tracing::warn!(%err, "could not put the table's photograph aside");
                                }
                                if let Some(frame) = self.last_frame.clone() {
                                    self.detector.learn_empty(&frame);
                                    self.trusted_baseline = true;
                                    self.save_baseline();
                                }
                                self.event(
                                    now,
                                    false,
                                    "nothing on it: that was the table, not a page — not kept, and \
                                     the table learnt again",
                                );
                            } else {
                                self.event(now, true, text);
                            }
                            if let Some(quality) =
                                quality.as_ref().filter(|q| !q.ok() && !envelope && kept)
                            {
                                let which = match page {
                                    Some(page) => format!("page {page}"),
                                    None => "the page".to_string(),
                                };
                                self.event(
                                    now,
                                    false,
                                    format!("{which} may not be readable: {}", quality.summary()),
                                );
                            }
                            // Read here, badly and at once, so the preview can
                            // say what the page looks like before Paperless
                            // has it. Paperless's answer replaces the guess.
                            if kept {
                                // A page of a new letter: what the last one
                                // said is not this one's.
                                if self.page_texts.is_empty() {
                                    self.sent_texts.clear();
                                    spool.clear_sent();
                                }
                                if let Some(reader) = &self.reader {
                                    reader.start(&path);
                                }
                            }
                            self.last_photographed = Some(Photographed {
                                at: now,
                                page,
                                quality,
                                thumbnail,
                                envelope,
                                kept,
                            });
                            // A single page goes straight away: it is the
                            // letter someone is standing next to. A page of a
                            // letter waits for the rest — unless an envelope
                            // just finished the one before. Nothing is sent
                            // for a photograph of the table.
                            if (!collecting && kept) || sent_before {
                                self.drain_now(spool, uploader, now).await;
                            }
                            return Turn::Captured(path);
                        }
                        Err(err) => {
                            tracing::error!(%err, "capture failed");
                            self.event(
                                now,
                                false,
                                format!("could not photograph the page: {err:#}"),
                            );
                            Turn::Watching
                        }
                    }
                } else {
                    Turn::Watching
                }
            }
            Err(err) => {
                self.preview_failures += 1;
                // A camera that has been unplugged should say so once and keep
                // trying, not fill a log at four lines a second.
                if self.preview_failures <= 3 || self.preview_failures.is_multiple_of(50) {
                    tracing::warn!(%err, failures = self.preview_failures, "no preview frame");
                }
                if self.preview_failures == 1 {
                    self.event(now, false, format!("the camera gave no picture: {err:#}"));
                }
                Turn::NoFrame
            }
        };

        // On a timer as well as after a capture. Draining only after a capture
        // meant a letter photographed while the network was down waited for
        // the next letter — or a restart — however soon the network came
        // back. And whatever the camera is doing: a broken camera is no reason
        // to keep post that is already photographed.
        if self.drain_due(now) {
            self.drain_now(spool, uploader, now).await;
        }
        self.follow(uploader, now).await;
        turn
    }

    /// Sends everything that is due, now.
    pub async fn drain_now(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        self.last_drain = Some(now);
        self.prepare_folders(spool, uploader, now).await;
        let filings = (!self.folders.is_empty()).then_some(&mut self.filings);
        drain(spool, uploader, now, false, &mut self.events, filings).await;
        self.cap_events();
    }

    /// Sends everything waiting, whatever its backoff says — for someone who
    /// has just fixed the network and does not want to wait five minutes to
    /// see whether it worked.
    pub async fn retry_all(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        self.last_drain = Some(now);
        self.prepare_folders(spool, uploader, now).await;
        let filings = (!self.folders.is_empty()).then_some(&mut self.filings);
        drain(spool, uploader, now, true, &mut self.events, filings).await;
        self.cap_events();
    }

    /// Before anything is sent: the folders' tags, so Paperless can give them
    /// to the letter as it reads it. Only when there is something to send,
    /// so an unreachable Paperless is not asked twice every thirty seconds.
    /// A failure is said and the letter sent anyway — it is filed without a
    /// folder, not kept back.
    async fn prepare_folders(&mut self, spool: &Spool, uploader: &Uploader, now: u64) {
        if self.folders.is_empty() || self.folder_tags.is_some() {
            return;
        }
        if spool
            .pending()
            .map(|pending| pending.is_empty())
            .unwrap_or(true)
        {
            return;
        }
        match self.sync_folders(uploader).await {
            Ok(count) => {
                tracing::info!(count, "folders set up in Paperless");
                if self.folders_failed.take().is_some() {
                    self.event(now, true, "the folders are set up in Paperless now");
                }
            }
            Err(err) => {
                let text = format!("could not set up the folders in Paperless: {err:#}");
                if self.folders_failed.as_deref() != Some(text.as_str()) {
                    tracing::warn!(%err, "could not set up the folders in Paperless");
                    self.event(now, false, text.clone());
                    self.folders_failed = Some(text);
                }
            }
        }
    }

    /// Asks Paperless about one letter it is reading, if one is due a
    /// question, and says where it goes once it knows.
    async fn follow(&mut self, uploader: &Uploader, now: u64) {
        let Some(filing) = self.filings.next_to_check(now) else {
            return;
        };
        let outcome = match uploader.task(&filing.task).await {
            Ok(Task::Working) => return,
            Ok(Task::Done(document)) => {
                if let Some(known) = self.filings.named(&filing.name) {
                    known.document = Some(document);
                }
                // Taken back while Paperless was still reading it.
                if filing.undo {
                    match uploader.delete_document(document).await {
                        Ok(()) => {
                            self.event(
                                now,
                                true,
                                format!(
                                    "{} was taken back and deleted from Paperless",
                                    filing.name
                                ),
                            );
                            self.filings.decide(&filing.task, Outcome::Deleted, now);
                        }
                        Err(err) => {
                            tracing::warn!(%err, document, "could not delete a letter taken back");
                            self.event(
                                now,
                                false,
                                format!("could not delete {} from Paperless: {err:#}", filing.name),
                            );
                        }
                    }
                    return;
                }
                if self.folder_tags.is_none() {
                    if let Err(err) = self.sync_folders(uploader).await {
                        tracing::warn!(%err, "could not look the folders up");
                        return;
                    }
                }
                // Filed by hand from the display while Paperless was still
                // reading it: where it goes was decided by whoever held the
                // paper, and that wins over what the words say.
                if let Some(into) = filing.refile.clone() {
                    match self.refile_document(uploader, document, into.clone()).await {
                        Ok(()) => {
                            let outcome = self.outcome_for(&into);
                            self.filings.decide(&filing.task, outcome, now);
                            self.event(
                                now,
                                true,
                                format!("{} {}", filing.name, words_for(&into, false)),
                            );
                        }
                        Err(err) => {
                            tracing::warn!(%err, document, "could not file a letter by hand");
                            self.event(
                                now,
                                false,
                                format!("could not file {}: {err:#}", filing.name),
                            );
                        }
                    }
                    return;
                }
                match uploader.document_tags(document).await {
                    Ok(tags) => Outcome::Folders(self.best_first(folders_among(
                        &tags,
                        self.folder_tags.as_deref().unwrap_or_default(),
                    ))),
                    Err(err) => {
                        tracing::warn!(%err, document, "could not read the document's tags");
                        return;
                    }
                }
            }
            Ok(Task::Duplicate) => Outcome::Duplicate,
            Ok(Task::Failed(why)) => Outcome::Failed(why),
            Err(err) => {
                tracing::debug!(%err, "could not ask Paperless about a letter");
                return;
            }
        };
        let (ok, text) = match &outcome {
            Outcome::Folders(folders) if folders.iter().any(|f| f.discard) => {
                (true, "can be thrown away".to_string())
            }
            Outcome::Folders(folders) if !folders.is_empty() => {
                let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
                (true, format!("goes in {}", names.join(" or ")))
            }
            Outcome::Folders(_) => (false, "matched no folder".to_string()),
            Outcome::Duplicate => (true, "was in Paperless already".to_string()),
            Outcome::Failed(why) => (false, format!("was not filed: {why}")),
            Outcome::Reading | Outcome::TimedOut => (false, "is still being read".to_string()),
            Outcome::Deleted => (
                true,
                "was taken back and deleted from Paperless".to_string(),
            ),
            Outcome::Nowhere => (true, "is kept in no folder".to_string()),
        };
        tracing::info!(letter = %filing.name, %text, "filed");
        self.event(now, ok, format!("{} {text}", filing.name));
        self.filings.decide(&filing.task, outcome, now);
    }

    fn drain_due(&self, now: u64) -> bool {
        match self.last_drain {
            None => true,
            // A clock that went backwards (no RTC on a Pi) must not postpone
            // retries until it catches up.
            Some(last) => now < last || now - last >= self.drain_every.as_secs(),
        }
    }

    fn event(&mut self, at: u64, ok: bool, text: impl Into<String>) {
        self.events.push(Event {
            at,
            ok,
            text: text.into(),
        });
        self.cap_events();
    }

    fn cap_events(&mut self) {
        if self.events.len() > EVENTS {
            let extra = self.events.len() - EVENTS;
            self.events.drain(..extra);
        }
    }

    /// Writes the baseline if it is trusted and not already on disk.
    fn save_baseline(&mut self) {
        let generation = self.detector.baseline_generation();
        if !self.trusted_baseline || generation == self.saved_generation {
            return;
        }
        let (Some(path), Some(baseline)) = (&self.baseline_file, self.detector.baseline()) else {
            return;
        };
        // Written whole and renamed: a half-written baseline would be one of
        // the wrong size, which the detector would then have to throw away.
        let partial = path.with_extension("partial");
        match std::fs::write(&partial, baseline).and_then(|()| std::fs::rename(&partial, path)) {
            Ok(()) => self.saved_generation = generation,
            Err(err) => tracing::warn!(%err, "could not save the empty table"),
        }
    }

    /// Closes the open letter if finishing was asked for or it has been left
    /// alone, and says whether a letter was closed.
    fn close_if_due(&mut self, spool: &Spool, now: u64) -> bool {
        let (pressed, idle, when_clear) = match &self.letters {
            Some(letters) => (letters.button.pressed(), letters.idle, letters.when_clear),
            None => return false,
        };
        if pressed && self.close_requested_at.is_none() {
            self.close_requested_at = Some(now);
        }

        // The last page taken away and nothing put down after it: the letter
        // is done. Only while the camera is watched — stopped, the table is
        // not looked at, and an empty-looking one means nothing.
        let open = spool.last_page_at().ok().flatten().is_some();
        if self.watching && open && !self.envelope_only && self.detector.state() == State::Waiting {
            let since = *self.clear_since.get_or_insert(now);
            if let Some(after) = when_clear {
                if now.saturating_sub(since) >= after.as_secs() && self.close_requested_at.is_none()
                {
                    tracing::info!(
                        "the table has been clear since the last page: finishing the letter"
                    );
                    self.close_requested_at = Some(now);
                }
            }
        } else {
            self.clear_since = None;
        }

        // Not while a page is settling. Putting the last page down and
        // pressing the button is one movement, and the press arrives before
        // the page has been still long enough to photograph — closing then
        // would send the letter without its last page.
        //
        // But not for ever. On the Pi, with the room in view, something was
        // always moving, and a press on the page did nothing at all until the
        // letter closed itself five minutes later. After a while the press
        // wins; anything not yet photographed starts the next letter.
        if matches!(self.detector.state(), State::Settling { .. }) {
            let waited = self
                .close_requested_at
                .is_some_and(|at| now.saturating_sub(at) >= FINISH_WAIT_SECS);
            if !waited {
                return false;
            }
        }

        let left_alone = match spool.last_page_at() {
            // Not when the clock went backwards: the button still works, and
            // closing a letter early is worse than closing it late.
            Ok(Some(at)) => now >= at && now - at >= idle.as_secs(),
            Ok(None) => false,
            Err(err) => {
                tracing::error!(%err, "could not read the open letter");
                false
            }
        };
        let requested = self.close_requested_at.is_some();
        if !requested && !left_alone {
            return false;
        }
        self.close_requested_at = None;

        let pages = spool.open_pages().map(|pages| pages.len()).unwrap_or(0);
        match spool.close_letter() {
            Ok(Some(letter)) => {
                self.closed(spool, &letter, now);
                let how = if requested { "finished" } else { "left alone" };
                tracing::info!(letter = %letter.display(), how, "letter closed");
                let plural = if pages == 1 { "" } else { "s" };
                self.event(
                    now,
                    true,
                    format!("letter of {pages} page{plural} {how}, sending"),
                );
                true
            }
            Ok(None) => {
                tracing::info!("finishing asked for with no pages photographed; nothing to close");
                self.event(now, true, "nothing to finish: no pages photographed yet");
                false
            }
            Err(err) => {
                tracing::error!(%err, "could not close the letter; its pages are kept");
                self.event(
                    now,
                    false,
                    format!("could not finish the letter (its pages are kept): {err:#}"),
                );
                false
            }
        }
    }
}

/// One photograph of a page, of those taken for its exposure: the best so far.
struct Take {
    quality: Option<Quality>,
    thumbnail: Option<Picture>,
    ev: f32,
    /// How much of the corners' area the paper covered.
    covered: Option<f64>,
}

/// A page photographed and in the spool.
struct Taken {
    path: PathBuf,
    /// What went wrong short of losing the page.
    warning: Option<String>,
    quality: Option<Quality>,
    thumbnail: Option<Picture>,
    /// Photographs taken again for their exposure.
    retakes: u32,
    /// The exposure of the one kept.
    ev: f32,
    /// The exposure for the next page.
    next_ev: f32,
    /// How much of the corners' area the paper covered: an envelope covers
    /// less than a page.
    covered: Option<f64>,
}

/// Photographs the page at `ev`, straightens it if the camera is at an angle,
/// judges it, and takes it again darker or lighter while the exposure is what
/// makes it unreadable — keeping the best of them. Then puts it in the spool:
/// in the open letter when collecting, or straight in the queue.
fn capture(
    camera: &dyn Camera,
    spool: &Spool,
    collecting: bool,
    corners: Option<&Corners>,
    ev: f32,
) -> Result<Taken> {
    let (partial, ready) = if collecting {
        spool.reserve_page()?
    } else {
        spool.reserve()
    };
    // Retakes are written beside it under a name the spool never reads, and
    // only replace it when they are better.
    let retake = partial.with_extension("retake");

    let mut warning = None;
    let mut best: Option<Take> = None;
    let (mut ev_now, mut retakes) = (ev, 0);
    loop {
        let shot = if best.is_none() { &partial } else { &retake };
        if let Err(err) = camera.capture_at(shot, ev_now) {
            if best.is_none() {
                return Err(err).context("could not photograph the page");
            }
            // The first photograph is kept: a retake that failed is no reason
            // to lose it.
            tracing::warn!(%err, "could not take the page again");
            let _ = std::fs::remove_file(&retake);
            break;
        }

        // A page that cannot be straightened is still kept as it was taken:
        // a slanted letter is better than none.
        let mut covered = None;
        if let Some(corners) = corners {
            let started = std::time::Instant::now();
            match straighten_file(shot, corners) {
                Ok(paper) => {
                    covered = paper;
                    tracing::info!(
                        ms = started.elapsed().as_millis() as u64,
                        ?paper,
                        "straightened"
                    );
                }
                Err(err) => {
                    tracing::warn!(%err, "could not straighten the page; keeping it as taken");
                    warning = Some(format!(
                        "could not straighten the page, kept it as taken: {err:#}"
                    ));
                }
            }
        }

        let (quality, thumbnail) = judge(shot);
        let keep = match (&best, &quality) {
            (None, _) => true,
            (
                Some(Take {
                    quality: Some(kept),
                    ..
                }),
                Some(new),
            ) => better(new, kept),
            (Some(Take { quality: None, .. }), Some(_)) => true,
            (Some(_), None) => false,
        };
        if best.is_some() {
            if keep {
                std::fs::rename(&retake, &partial)
                    .context("could not keep the page taken again")?;
            } else {
                let _ = std::fs::remove_file(&retake);
            }
        }
        if keep {
            best = Some(Take {
                quality: quality.clone(),
                thumbnail,
                ev: ev_now,
                covered,
            });
        }

        match quality.as_ref().map(|quality| exposure(ev_now, quality)) {
            Some(Exposure::Retake(next)) if retakes < RETAKES => {
                tracing::info!(from = ev_now, to = next, "taking the page again");
                ev_now = next;
                retakes += 1;
            }
            _ => break,
        }
    }

    let Take {
        quality,
        thumbnail,
        ev: kept_ev,
        covered,
    } = best.expect("at least one photograph was taken");
    let next_ev = match quality.as_ref().map(|quality| exposure(kept_ev, quality)) {
        Some(Exposure::Keep(next) | Exposure::Retake(next)) => next,
        None => kept_ev,
    };

    // Only now is it a capture. Until the rename it is a file that may be
    // half-written, and nothing reads it.
    spool.commit(&partial, &ready)?;
    tracing::info!(file = %ready.display(), ev = kept_ev, retakes, "captured");
    Ok(Taken {
        path: ready,
        warning,
        quality,
        thumbnail,
        retakes,
        ev: kept_ev,
        next_ev,
        covered,
    })
}

/// Whether a photograph is good enough to read. After straightening, so the
/// page is judged as it will be sent — at the cost, on a Pi Zero, of reading
/// it a second time. A photograph that cannot be read is not judged, and is
/// sent all the same.
fn judge(path: &std::path::Path) -> (Option<Quality>, Option<Picture>) {
    let started = std::time::Instant::now();
    match std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|jpeg| read_luma(&jpeg))
    {
        Ok((luma, width, height)) => {
            let quality = assess(&luma, width, height);
            let thumbnail = Picture::scaled(&luma, width, height, THUMBNAIL, THUMBNAIL);
            tracing::info!(
                ms = started.elapsed().as_millis() as u64,
                verdict = %quality.summary(),
                ink = quality.ink,
                paper = quality.paper,
                sharpness = ?quality.sharpness,
                "judged"
            );
            (Some(quality), Some(thumbnail))
        }
        Err(err) => {
            tracing::warn!(%err, "could not judge the photograph");
            (None, None)
        }
    }
}

/// Replaces a photograph with its straightened self — whole, by rename, so a
/// crash leaves one or the other and never half of each.
fn straighten_file(path: &std::path::Path, corners: &Corners) -> Result<Option<f64>> {
    let straight = straighten_trimmed(&std::fs::read(path)?, corners)?;
    let next = path.with_extension("straight");
    std::fs::write(&next, straight.jpeg)?;
    std::fs::rename(&next, path)?;
    Ok(straight.covered)
}

/// Sends everything waiting — and due, unless `force` — oldest first.
async fn drain(
    spool: &Spool,
    uploader: &Uploader,
    now: u64,
    force: bool,
    events: &mut Vec<Event>,
    mut filings: Option<&mut Filings>,
) {
    let pending = match spool.pending() {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(%err, "could not read the spool");
            return;
        }
    };

    let due: Vec<_> = pending
        .into_iter()
        .filter(|item| force || item.due(now))
        .collect();
    if due.is_empty() {
        return;
    }
    tracing::info!(waiting = due.len(), "draining the spool");

    for item in due {
        let filename = item
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "post.jpg".to_string());

        let bytes = match std::fs::read(&item.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!(%err, file = %item.path.display(), "could not read a capture");
                continue;
            }
        };

        // Where it was decided to go, written beside it when it was closed.
        let folders = spool.folders_decided(&item.path);
        match uploader.send(&filename, bytes, &folders).await {
            Ok(task) => {
                tracing::info!(file = %filename, %task, "uploaded");
                if let Some(filings) = filings.as_deref_mut() {
                    filings.sent(&filename, &task, now);
                }
                events.push(Event {
                    at: now,
                    ok: true,
                    text: format!("sent {filename} to Paperless"),
                });
                if let Err(err) = spool.done(&item) {
                    // The upload succeeded, so this is not a failure of the
                    // capture — but it will be uploaded again next time, and
                    // Paperless will deduplicate it by checksum.
                    tracing::warn!(%err, "uploaded but could not clear the spool");
                }
            }
            Err(err) => {
                let attempts = spool.failed(&item).unwrap_or(item.attempts + 1);
                tracing::warn!(%err, file = %filename, attempts, "upload failed; kept");
                events.push(Event {
                    at: now,
                    ok: false,
                    text: format!("could not send {filename} (kept, will retry): {err:#}"),
                });
            }
        }
    }
}

/// What the activity list says about a letter filed by hand, after whatever
/// the letter is called.
fn words_for(into: &Refiling, later: bool) -> String {
    match (into, later) {
        (Refiling::Nowhere, false) => "is kept in no folder".into(),
        (Refiling::Nowhere, true) => "goes in no folder, as soon as Paperless has read it".into(),
        (Refiling::Bin, false) => "can be thrown away; Paperless keeps it".into(),
        (Refiling::Bin, true) => "goes in the bin, as soon as Paperless has read it".into(),
        (Refiling::Into(name), false) => format!("goes in {name}"),
        (Refiling::Into(name), true) => {
            format!("goes in {name}, as soon as Paperless has read it")
        }
    }
}
