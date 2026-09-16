//! Raw message bodies on disk.
//!
//! Message bodies stay out of SQLite: they are large, immutable, and never
//! queried by content (search goes through FTS5). Keeping them as files means
//! the database stays small enough to be cheap to back up and fast to open.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub struct Blobs {
    root: PathBuf,
}

impl Blobs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes a raw message and returns its path relative to the blob root,
    /// which is what belongs in `message.body_path`.
    ///
    /// Content is keyed by the message's dedup key, so re-syncing the same
    /// message overwrites in place rather than accumulating copies.
    pub fn put(&self, account_id: i64, dedup_key: &str, raw: &[u8]) -> io::Result<String> {
        let digest = Sha256::digest(dedup_key.as_bytes());
        let mut hex = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }

        // Two-level fan-out: a flat directory with 100k files is slow to list
        // on every filesystem worth supporting.
        let relative = format!("{account_id}/{}/{hex}.eml", &hex[0..2]);
        let absolute = self.root.join(&relative);

        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&absolute, raw)?;

        Ok(relative)
    }

    pub fn get(&self, relative: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        fs::read(self.root.join(relative))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_is_stable_for_one_key() {
        let dir = std::env::temp_dir().join(format!("kuverta-blobs-{}", std::process::id()));
        let blobs = Blobs::new(&dir);

        let first = blobs.put(1, "mid:abc@example.com", b"raw message").unwrap();
        let second = blobs.put(1, "mid:abc@example.com", b"raw message").unwrap();

        assert_eq!(first, second, "same key must map to the same path");
        assert_eq!(blobs.get(&first).unwrap(), b"raw message");

        // Different accounts must not share a blob even for identical keys.
        let other = blobs.put(2, "mid:abc@example.com", b"raw message").unwrap();
        assert_ne!(first, other);

        let _ = fs::remove_dir_all(&dir);
    }
}
