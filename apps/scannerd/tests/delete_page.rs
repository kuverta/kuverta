//! Deleting a page photographed by mistake, before its letter is sent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scannerd::hub::{Command, Hub};
use scannerd::spool::{is_page_name, Spool};
use scannerd::web::{serve, Web};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scannerd-delete-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn page(spool: &Spool, bytes: &[u8]) -> String {
    let (partial, ready) = spool.reserve_page().unwrap();
    std::fs::write(&partial, bytes).unwrap();
    spool.commit(&partial, &ready).unwrap();
    ready.file_name().unwrap().to_string_lossy().into_owned()
}

fn names(spool: &Spool) -> Vec<String> {
    spool
        .open_pages()
        .unwrap()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_deleted_page_leaves_the_letter_and_the_others_keep_their_order() {
    let dir = TempDir::new("spool");
    let spool = Spool::open(&dir.0).unwrap();
    let first = page(&spool, b"first");
    let mistake = page(&spool, b"a hand");
    let last = page(&spool, b"last");

    assert!(spool.delete_page(&mistake).unwrap());

    assert_eq!(names(&spool), vec![first, last]);
    // Deleted twice — two taps, or a letter sent in between — is not an error.
    assert!(!spool.delete_page(&mistake).unwrap());
}

#[test]
fn only_a_pages_name_can_be_deleted() {
    let dir = TempDir::new("names");
    let spool = Spool::open(dir.0.join("spool")).unwrap();
    std::fs::write(dir.0.join("spool/settings.toml"), "token = \"secret\"").unwrap();

    for name in [
        "../settings.toml",
        "settings.toml",
        "..jpg",
        ".jpg",
        "open/x.jpg",
        "",
    ] {
        assert!(!is_page_name(name), "{name:?}");
        assert!(spool.delete_page(name).is_err(), "{name:?}");
    }
    assert!(dir.0.join("spool/settings.toml").exists());
    assert!(is_page_name("1789489070-998406103.jpg"));
}

async fn start(dir: &Path) -> (String, Arc<Hub>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let hub = Arc::new(Hub::new(320, 240));
    let web = Arc::new(Web::new(hub.clone(), dir.to_path_buf(), None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, web));
    (base, hub)
}

#[tokio::test]
async fn the_page_asks_the_loop_to_delete_a_page_by_name() {
    let dir = TempDir::new("web");
    let (base, hub) = start(&dir.0).await;
    let client = reqwest::Client::new();
    let url = format!("{base}/api/delete-page");
    let send = |body: &'static str, header: bool| {
        let request = client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body);
        if header {
            request.header("X-Scannerd", "1").send()
        } else {
            request.send()
        }
    };

    let good = r#"{"name":"1789489070-998406103.jpg"}"#;
    assert_eq!(send(good, false).await.unwrap().status(), 403);
    assert_eq!(
        send(r#"{"name":"../settings.toml"}"#, true)
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(send("{}", true).await.unwrap().status(), 400);
    assert!(hub.take_commands().is_empty());

    assert!(send(good, true).await.unwrap().status().is_success());
    assert_eq!(
        hub.take_commands(),
        vec![Command::DeletePage("1789489070-998406103.jpg".into())]
    );
}
