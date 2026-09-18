//! The log: to the terminal as before, and to a file a person can send.
//!
//! A bug report without a log is a description of a symptom. So everything
//! the app logs also goes to `<data dir>/logs/kuverta.log`, and settings can
//! turn on **detailed logging** — debug level for kuverta's own crates, live,
//! without a restart — and **export** the log to Downloads to attach to a
//! report.
//!
//! Only kuverta's crates go to debug. The network libraries underneath log
//! protocol traffic at their debug levels, and a log that might hold an IMAP
//! `LOGIN` line is not one anybody should be asked to send. Nothing here logs
//! a password or a token; addresses and subjects do appear, and at debug level
//! so does a model's answer about a message, which can quote from it. The
//! export says so.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Registry};

/// Past this, the log is started afresh at the next launch, keeping one
/// previous file. A log is for the recent past.
const ROTATE_AT: u64 = 4 * 1024 * 1024;

const DETAILED: &str = "info,core_rpc=debug,core_proto=debug,core_store=debug,core_smtp=debug,\
core_accounts=debug,core_paper=debug,core_ai=debug,core_pgp=debug,kuverta_desktop=debug";

pub struct Logs {
    dir: PathBuf,
    filter: reload::Handle<EnvFilter, Registry>,
}

/// What settings shows about the log.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogStatus {
    /// The running version, which settings shows beside the log.
    pub version: &'static str,
    pub path: String,
    pub detailed: bool,
    pub size_bytes: u64,
}

impl Logs {
    pub fn init(data_dir: &Path) -> Self {
        let dir = data_dir.join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("kuverta.log");
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > ROTATE_AT) {
            let _ = std::fs::rename(&path, dir.join("kuverta.1.log"));
        }

        let detailed = dir.join("detailed").exists();
        let initial = if detailed {
            EnvFilter::new(DETAILED)
        } else {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        };
        let (filter, handle) = reload::Layer::new(initial);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        let registry = tracing_subscriber::registry().with(filter).with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        );
        match file {
            Some(file) => registry
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_target(true)
                        .with_writer(Mutex::new(file)),
                )
                .init(),
            None => registry.init(),
        }

        Self {
            dir,
            filter: handle,
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("kuverta.log")
    }

    pub fn status(&self) -> LogStatus {
        LogStatus {
            version: env!("CARGO_PKG_VERSION"),
            path: self.path().display().to_string(),
            detailed: self.dir.join("detailed").exists(),
            size_bytes: std::fs::metadata(self.path()).map(|m| m.len()).unwrap_or(0),
        }
    }

    /// Turns detailed logging on or off, now and for later launches.
    pub fn set_detailed(&self, on: bool) -> Result<(), String> {
        let marker = self.dir.join("detailed");
        if on {
            File::create(&marker).map_err(|err| err.to_string())?;
        } else if marker.exists() {
            std::fs::remove_file(&marker).map_err(|err| err.to_string())?;
        }
        let filter = if on {
            EnvFilter::new(DETAILED)
        } else {
            EnvFilter::new("info")
        };
        self.filter
            .reload(filter)
            .map_err(|err| format!("could not change the log level: {err}"))?;
        tracing::info!(detailed = on, "log level changed");
        Ok(())
    }

    /// The last `lines` lines of the log.
    pub fn tail(&self, lines: usize) -> String {
        let Ok(mut file) = File::open(self.path()) else {
            return String::new();
        };
        // The end of the file is what is wanted; reading a few hundred
        // kilobytes of it is enough for any number of lines a window shows.
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let start = len.saturating_sub(256 * 1024);
        let _ = file.seek(SeekFrom::Start(start));
        let mut text = String::new();
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_ok() {
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    /// Writes the log, and the one before it, to a file in Downloads with a
    /// header saying what produced it, and returns where.
    pub fn export(&self, data_dir: &Path) -> Result<PathBuf, String> {
        let downloads = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join("Downloads"))
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| self.dir.clone());
        let stamp = chrono_stamp();
        let target = downloads.join(format!("kuverta-log-{stamp}.txt"));

        let mut out = File::create(&target)
            .map_err(|err| format!("cannot write {}: {err}", target.display()))?;
        let header = format!(
            "kuverta {version}\n{os} {arch}\ninstance: {instance}\ndata directory: {dir}\n\
             detailed logging: {detailed}\nexported: {stamp}\n\n\
             This log holds addresses and subjects of mail kuverta worked on, and never \
             passwords or tokens. With detailed logging on it can also hold what a model \
             answered about a message, which may quote from it.\n\n",
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            instance = core_accounts::instance().unwrap_or_else(|| "installed".into()),
            dir = data_dir.display(),
            detailed = self.dir.join("detailed").exists(),
        );
        out.write_all(header.as_bytes())
            .map_err(|err| err.to_string())?;
        for name in ["kuverta.1.log", "kuverta.log"] {
            if let Ok(bytes) = std::fs::read(self.dir.join(name)) {
                let _ = writeln!(out, "===== {name} =====");
                out.write_all(&bytes).map_err(|err| err.to_string())?;
                let _ = writeln!(out);
            }
        }
        tracing::info!(path = %target.display(), "exported the log");
        Ok(target)
    }
}

/// `20260918-143012`, in UTC, without a date library for one filename.
fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Shows a file in the system's file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .status()
    } else {
        std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .status()
    };
    status.map(|_| ()).map_err(|err| err.to_string())
}
