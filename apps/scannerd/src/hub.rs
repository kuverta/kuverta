//! What the web page and the capture loop share.
//!
//! The loop owns the camera, the spool and the uploader, and the page must
//! never touch them: two `rpicam-still` processes at once fail, and an upload
//! started from a request would race the loop's own. So the page reads a status
//! the loop publishes after every turn, and leaves commands the loop carries out
//! at the start of the next one. A mutex each, held only for a copy.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;

use crate::folders::Folder;
use crate::settings::Settings;

/// A folder the preview thinks a letter goes in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuessedFolder {
    pub name: String,
    /// The bin: the paper can go.
    pub discard: bool,
    /// Somebody the letter is for, rather than somewhere it goes.
    pub person: bool,
}

impl From<&crate::folders::Folder> for GuessedFolder {
    fn from(folder: &crate::folders::Folder) -> Self {
        Self {
            name: folder.name.clone(),
            discard: folder.discard,
            person: folder.person,
        }
    }
}

/// Something the page asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// The on-screen twin of the button.
    FinishLetter,
    /// Start watching the camera for pages.
    StartScanning,
    /// Stop watching it: nothing is photographed until started again.
    StopScanning,
    /// Take the newest page of the open letter back.
    UndoPage,
    /// Throw the open letter away. Unconfirmed, the display asks first.
    CancelLetter {
        confirmed: bool,
    },
    /// Take the last finished letter back. Unconfirmed, the display asks
    /// first.
    UndoLetter {
        confirmed: bool,
    },
    /// File the last letter by hand rather than by what Paperless made of
    /// it: in no folder at all, or in the bin. The letter stays in Paperless
    /// either way.
    Refile(crate::folders::Refiling),
    /// Show what is waiting to be sent, on the display, or stop showing it.
    ShowQueue(bool),
    /// Throw a letter waiting to be sent away: the display knows where it is
    /// in the list, the page knows what it is called.
    DiscardQueued {
        which: Which,
        confirmed: bool,
    },
    /// Take the current frame as the empty table.
    LearnEmpty,
    /// Send everything waiting, whatever its backoff says.
    RetryNow,
    /// One frame of the whole view, uncropped, for choosing the crop.
    FullView,
    /// Connect to Paperless and say whether the token and tags work.
    CheckPaperless,
    /// Delete a page of the open letter, by file name — one photographed by
    /// mistake.
    DeletePage(String),
    /// Read a page again, cutting it up the other way.
    ReadAgain(String),
    Settings(Settings),
}

/// A line in the page's activity list.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub at: u64,
    pub ok: bool,
    pub text: String,
}

/// Which letter of the queue.
#[derive(Debug, Clone, PartialEq)]
pub enum Which {
    At(usize),
    Named(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Queued {
    pub name: String,
    pub attempts: u32,
    /// When it was photographed, from its name.
    pub at: u64,
}

/// Settings as the page may see them: the token only as whether there is one.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SettingsView {
    pub url: String,
    pub tags: Vec<String>,
    pub roi: Option<String>,
    /// The page's corners, when the camera is at an angle.
    pub corners: Option<String>,
    /// Degrees clockwise every photograph is turned.
    pub rotate: u16,
    pub token_set: bool,
    pub folders: Vec<Folder>,
    /// The exposure photographs are taken at, in stops.
    pub ev: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// `starting`, `waiting`, `settling`, `photographed` or `no-camera`.
    pub state: String,
    pub frames_still: u8,
    pub settle_frames: u8,
    /// Whether the empty table has been learnt.
    pub has_baseline: bool,
    /// Whether pages collect into letters.
    pub collecting: bool,
    /// Whether the camera is being watched for pages.
    pub scanning: bool,
    /// Whether the last letter can still be taken back.
    pub can_undo_letter: bool,
    /// Whether the last letter can still be filed by hand: in no folder, or
    /// in the bin.
    pub can_refile: bool,
    /// How much of the last frame looked like paper lying on the table, and
    /// how densely it sat together: what tells a page from the light changing.
    pub paper_share: f32,
    pub paper_fill: f32,
    pub open_pages: Vec<String>,
    /// What those pages say, as the Pi read them for the preview — empty
    /// where nothing could read them.
    pub read_pages: Vec<crate::read::PageText>,
    /// Which folders that text looks like, before Paperless has said. Each is
    /// a name, with the bin marked.
    pub guess: Vec<GuessedFolder>,
    pub queue: Vec<Queued>,
    /// Newest first.
    pub events: Vec<Event>,
    pub settings: SettingsView,
    /// The last connection check, if one was asked for.
    pub check: Option<Event>,
    /// What just came of a button that does its work at once — learning the
    /// table — pressed here or on the display, while it is news.
    pub notice: Option<Event>,
    /// Counters, not times: the page reloads an image when its number changes.
    pub frame_at: u64,
    pub full_view_at: u64,
    /// Where setup found a page in the last picture of the whole view, as a
    /// crop — `None` when it found none.
    pub suggested_crop: Option<String>,
    /// The corners of the page found there, as the `corners` setting takes
    /// them.
    pub suggested_corners: Option<String>,
    /// Finishing was asked for and is waiting for things to be still.
    pub finishing: bool,
    /// The last frame's change from the empty table, and from the frame
    /// before, as fractions — what a person tuning the rig looks at.
    pub table_change: f32,
    pub movement: f32,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: "starting".into(),
            frames_still: 0,
            settle_frames: 0,
            has_baseline: false,
            collecting: false,
            scanning: true,
            can_undo_letter: false,
            can_refile: false,
            paper_share: 0.0,
            paper_fill: 0.0,
            open_pages: Vec::new(),
            read_pages: Vec::new(),
            guess: Vec::new(),
            queue: Vec::new(),
            events: Vec::new(),
            settings: SettingsView::default(),
            check: None,
            notice: None,
            frame_at: 0,
            full_view_at: 0,
            suggested_crop: None,
            suggested_corners: None,
            finishing: false,
            table_change: 0.0,
            movement: 0.0,
        }
    }
}

/// How many activity lines the page keeps.
const EVENTS: usize = 30;

pub struct Hub {
    /// Every frame is this size, cropped or not.
    pub frame_size: (u32, u32),
    status: Mutex<Status>,
    frame: Mutex<Option<Vec<u8>>>,
    full_view: Mutex<Option<Vec<u8>>>,
    commands: Mutex<VecDeque<Command>>,
}

impl Hub {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            frame_size: (width, height),
            status: Mutex::new(Status::default()),
            frame: Mutex::new(None),
            full_view: Mutex::new(None),
            commands: Mutex::new(VecDeque::new()),
        }
    }

    pub fn send(&self, command: Command) {
        self.commands.lock().unwrap().push_back(command);
    }

    pub fn take_commands(&self) -> Vec<Command> {
        self.commands.lock().unwrap().drain(..).collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn update(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap());
    }

    pub fn event(&self, at: u64, ok: bool, text: impl Into<String>) {
        let mut status = self.status.lock().unwrap();
        status.events.insert(
            0,
            Event {
                at,
                ok,
                text: text.into(),
            },
        );
        status.events.truncate(EVENTS);
    }

    pub fn set_frame(&self, frame: Vec<u8>) {
        *self.frame.lock().unwrap() = Some(frame);
        self.update(|status| status.frame_at += 1);
    }

    pub fn frame(&self) -> Option<Vec<u8>> {
        self.frame.lock().unwrap().clone()
    }

    /// A colour JPEG of the whole view, and where a page was found in it.
    pub fn set_full_view(
        &self,
        jpeg: Vec<u8>,
        suggested_crop: Option<String>,
        suggested_corners: Option<String>,
    ) {
        *self.full_view.lock().unwrap() = Some(jpeg);
        self.update(|status| {
            status.suggested_crop = suggested_crop;
            status.suggested_corners = suggested_corners;
            status.full_view_at += 1;
        });
    }

    pub fn full_view(&self) -> Option<Vec<u8>> {
        self.full_view.lock().unwrap().clone()
    }
}
