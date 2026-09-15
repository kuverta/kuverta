//! Settings from the setup page, on disk.

use std::path::PathBuf;

use scannerd::settings::{valid_roi, Settings};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scannerd-settings-{}-{name}", std::process::id()));
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

#[test]
fn nothing_saved_is_no_settings() {
    let dir = TempDir::new("none");
    assert_eq!(Settings::load(&dir.0).unwrap(), Settings::default());
}

#[test]
fn saved_settings_come_back_and_only_their_owner_can_read_them() {
    let dir = TempDir::new("roundtrip");
    let settings = Settings {
        url: Some("http://paperless.local:8000".into()),
        token: Some("secret".into()),
        tags: Some(vec!["Hauptstraße 12".into()]),
        roi: Some("0.25,0.25,0.75,0.75".into()),
    };
    settings.save(&dir.0).unwrap();

    assert_eq!(Settings::load(&dir.0).unwrap(), settings);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(Settings::path(&dir.0))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "it holds a token");
    }
}

#[test]
fn a_blank_token_from_the_page_keeps_the_one_there_is() {
    // The page never shows the token, so it cannot send it back: saving the
    // crop must not log the scanner out of Paperless.
    let mut settings = Settings {
        token: Some("secret".into()),
        url: Some("http://old.local".into()),
        ..Default::default()
    };
    settings.merge(Settings {
        url: Some("  ".into()),
        token: Some("".into()),
        tags: Some(vec![" post ".into(), "".into()]),
        roi: Some("0.1,0.1,0.5,0.5".into()),
    });

    assert_eq!(settings.token.as_deref(), Some("secret"));
    assert_eq!(settings.url.as_deref(), Some("http://old.local"));
    assert_eq!(settings.tags, Some(vec!["post".to_string()]));
    assert_eq!(settings.roi.as_deref(), Some("0.1,0.1,0.5,0.5"));
}

#[test]
fn an_empty_crop_means_no_crop_and_an_unset_one_means_the_env_files() {
    let unset = Settings::default();
    assert_eq!(
        unset.effective_roi(Some("0.2,0.2,0.6,0.6")).as_deref(),
        Some("0.2,0.2,0.6,0.6")
    );

    let cleared = Settings {
        roi: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(cleared.effective_roi(Some("0.2,0.2,0.6,0.6")), None);
}

#[test]
fn a_crop_must_fit_inside_the_picture() {
    assert!(valid_roi("0.25,0.25,0.75,0.75").is_ok());
    assert!(valid_roi("0,0,1,1").is_ok());
    for bad in [
        "",
        "0.1,0.1,0.5",
        "a,b,c,d",
        "0.5,0.5,0.6,0.1",
        "0.1,0.1,0,0.5",
        "-0.1,0,0.5,0.5",
    ] {
        assert!(valid_roi(bad).is_err(), "{bad:?}");
    }
}
