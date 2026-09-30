//! Safe preview: an attachment drawn so that it cannot reach the system.
//!
//! A picture or a PDF from a stranger is the classic way in without a click:
//! FORCEDENTRY was a JBIG2 image inside a PDF, BLASTPASS a WebP, and both were
//! bugs in the system's own decoders, written in C and Objective-C, that a
//! window hands every image and every PDF to. So in safe preview nothing the
//! sender wrote reaches them. Four layers, each on its own enough to stop a
//! different mistake:
//!
//! 1. **Decoders written in Rust.** PDFs are drawn by hayro, which forbids
//!    unsafe code; pictures by the `image` crate's codecs; everything else —
//!    Word, Excel, a web page, a forwarded message — is read as text by
//!    [`crate::readable`]. A malformed file is a panic or an error there, not a
//!    write past the end of a buffer.
//! 2. **In a process of its own, locked down before it looks.** The worker is
//!    this program again, started with [`WORKER_FLAG`]. It reads the file from
//!    its standard input, then takes away what it could do with a foothold —
//!    on macOS by the system's sandbox in its pure-computation profile (no
//!    files, no network, no programs), everywhere on Unix by resource limits
//!    (no new files or sockets, no new processes, no core file, a CPU budget)
//!    — and only then parses. It fails closed: a worker that cannot lock
//!    itself down draws nothing.
//! 3. **Bounded, and killed if it is not done.** A wall-clock limit, input and
//!    output caps, a limit on how large a picture may claim to be, and pages
//!    drawn a few at a time.
//! 4. **Only pixels come back.** The worker answers with raw RGB and the size
//!    of each page. This side checks that the sizes add up and encodes the PNG
//!    itself, so the window's image decoder only ever sees a PNG kuverta wrote
//!    — even a worker that had been taken over could hand it nothing else.
//!
//! What it cannot cover is a file opened in another program; that program
//! reads it as it reads anything. The window marks such files as come from
//! elsewhere (quarantine on macOS, Mark of the Web on Windows) and, in safe
//! preview, asks twice.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::readable::Contents;

/// The flag that makes a kuverta program the preview worker.
pub const WORKER_FLAG: &str = "--preview-worker";

/// The most of a file the worker is sent.
const MOST_INPUT: usize = 64 * 1024 * 1024;
/// The most it may answer with: a few pages of RGB.
const MOST_OUTPUT: usize = 96 * 1024 * 1024;
/// How wide a page is drawn, and the most either side may be.
const PAGE_WIDTH: f32 = 1400.0;
const MOST_SIDE: u32 = 2400;
/// How many pages one request draws.
pub const MOST_PAGES_AT_ONCE: usize = 4;
/// How large a picture may say it is, and how much decoding it may take.
const MOST_PICTURE_SIDE: u32 = 20_000;
const MOST_PICTURE_ALLOC: u64 = 512 * 1024 * 1024;
/// CPU seconds the worker may use; the wall clock is the caller's.
#[cfg(unix)]
const CPU_SECONDS: u64 = 25;

/// How the worker is started.
#[derive(Debug, Clone)]
pub struct Worker {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Killed when it has not answered by then.
    pub timeout: Duration,
}

impl Worker {
    /// This program again, as the worker. Its `main` must hand
    /// [`WORKER_FLAG`] to [`worker_main`] before doing anything else.
    pub fn this_program() -> std::io::Result<Self> {
        Ok(Self {
            program: std::env::current_exe()?,
            args: vec![WORKER_FLAG.to_string()],
            timeout: Duration::from_secs(30),
        })
    }
}

static WORKER: OnceLock<Worker> = OnceLock::new();

/// Says how to start the worker, once, at startup. Without it, files the
/// assistant reads are read in this process — which is what the CLI does.
pub fn set_worker(worker: Worker) {
    let _ = WORKER.set(worker);
}

/// The worker, when one was set.
pub fn worker() -> Option<&'static Worker> {
    WORKER.get()
}

/// An attachment as the window shows it in safe preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SafePreview {
    /// Pages drawn as pictures: PNGs written here, base64.
    Pages {
        /// The first page drawn, from 0.
        first: usize,
        /// How many pages the file has.
        total: usize,
        pages: Vec<String>,
    },
    /// The text of a file that is read rather than drawn.
    Text { text: String },
    /// Nothing to show, and why.
    Unavailable { why: String },
}

// -- what goes between the two processes -------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Job {
    /// Draw pages, or read the text.
    Preview,
    /// What [`crate::readable::contents`] makes of it, for the assistant.
    Contents,
    /// Try what a locked-down worker must not be able to do, and say.
    Probe,
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    job: Job,
    name: String,
    content_type: String,
    first: usize,
    count: usize,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Reply {
    /// Blobs are RGB, one per page, each `width * height * 3` bytes.
    Pages {
        first: usize,
        total: usize,
        sizes: Vec<(u32, u32)>,
    },
    /// Blobs are the pages of a scan or a photograph, as they were, for a
    /// vision model.
    Scans,
    Text {
        text: String,
    },
    Unavailable {
        why: String,
    },
    Probe {
        /// What was tried, and whether it was stopped.
        tried: Vec<(String, bool)>,
    },
}

/// A frame: a JSON header and length-prefixed blobs, each length a u32.
fn frame<T: Serialize>(header: &T, blobs: &[Vec<u8>]) -> Vec<u8> {
    let header = serde_json::to_vec(header).expect("a header always serialises");
    let mut out = Vec::with_capacity(8 + header.len() + blobs.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&(blobs.len() as u32).to_le_bytes());
    for blob in blobs {
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(blob);
    }
    out
}

/// Takes a frame apart. Everything is checked against what is there: a
/// length that runs past the end is an error, not a read past it.
fn unframe<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<(T, Vec<&[u8]>), String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let end = at.checked_add(n).filter(|end| *end <= bytes.len());
        let end = end.ok_or("the answer ends early")?;
        let slice = &bytes[at..end];
        at = end;
        Ok(slice)
    };
    let length = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let header_len = length(take(4)?);
    let header: T = serde_json::from_slice(take(header_len)?)
        .map_err(|err| format!("the answer's header is not readable: {err}"))?;
    let count = length(take(4)?);
    if count > 64 {
        return Err("the answer has more parts than a request asks for".into());
    }
    let mut blobs = Vec::with_capacity(count);
    for _ in 0..count {
        let n = length(take(4)?);
        blobs.push(take(n)?);
    }
    if at != bytes.len() {
        return Err("the answer goes on past its end".into());
    }
    Ok((header, blobs))
}

// -- the worker's side ------------------------------------------------------------------

/// The worker: one request on standard input, one answer on standard output,
/// and exit. Never returns.
pub fn worker_main() -> ! {
    let code = match serve() {
        Ok(()) => 0,
        Err(err) => {
            // Nothing on stderr: the caller does not read it, and a message
            // there is one more thing a file could put words into.
            let _ = err;
            1
        }
    };
    std::process::exit(code)
}

fn serve() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(MOST_INPUT as u64 + 4096)
        .read_to_end(&mut input)
        .map_err(|err| err.to_string())?;

    // Before a byte of the file is looked at.
    lock_down()?;

    let (request, blobs) = unframe::<Request>(&input)?;
    let bytes = blobs.first().copied().unwrap_or_default();
    let (reply, out) = match request.job {
        Job::Probe => (Reply::Probe { tried: probe() }, Vec::new()),
        job => std::panic::catch_unwind(|| match job {
            Job::Contents => contents_reply(&request, bytes),
            _ => draw(&request, bytes),
        })
        .unwrap_or_else(|_| {
            (
                Reply::Unavailable {
                    why: "kuverta could not draw this file: it is damaged, or built to break a reader"
                        .into(),
                },
                Vec::new(),
            )
        }),
    };
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&frame(&reply, &out))
        .and_then(|()| stdout.flush())
        .map_err(|err| err.to_string())
}

/// What the file is, by its first bytes — not by what the sender called it.
fn is_pdf(request: &Request, bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
        || request.content_type == "application/pdf"
        || request.name.to_ascii_lowercase().ends_with(".pdf")
}

fn is_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"BM")
}

fn unavailable(why: impl Into<String>) -> (Reply, Vec<Vec<u8>>) {
    (Reply::Unavailable { why: why.into() }, Vec::new())
}

/// Pages drawn, or the text read: what the window shows.
fn draw(request: &Request, bytes: &[u8]) -> (Reply, Vec<Vec<u8>>) {
    if is_picture(bytes) {
        return match picture(bytes) {
            Ok((size, rgb)) => (
                Reply::Pages {
                    first: 0,
                    total: 1,
                    sizes: vec![size],
                },
                vec![rgb],
            ),
            Err(why) => unavailable(why),
        };
    }
    if is_pdf(request, bytes) {
        return pdf(request, bytes);
    }
    match crate::readable::contents(&request.name, &request.content_type, bytes) {
        Contents::Text(text) => (Reply::Text { text }, Vec::new()),
        Contents::Unreadable(why) => unavailable(why),
        // A picture kuverta does not decode, found another way.
        Contents::Pages(_) => unavailable("a picture in a format kuverta does not draw"),
    }
}

fn contents_reply(request: &Request, bytes: &[u8]) -> (Reply, Vec<Vec<u8>>) {
    match crate::readable::contents(&request.name, &request.content_type, bytes) {
        Contents::Text(text) => (Reply::Text { text }, Vec::new()),
        Contents::Unreadable(why) => unavailable(why),
        Contents::Pages(pages) => (
            Reply::Scans,
            pages
                .into_iter()
                .take(crate::readable::MOST_PAGES)
                .collect(),
        ),
    }
}

/// A picture decoded, made to fit, and put on white: RGB and its size.
fn picture(bytes: &[u8]) -> Result<((u32, u32), Vec<u8>), String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MOST_PICTURE_SIDE);
    limits.max_image_height = Some(MOST_PICTURE_SIDE);
    limits.max_alloc = Some(MOST_PICTURE_ALLOC);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|err| format!("the picture could not be read: {err}"))?;
    let (width, height) = (decoded.width(), decoded.height());
    let fit = (PAGE_WIDTH / width as f32)
        .min(MOST_SIDE as f32 / height as f32)
        .min(1.0);
    let decoded = if fit < 1.0 {
        decoded.resize(
            ((width as f32 * fit) as u32).max(1),
            ((height as f32 * fit) as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let rgba = decoded.to_rgba8();
    let size = (rgba.width(), rgba.height());
    Ok((size, on_white(rgba.as_raw())))
}

/// RGBA over a white page, as RGB.
fn on_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        let alpha = pixel[3] as u32;
        for &channel in &pixel[..3] {
            rgb.push(((channel as u32 * alpha + 255 * (255 - alpha)) / 255) as u8);
        }
    }
    rgb
}

fn pdf(request: &Request, bytes: &[u8]) -> (Reply, Vec<Vec<u8>>) {
    use hayro::hayro_syntax::Pdf;
    use std::sync::Arc;

    let data: hayro::hayro_syntax::PdfData = Arc::new(bytes.to_vec());
    let document = match Pdf::new(data) {
        Ok(document) => document,
        // Encrypted, or not a PDF after all: its text, if anything reads it.
        Err(_) => {
            return match crate::readable::contents(&request.name, "application/pdf", bytes) {
                Contents::Text(text) => (Reply::Text { text }, Vec::new()),
                _ => {
                    unavailable("a PDF kuverta cannot draw — protected with a password, or damaged")
                }
            };
        }
    };
    let pages = document.pages();
    let total = pages.len();
    if total == 0 {
        return unavailable("a PDF with no pages");
    }
    let first = request.first.min(total - 1);
    let count = request.count.clamp(1, MOST_PAGES_AT_ONCE);
    let settings = hayro::hayro_interpret::InterpreterSettings::default();
    let mut sizes = Vec::new();
    let mut blobs = Vec::new();
    for page in pages.iter().skip(first).take(count) {
        let (width, height) = page.render_dimensions();
        if !(width > 0.0 && height > 0.0) {
            continue;
        }
        let scale = (PAGE_WIDTH / width).min(MOST_SIDE as f32 / height).min(8.0);
        let pixmap = hayro::render(
            page,
            &settings,
            &hayro::RenderSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..Default::default()
            },
        );
        let (w, h) = (pixmap.width() as u32, pixmap.height() as u32);
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        // On an opaque white page every pixel is opaque, so premultiplied
        // is plain RGB.
        for pixel in pixmap.data() {
            rgb.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        sizes.push((w, h));
        blobs.push(rgb);
    }
    (
        Reply::Pages {
            first,
            total,
            sizes,
        },
        blobs,
    )
}

/// What a worker that has been taken over would try first, tried.
fn probe() -> Vec<(String, bool)> {
    let home = std::env::temp_dir().join(format!("kuverta-probe-{}", std::process::id()));
    let mut tried = vec![
        (
            "write a file".to_string(),
            std::fs::write(&home, b"x").is_err(),
        ),
        (
            "open a network socket".to_string(),
            std::net::UdpSocket::bind("127.0.0.1:0").is_err(),
        ),
        (
            "start a program".to_string(),
            Command::new(if cfg!(windows) { "cmd" } else { "/bin/echo" })
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_err(),
        ),
    ];
    if cfg!(unix) {
        tried.push((
            "read a file".to_string(),
            std::fs::read("/etc/hosts").is_err(),
        ));
    }
    let _ = std::fs::remove_file(&home);
    tried
}

// -- locking the worker down -----------------------------------------------------------

#[cfg(target_os = "macos")]
fn sandbox() -> Result<(), String> {
    use std::ffi::{c_char, c_int, CStr};

    // libsandbox, which every macOS program links through libSystem. The API
    // is marked deprecated, and is what Chromium's and WebKit's helpers used
    // before they had their own; pure computation allows nothing but
    // computing: no file opened, no socket, no program started. Descriptors
    // open before it — standard input and output — stay usable.
    extern "C" {
        // `const char kSBXProfilePureComputation[]` in sandbox.h: an array,
        // so the symbol is the first character, and its address the string.
        static kSBXProfilePureComputation: c_char;
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }
    const SANDBOX_NAMED: u64 = 0x0001;

    let mut error: *mut c_char = std::ptr::null_mut();
    // SAFETY: the profile is a static C string the library defines, passed
    // by address; `error` is written only on failure, read once as a C
    // string, and freed with the library's own function.
    let status = unsafe {
        sandbox_init(
            std::ptr::addr_of!(kSBXProfilePureComputation),
            SANDBOX_NAMED,
            &mut error,
        )
    };
    if status == 0 {
        return Ok(());
    }
    let why = if error.is_null() {
        "no reason given".to_string()
    } else {
        // SAFETY: as above.
        let why = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { sandbox_free_error(error) };
        why
    };
    Err(format!("the sandbox refused: {why}"))
}

#[cfg(unix)]
fn limit(resource: libc::c_int, value: u64) -> Result<(), String> {
    let wanted = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    // SAFETY: setrlimit reads the struct it is given and nothing else.
    if unsafe { libc::setrlimit(resource as _, &wanted) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "a resource limit ({resource}) could not be set: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Takes away what the worker does not need. Failing any of it is failing:
/// the file is then not looked at.
fn lock_down() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    sandbox()?;

    #[cfg(unix)]
    {
        // A crash leaves no core file with the attachment in it.
        limit(libc::RLIMIT_CORE as _, 0)?;
        // A file built to spin runs out of CPU, as well as out of time.
        let cpu = libc::rlimit {
            rlim_cur: CPU_SECONDS as libc::rlim_t,
            rlim_max: (CPU_SECONDS + 5) as libc::rlim_t,
        };
        // SAFETY: as in `limit`.
        if unsafe { libc::setrlimit(libc::RLIMIT_CPU as _, &cpu) } != 0 {
            return Err("the CPU limit could not be set".into());
        }
        // Nothing written to disk: a write to a file is a signal that ends it.
        limit(libc::RLIMIT_FSIZE as _, 0)?;
        // No new processes.
        limit(libc::RLIMIT_NPROC as _, 0)?;
        // No new descriptors: no file opened, no socket, no pipe. The three
        // it has stay open.
        limit(libc::RLIMIT_NOFILE as _, 0)?;
        // Memory, where the system enforces it (Linux; macOS ignores it and
        // the decoders' own limits stand in).
        #[cfg(target_os = "linux")]
        limit(libc::RLIMIT_AS as _, 2 * 1024 * 1024 * 1024)?;
    }
    Ok(())
}

// -- the caller's side -------------------------------------------------------------------

/// Sends one request to a fresh worker and reads its answer, within the
/// worker's time. Returns the raw answer.
fn exchange(worker: &Worker, request: &Request, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MOST_INPUT {
        return Err(format!(
            "the file is larger than {} MB, which safe preview does not draw",
            MOST_INPUT / 1024 / 1024
        ));
    }
    let input = frame(request, &[bytes.to_vec()]);
    let mut command = Command::new(&worker.program);
    command
        .args(&worker.args)
        // Nothing of this process's environment: no tokens, no paths.
        .env_clear()
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        // Windows will not start a process without it.
        command.env("SystemRoot", root);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("the preview worker did not start: {err}"))?;

    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || {
        // A worker that stops reading early closes the pipe; that is its
        // answer to give, not an error here.
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout)
            .take(MOST_OUTPUT as u64 + 1)
            .read_to_end(&mut out);
        out
    });

    let deadline = Instant::now() + worker.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                return Err(format!(
                    "drawing it took longer than {} seconds, so it was stopped",
                    worker.timeout.as_secs()
                ));
            }
            Err(err) => return Err(format!("the preview worker was lost: {err}")),
        }
    };
    let _ = writer.join();
    let out = reader
        .join()
        .map_err(|_| "the preview worker's answer was lost".to_string())?;
    if !status.success() {
        return Err(format!(
            "the preview worker stopped without an answer ({status}): the file may be built to break a reader"
        ));
    }
    if out.len() > MOST_OUTPUT {
        return Err("the preview worker answered with more than a preview".into());
    }
    Ok(out)
}

/// Draws an attachment, or reads its text, in the worker.
pub fn preview(
    worker: &Worker,
    name: &str,
    content_type: &str,
    bytes: &[u8],
    first: usize,
    count: usize,
) -> Result<SafePreview, String> {
    let request = Request {
        job: Job::Preview,
        name: name.to_string(),
        content_type: content_type.to_string(),
        first,
        count: count.clamp(1, MOST_PAGES_AT_ONCE),
    };
    let out = exchange(worker, &request, bytes)?;
    let (reply, blobs) = unframe::<Reply>(&out)?;
    match reply {
        Reply::Pages {
            first,
            total,
            sizes,
        } => {
            if sizes.len() != blobs.len() || sizes.len() > MOST_PAGES_AT_ONCE {
                return Err("the preview worker's pages do not add up".into());
            }
            let mut pages = Vec::with_capacity(blobs.len());
            for ((width, height), rgb) in sizes.into_iter().zip(blobs) {
                pages.push(png_of(width, height, rgb)?);
            }
            Ok(SafePreview::Pages {
                first,
                total,
                pages,
            })
        }
        Reply::Text { text } => Ok(SafePreview::Text { text }),
        Reply::Unavailable { why } => Ok(SafePreview::Unavailable { why }),
        Reply::Scans | Reply::Probe { .. } => {
            Err("the preview worker answered another question".into())
        }
    }
}

/// A PNG, base64, written here from pixels the worker drew — after checking
/// that they are as many as it says.
fn png_of(width: u32, height: u32, rgb: &[u8]) -> Result<String, String> {
    use base64::Engine as _;
    use image::ImageEncoder as _;

    if width == 0 || height == 0 || width > MOST_SIDE + 1 || height > MOST_SIDE + 1 {
        return Err(format!(
            "the preview worker drew a page of {width}×{height}"
        ));
    }
    if rgb.len() != (width as usize) * (height as usize) * 3 {
        return Err("the preview worker's page is not the size it says".into());
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|err| err.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// What [`crate::readable::contents`] makes of a file, read in the worker.
/// A worker that fails is a file that could not be read.
pub fn contents(worker: &Worker, name: &str, content_type: &str, bytes: &[u8]) -> Contents {
    let request = Request {
        job: Job::Contents,
        name: name.to_string(),
        content_type: content_type.to_string(),
        first: 0,
        count: 0,
    };
    let answer = exchange(worker, &request, bytes).and_then(|out| {
        let (reply, blobs) = unframe::<Reply>(&out)?;
        Ok(match reply {
            Reply::Text { text } => Contents::Text(text),
            Reply::Unavailable { why } => Contents::Unreadable(why),
            Reply::Scans => Contents::Pages(blobs.into_iter().map(<[u8]>::to_vec).collect()),
            _ => return Err("the preview worker answered another question".to_string()),
        })
    });
    answer.unwrap_or_else(Contents::Unreadable)
}

/// Asks a worker to try what a locked-down one must not manage: write a
/// file, open a socket, start a program, read a file. Each with whether it
/// was stopped.
pub fn probe_worker(worker: &Worker) -> Result<Vec<(String, bool)>, String> {
    let request = Request {
        job: Job::Probe,
        name: String::new(),
        content_type: String::new(),
        first: 0,
        count: 0,
    };
    let out = exchange(worker, &request, &[])?;
    match unframe::<Reply>(&out)?.0 {
        Reply::Probe { tried } => Ok(tried),
        _ => Err("the preview worker answered another question".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str, content_type: &str) -> Request {
        Request {
            job: Job::Preview,
            name: name.into(),
            content_type: content_type.into(),
            first: 0,
            count: 2,
        }
    }

    fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        use image::ImageEncoder as _;
        let pixels: Vec<u8> = rgba
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect();
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn a_picture_is_drawn_on_white_whatever_it_is_called() {
        // Half-transparent red, named as a PDF: the bytes decide.
        let (reply, blobs) = draw(
            &request("x.pdf", "application/pdf"),
            &png(3, 2, [255, 0, 0, 128]),
        );
        assert_eq!(
            reply,
            Reply::Pages {
                first: 0,
                total: 1,
                sizes: vec![(3, 2)]
            }
        );
        assert_eq!(blobs[0].len(), 3 * 2 * 3);
        assert_eq!(&blobs[0][..3], &[255, 127, 127]);
    }

    #[test]
    fn a_picture_that_claims_to_be_enormous_is_refused_before_it_is_decoded() {
        // A PNG header saying 100000 × 100000: decoding it would ask for
        // forty gigabytes.
        let mut claim = png(1, 1, [0, 0, 0, 255]);
        claim[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        claim[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let (reply, blobs) = draw(&request("big.png", "image/png"), &claim);
        assert!(matches!(reply, Reply::Unavailable { .. }), "{reply:?}");
        assert!(blobs.is_empty());
    }

    #[test]
    fn a_pdf_is_drawn_page_by_page_with_ink_on_it() {
        let pdf = crate::readable::tests_support::pdf_saying("Rechnung 4711");
        let (reply, blobs) = draw(&request("r.pdf", "application/pdf"), &pdf);
        let Reply::Pages { total, sizes, .. } = reply else {
            panic!("expected pages, got {reply:?}")
        };
        assert_eq!(total, 1);
        let (w, h) = sizes[0];
        assert!(w as f32 <= PAGE_WIDTH + 1.0 && h <= MOST_SIDE, "{w}×{h}");
        assert_eq!(blobs[0].len(), (w * h * 3) as usize);
        // White paper, and somewhere on it, dark text.
        assert_eq!(&blobs[0][..3], &[255, 255, 255]);
        assert!(blobs[0].iter().any(|&c| c < 80), "no ink drawn");
    }

    #[test]
    fn a_damaged_pdf_says_so() {
        let (reply, _) = draw(
            &request("x.pdf", "application/pdf"),
            b"%PDF-1.4 and nothing",
        );
        assert!(matches!(reply, Reply::Unavailable { .. }), "{reply:?}");
    }

    #[test]
    fn a_web_page_is_read_as_text_and_never_drawn() {
        let (reply, blobs) = draw(
            &request("login.html", "text/html"),
            b"<form><p>Passwort eingeben</p><script>steal()</script></form>",
        );
        match reply {
            Reply::Text { text } => {
                assert!(text.contains("Passwort eingeben"), "{text}");
                assert!(!text.contains("steal"), "{text}");
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert!(blobs.is_empty());
    }

    #[test]
    fn frames_that_lie_about_their_lengths_are_refused() {
        let good = frame(&request("a", "b"), &[vec![1, 2, 3]]);
        assert!(unframe::<Request>(&good).is_ok());
        // Cut short, and too long.
        assert!(unframe::<Request>(&good[..good.len() - 1]).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(unframe::<Request>(&long).is_err());
        // A blob length far past the end.
        let mut lying = good.clone();
        let at = good.len() - 3 - 4;
        lying[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(unframe::<Request>(&lying).is_err());
        assert!(unframe::<Request>(&[]).is_err());
    }

    #[test]
    fn a_page_is_only_encoded_when_its_pixels_add_up() {
        assert!(png_of(2, 2, &[0; 12]).is_ok());
        assert!(png_of(2, 2, &[0; 11]).is_err());
        assert!(png_of(0, 2, &[]).is_err());
        assert!(png_of(MOST_SIDE * 4, 1, &vec![0; (MOST_SIDE * 12) as usize]).is_err());
    }
}
