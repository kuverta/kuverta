//! What fuckmail keeps about post that Paperless does not: read or not, and
//! a transcript of a scan whose OCR is no use.

use std::path::PathBuf;

use core_store::Store;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fuckmail-paper-state-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> Store {
        Store::open(self.0.join("fuckmail.db")).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PAPERLESS: &str = "http://localhost:8000";

#[test]
fn a_letter_is_unread_until_it_is_read_here() {
    let dir = TempDir::new("read");
    let store = dir.store();
    assert!(store.paper_read_ids(PAPERLESS).unwrap().is_empty());

    store.set_paper_read(PAPERLESS, 6, true).unwrap();
    // Twice is the same as once: opening a letter again is not an error.
    store.set_paper_read(PAPERLESS, 6, true).unwrap();
    store.set_paper_read(PAPERLESS, 4, true).unwrap();
    assert_eq!(store.paper_read_ids(PAPERLESS).unwrap(), vec![4, 6]);

    store.set_paper_read(PAPERLESS, 6, false).unwrap();
    assert_eq!(store.paper_read_ids(PAPERLESS).unwrap(), vec![4]);
}

#[test]
fn read_state_belongs_to_one_instance_whatever_its_url_ends_with() {
    let dir = TempDir::new("instance");
    let store = dir.store();
    store
        .set_paper_read("http://localhost:8000/", 1, true)
        .unwrap();

    assert_eq!(store.paper_read_ids(PAPERLESS).unwrap(), vec![1]);
    assert!(store
        .paper_read_ids("http://paperless.local:8000")
        .unwrap()
        .is_empty());
}

#[test]
fn a_transcript_is_kept_and_a_new_one_replaces_it() {
    let dir = TempDir::new("transcript");
    let store = dir.store();
    store
        .save_paper_transcript(PAPERLESS, 6, "qwen2.5vl:3b", "first reading")
        .unwrap();
    store
        .save_paper_transcript(PAPERLESS, 5, "qwen2.5vl:3b", "another letter")
        .unwrap();
    store
        .save_paper_transcript(PAPERLESS, 6, "qwen2.5vl:7b", "better reading")
        .unwrap();

    assert_eq!(
        store.paper_transcripts(PAPERLESS).unwrap(),
        vec![
            (5, "qwen2.5vl:3b".to_string(), "another letter".to_string()),
            (6, "qwen2.5vl:7b".to_string(), "better reading".to_string()),
        ]
    );
    assert!(store
        .paper_transcripts("http://elsewhere:8000")
        .unwrap()
        .is_empty());
}

#[test]
fn read_state_and_transcripts_survive_reopening_the_store() {
    let dir = TempDir::new("reopen");
    {
        let store = dir.store();
        store.set_paper_read(PAPERLESS, 2, true).unwrap();
        store
            .save_paper_transcript(PAPERLESS, 2, "m", "text")
            .unwrap();
    }
    let store = dir.store();
    assert_eq!(store.paper_read_ids(PAPERLESS).unwrap(), vec![2]);
    assert_eq!(store.paper_transcripts(PAPERLESS).unwrap().len(), 1);
}
