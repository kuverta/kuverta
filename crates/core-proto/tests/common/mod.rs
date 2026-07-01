//! Scaffolding shared by the integration-test binaries.
//!
//! Each file in `tests/` is its own crate, so anything used by more than one
//! of them has to live here rather than be copied into each.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory that removes itself, so a test store leaves nothing behind.
pub struct TempDir(PathBuf);

impl TempDir {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("kuverta-it-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
