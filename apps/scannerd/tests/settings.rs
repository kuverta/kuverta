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
        corners: Some("0.1,0.1,0.9,0.1,0.9,0.9,0.1,0.9".into()),
        rotate: Some(90),
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
        corners: None,
        rotate: None,
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

#[test]
fn corners_decide_the_crop_and_the_page_decides_over_the_env_file() {
    let corners = "0.300,0.150,0.700,0.150,0.900,0.850,0.100,0.850";

    let from_env = Settings::default()
        .effective_view(Some("0.2,0.2,0.6,0.6"), Some(corners), None)
        .unwrap();
    assert_eq!(from_env.roi.as_deref(), Some("0.100,0.150,0.800,0.700"));
    assert_eq!(from_env.corners.unwrap().to_setting(), corners);

    // The page saved a plain crop: the env file's corners no longer apply.
    let boxed = Settings {
        roi: Some("0.1,0.1,0.5,0.5".into()),
        corners: Some(String::new()),
        ..Default::default()
    };
    let view = boxed.effective_view(None, Some(corners), None).unwrap();
    assert_eq!(view.roi.as_deref(), Some("0.1,0.1,0.5,0.5"));
    assert_eq!(view.corners, None);
    assert_eq!(view.corners_in_crop(), None);
    assert_eq!(view.warp(), None, "nothing to straighten or turn");

    // The page saved corners: they win over the env file's crop, and are
    // given to straightening as fractions of the photograph.
    let angled = Settings {
        roi: Some("0.100,0.150,0.800,0.700".into()),
        corners: Some(corners.into()),
        ..Default::default()
    };
    let view = angled.effective_view(Some("0,0,1,1"), None, None).unwrap();
    let inside = view.corners_in_crop().unwrap();
    assert!(
        (inside.0[0].0 - 0.25).abs() < 1e-9 && inside.0[0].1.abs() < 1e-9,
        "{inside:?}"
    );
    assert!(
        (inside.0[2].0 - 1.0).abs() < 1e-9 && (inside.0[2].1 - 1.0).abs() < 1e-9,
        "{inside:?}"
    );
}

#[test]
fn corners_that_cannot_be_used_are_refused() {
    let crossed = Settings {
        corners: Some("0.3,0.15,0.9,0.85,0.7,0.15,0.1,0.85".into()),
        ..Default::default()
    };
    assert!(crossed.validate().is_err());
    assert!(crossed.effective_view(None, None, None).is_err());

    let cleared = Settings {
        corners: Some(" ".into()),
        ..Default::default()
    };
    assert!(cleared.validate().is_ok());
}

#[test]
fn a_turn_is_the_pages_choice_or_the_starts_and_only_by_quarters() {
    let unset = Settings::default();
    assert_eq!(
        unset.effective_view(None, None, Some(180)).unwrap().rotate,
        180
    );
    assert_eq!(unset.effective_view(None, None, None).unwrap().rotate, 0);
    assert!(unset.effective_view(None, None, Some(45)).is_err());

    let turned = Settings {
        rotate: Some(0),
        ..Default::default()
    };
    assert_eq!(
        turned.effective_view(None, None, Some(180)).unwrap().rotate,
        0,
        "the page wins, even with none"
    );

    for bad in [45, 360, 1] {
        let settings = Settings {
            rotate: Some(bad),
            ..Default::default()
        };
        assert!(settings.validate().is_err(), "{bad}");
    }

    // Kept across a save that does not mention it.
    let mut kept = Settings {
        rotate: Some(270),
        ..Default::default()
    };
    kept.merge(Settings {
        roi: Some(String::new()),
        ..Default::default()
    });
    assert_eq!(kept.rotate, Some(270));
}

#[test]
fn a_turn_alone_still_goes_through_straightening_and_with_corners_starts_them_further_round() {
    let turned = Settings {
        rotate: Some(90),
        ..Default::default()
    };
    let view = turned.effective_view(None, None, None).unwrap();
    let warp = view.warp().expect("a turn is a warp");
    // Clockwise: the picture's bottom left becomes the result's top left.
    assert_eq!(warp.0[0], (0.0, 1.0));
    assert_eq!(warp.0[1], (0.0, 0.0));

    let corners = "0.300,0.150,0.700,0.150,0.900,0.850,0.100,0.850";
    let both = Settings {
        roi: Some("0.100,0.150,0.800,0.700".into()),
        corners: Some(corners.into()),
        rotate: Some(180),
        ..Default::default()
    };
    let view = both.effective_view(None, None, None).unwrap();
    let inside = view.corners_in_crop().unwrap();
    let warp = view.warp().unwrap();
    assert_eq!(warp.0[0], inside.0[2]);
    assert_eq!(warp.0[2], inside.0[0]);
}
