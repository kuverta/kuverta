//! The setup page, over real HTTP.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scannerd::hub::{Command, Hub};
use scannerd::web::{bmp, serve, Web};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("scannerd-web-{}-{name}", std::process::id()));
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

async fn start(dir: &Path, password: Option<&str>) -> (String, Arc<Hub>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let hub = Arc::new(Hub::new(320, 240));
    let web = Arc::new(Web::new(
        hub.clone(),
        dir.to_path_buf(),
        password.map(str::to_string),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, web));
    (base, hub)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn the_page_and_the_status_are_served() {
    let dir = TempDir::new("page");
    let (base, _hub) = start(&dir.0, None).await;

    let page = client().get(&base).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(page.text().await.unwrap().contains("Finish letter"));

    let status: serde_json::Value = serde_json::from_str(
        &client()
            .get(format!("{base}/api/status"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status["state"], "starting");
    // The token itself never appears, only whether there is one.
    assert!(status["settings"].get("token").is_none());
    assert_eq!(status["settings"]["token_set"], false);
}

#[tokio::test]
async fn the_latest_frame_is_a_bitmap_a_browser_can_show() {
    let dir = TempDir::new("frame");
    let (base, hub) = start(&dir.0, None).await;
    let url = format!("{base}/frame.bmp");

    assert_eq!(
        client().get(&url).send().await.unwrap().status(),
        404,
        "no frame yet"
    );

    hub.set_frame((0..320 * 240).map(|i| (i / 320) as u8).collect());
    let reply = client().get(&url).send().await.unwrap();
    assert_eq!(reply.headers()["content-type"], "image/bmp");
    let bytes = reply.bytes().await.unwrap();

    assert!(bytes.starts_with(b"BM"));
    assert_eq!(bytes.len(), 14 + 40 + 1024 + 320 * 240);
    assert_eq!(
        u32::from_le_bytes(bytes[2..6].try_into().unwrap()) as usize,
        bytes.len()
    );
    // Bottom-up: the file's first row is the frame's last, row 239.
    assert_eq!(bytes[1078], 239);
    assert_eq!(bytes[bytes.len() - 1], 0);
}

#[test]
fn a_bitmap_row_is_padded_to_four_bytes() {
    let luma = [10, 20, 30, 40, 50, 60];
    let bytes = bmp(&luma, 3, 2).unwrap();
    assert_eq!(bytes.len(), 1078 + 2 * 4);
    assert_eq!(&bytes[1078..], &[40, 50, 60, 0, 10, 20, 30, 0]);
    assert!(bmp(&luma, 4, 2).is_none(), "too few pixels for the size");
}

#[tokio::test]
async fn a_button_needs_the_header_another_site_cannot_send() {
    let dir = TempDir::new("header");
    let (base, hub) = start(&dir.0, None).await;
    let url = format!("{base}/api/finish");

    let refused = client().post(&url).send().await.unwrap();
    assert_eq!(refused.status(), 403);
    assert!(hub.take_commands().is_empty());

    let accepted = client()
        .post(&url)
        .header("X-Scannerd", "1")
        .send()
        .await
        .unwrap();
    assert!(accepted.status().is_success());
    assert_eq!(hub.take_commands(), vec![Command::FinishLetter]);
}

#[tokio::test]
async fn with_a_password_nothing_is_served_without_it() {
    let dir = TempDir::new("password");
    let (base, _hub) = start(&dir.0, Some("letterbox")).await;

    let bare = client().get(&base).send().await.unwrap();
    assert_eq!(bare.status(), 401);
    assert!(bare.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .starts_with("Basic"));

    for (password, expected) in [("wrong", 401), ("letterbo", 401), ("letterbox", 200)] {
        let reply = client()
            .get(format!("{base}/api/status"))
            .basic_auth("anyone", Some(password))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), expected, "password {password:?}");
    }
}

#[tokio::test]
async fn settings_are_checked_before_the_loop_sees_them() {
    let dir = TempDir::new("settings");
    let (base, hub) = start(&dir.0, None).await;
    let url = format!("{base}/api/settings");
    let send = |body: &'static str| {
        client()
            .post(&url)
            .header("X-Scannerd", "1")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
    };

    let off_the_edge = send(r#"{"roi":"0.5,0.5,0.9,0.9"}"#).await.unwrap();
    assert_eq!(off_the_edge.status(), 400);
    assert!(off_the_edge.text().await.unwrap().contains("edge"));

    assert_eq!(
        send(r#"{"url":"paperless.local"}"#).await.unwrap().status(),
        400
    );
    assert_eq!(send("not json").await.unwrap().status(), 400);
    assert!(hub.take_commands().is_empty());

    let good = send(r#"{"url":"http://paperless.local:8000","roi":"0.1,0.1,0.5,0.5","tags":["Hauptstraße 12"]}"#)
        .await
        .unwrap();
    assert!(good.status().is_success());
    match hub.take_commands().as_slice() {
        [Command::Settings(settings)] => {
            assert_eq!(settings.roi.as_deref(), Some("0.1,0.1,0.5,0.5"));
            assert_eq!(
                settings.tags.as_deref(),
                Some(&["Hauptstraße 12".to_string()][..])
            );
        }
        other => panic!("expected one settings command, got {other:?}"),
    }
}

#[tokio::test]
async fn only_pages_of_the_open_letter_are_served() {
    let dir = TempDir::new("pages");
    std::fs::create_dir_all(dir.0.join("open")).unwrap();
    std::fs::write(dir.0.join("open/1789400000-000000001.jpg"), b"\xff\xd8page").unwrap();
    std::fs::write(dir.0.join("settings.toml"), "token = \"secret\"").unwrap();
    let (base, _hub) = start(&dir.0, None).await;

    let page = client()
        .get(format!("{base}/page/1789400000-000000001.jpg"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["content-type"], "image/jpeg");

    for sneaky in [
        "..%2Fsettings.toml",
        "%2E%2E%2Fsettings.toml",
        "settings.toml",
        ".jpg",
    ] {
        let reply = client()
            .get(format!("{base}/page/{sneaky}"))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 404, "{sneaky}");
    }
}
