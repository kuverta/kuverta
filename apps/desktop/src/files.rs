//! Attachments leaving the app: saved to Downloads, or handed to the program
//! the system opens that kind of file with.
//!
//! The name is the sender's, already made safe by the core; here it only has
//! to not overwrite anything. A copy opened is written to a folder of its own
//! under the system's temporary directory, read-only — a change made in the
//! viewer would otherwise look saved and be lost — and the system cleans it up
//! as it does the rest of that directory.
//!
//! Every file written here is marked as come from elsewhere, the way a
//! browser marks a download: the quarantine attribute on macOS, the Mark of
//! the Web on Windows. That is what makes Gatekeeper look at a program before
//! it runs and Office open a document in Protected View — the system's own
//! checks, for a file that leaves kuverta's safe preview for another program.
//! A file that cannot be marked is not left behind unmarked.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The person's Downloads folder, when there is one.
pub fn downloads() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|dir| dir.is_dir())
}

/// Writes `bytes` into `dir` as `name`, or `name (2)`, `name (3)`… when that
/// is taken. Never overwrites: the file is created only if it is new.
pub fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    for n in 1..1000 {
        let candidate = if n == 1 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(bytes)
                    .map_err(|err| format!("cannot write {}: {err}", candidate.display()))?;
                drop(file);
                if let Err(err) = mark_from_elsewhere(&candidate) {
                    let _ = std::fs::remove_file(&candidate);
                    return Err(format!(
                        "cannot mark {} as downloaded, so it was not kept: {err}",
                        candidate.display()
                    ));
                }
                return Ok(candidate);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("cannot write {}: {err}", candidate.display())),
        }
    }
    Err(format!(
        "{name} already exists too many times in {}",
        dir.display()
    ))
}

/// Marks a file as downloaded from somewhere else, for the system's checks.
#[cfg(target_os = "macos")]
pub fn mark_from_elsewhere(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;

    // flags;time;agent — 0081 is "quarantined, not yet opened", the same as
    // a browser writes. The time is hexadecimal seconds.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let value = format!("0081;{now:08x};kuverta;");
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let name = c"com.apple.quarantine";
    // SAFETY: both strings are NUL-terminated and outlive the call, and the
    // value is passed with its length.
    let status = unsafe {
        libc::setxattr(
            path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// Marks a file as downloaded from somewhere else, for the system's checks.
#[cfg(windows)]
pub fn mark_from_elsewhere(path: &Path) -> Result<(), String> {
    // Zone 3 is the Internet zone: the Mark of the Web, in the file's own
    // alternate data stream.
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n").map_err(|e| e.to_string())
}

/// Linux has no mark a desktop honours; nothing to do.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn mark_from_elsewhere(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Writes a read-only copy to open, in a folder of its own, and returns it.
pub fn copy_to_open(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir()
        .join("kuverta-attachments")
        .join(format!("{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let path = save_new(&dir, name, bytes)?;
    let mut permissions = std::fs::metadata(&path)
        .map_err(|err| err.to_string())?
        .permissions();
    permissions.set_readonly(true);
    let _ = std::fs::set_permissions(&path, permissions);
    Ok(path)
}

/// Opens a file in the program the system has for its kind.
pub fn open(path: &Path) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(path).status()
    } else {
        std::process::Command::new("xdg-open").arg(path).status()
    };
    match status {
        Ok(_) => Ok(()),
        Err(err) => Err(format!("could not open {}: {err}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("kuverta-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = save_new(&dir, "esim.png", b"one").unwrap();
        let second = save_new(&dir, "esim.png", b"two").unwrap();
        let bare = save_new(&dir, "README", b"x").unwrap();
        let bare2 = save_new(&dir, "README", b"y").unwrap();
        assert_eq!(first.file_name().unwrap(), "esim.png");
        assert_eq!(second.file_name().unwrap(), "esim (2).png");
        assert_eq!(bare2.file_name().unwrap(), "README (2)");
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&bare).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_saved_attachment_is_marked_as_downloaded() {
        let dir = std::env::temp_dir().join(format!("kuverta-mark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = save_new(&dir, "Rechnung.pdf", b"%PDF").unwrap();
        let out = std::process::Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&path)
            .output()
            .unwrap();
        let mark = String::from_utf8_lossy(&out.stdout);
        assert!(
            mark.starts_with("0081;") && mark.contains(";kuverta;"),
            "{mark:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
