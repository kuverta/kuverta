//! Reading a page on the Pi itself, for the preview — not for the archive.
//!
//! Paperless is what reads a letter properly: it has the language data, the
//! deskewing and the tags, and what it decides is what the letter is filed
//! under. But it only gets the letter once the letter is finished, and by then
//! whoever is standing at the table has already put the paper down somewhere.
//!
//! So the page is read here too, badly and at once: `tesseract` on the Pi 4
//! takes a few seconds a page, which is slower than Paperless and quite fast
//! enough for a preview that says *this looks like Taxes* while the next page
//! is going under the camera. Nothing is filed on it; when the letter is sent,
//! Paperless's answer replaces the guess.
//!
//! Reading happens on a thread of its own — the loop must keep watching the
//! table — and the text comes back through a channel, one message a page.
//! Without `tesseract` installed there is simply no preview text, and
//! everything else works as before.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

/// A page that has been read, as the preview shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PageText {
    /// The page's file name, as the open letter lists it.
    pub name: String,
    pub text: String,
}

/// How long a page may take before it is given up on: a Pi 4 reads an A4 page
/// in a few seconds, and a minute means something is wrong rather than slow.
const LIMIT_SECS: u64 = 60;

pub struct Reader {
    program: String,
    languages: String,
    sent: Sender<PageText>,
    read: Receiver<PageText>,
}

impl Reader {
    /// `program` is `tesseract` unless something else is wanted; `languages`
    /// is its `-l`, such as `deu+eng`.
    pub fn new(program: &str, languages: &str) -> Self {
        let (sent, read) = mpsc::channel();
        Self {
            program: program.to_string(),
            languages: languages.to_string(),
            sent,
            read,
        }
    }

    /// Whether the program is there at all. Said once at startup, so that a
    /// missing tesseract is a line in the log and not a failure a page.
    pub fn available(&self) -> bool {
        Command::new(&self.program)
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success())
    }

    /// Reads a page in the background. The text arrives at [`Self::texts`].
    pub fn start(&self, page: &Path) {
        self.start_as(page, Layout::default());
    }

    /// The same, cutting the page up another way — for a page the usual
    /// reading mangles.
    pub fn start_as(&self, page: &Path, how: Layout) {
        let Some(name) = page
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };
        let (program, languages, sent) = (
            self.program.clone(),
            self.languages.clone(),
            self.sent.clone(),
        );
        let path = page.to_path_buf();
        std::thread::spawn(move || {
            let started = Instant::now();
            match read_page(&program, &languages, &path, how) {
                Ok(text) => {
                    tracing::info!(
                        page = %name,
                        ms = started.elapsed().as_millis(),
                        chars = text.chars().count(),
                        "read the page for the preview"
                    );
                    let _ = sent.send(PageText { name, text });
                }
                Err(err) => {
                    tracing::warn!(%err, page = %name, "could not read the page for the preview");
                    let _ = sent.send(PageText {
                        name,
                        text: String::new(),
                    });
                }
            }
        });
    }

    /// Whatever has been read since this was last asked.
    pub fn texts(&self) -> Vec<PageText> {
        self.read.try_iter().collect()
    }
}

/// How the page is cut up before the words are read.
///
/// Measured on the rig's own photographs rather than chosen: a letter read as
/// [`Layout::Column`] gave fifteen of the words a German reader recognises
/// against thirteen for tesseract's default, with a quarter less of the
/// punctuation-soup that marks a misread line; on a second page, nine against
/// eight. [`Layout::Block`] finds many more "words" and no more real ones —
/// it is reading the noise between the columns.
///
/// [`Layout::Block`] is still worth having, because a page the other two
/// mangle is usually one whose columns they have run together, and it is what
/// **Read it again** tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// A column of text whose sizes vary: a letter, in other words.
    #[default]
    Column,
    /// One uniform block, columns and all.
    Block,
}

impl Layout {
    fn psm(self) -> &'static str {
        match self {
            Self::Column => "4",
            Self::Block => "6",
        }
    }

    /// The other one, for reading a page a second time.
    pub fn other(self) -> Self {
        match self {
            Self::Column => Self::Block,
            Self::Block => Self::Column,
        }
    }
}

/// One page through the OCR, as text.
///
/// Given a while and then killed: a reader that never comes back would hold a
/// core of the four for as long as scannerd runs, and the loop still has a
/// table to watch.
fn read_page(program: &str, languages: &str, page: &PathBuf, how: Layout) -> Result<String> {
    let mut child = Command::new(program)
        .arg(page)
        // To standard output rather than a file beside the page: the spool is
        // the letter, and nothing else belongs in it.
        .arg("stdout")
        .args(["-l", languages])
        .args(["--psm", how.psm()])
        // Three of the Pi's four cores: the fourth keeps watching the table.
        .env("OMP_THREAD_LIMIT", "3")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("could not run {program}"))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(LIMIT_SECS) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{program} took longer than {LIMIT_SECS} seconds");
            }
            Err(err) => bail!("could not wait for {program}: {err}"),
        }
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("could not read what {program} said"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(tidy(&String::from_utf8_lossy(&output.stdout)))
}

/// What the OCR gives back, with the empty lines it leaves between paragraphs
/// squeezed and the line ends tidied: the preview shows it as it is.
fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if matches!(lines.last(), Some(last) if last.is_empty()) {
                continue;
            }
            lines.push("");
        } else {
            lines.push(line);
        }
    }
    while matches!(lines.last(), Some(last) if last.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}
