//! Safe preview through a real worker process: that it draws, that it is
//! locked down before it looks, and that a worker that hangs, dies or lies
//! gets nothing past the caller.

use std::time::{Duration, Instant};

use core_rpc::preview::{self, SafePreview, Worker};

fn worker() -> Worker {
    Worker {
        program: env!("CARGO_BIN_EXE_kuverta-preview-worker").into(),
        args: Vec::new(),
        timeout: Duration::from_secs(20),
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let pixels = vec![40u8; (width * height * 3) as usize];
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn decoded(page: &str) -> image::DynamicImage {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(page)
        .unwrap();
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap()
}

#[test]
fn a_picture_comes_back_as_a_png_written_on_this_side() {
    let shown = preview::preview(&worker(), "Foto.jpg", "image/jpeg", &png(30, 20), 0, 1).unwrap();
    let SafePreview::Pages {
        first,
        total,
        pages,
    } = shown
    else {
        panic!("expected pages, got {shown:?}")
    };
    assert_eq!((first, total, pages.len()), (0, 1, 1));
    let page = decoded(&pages[0]);
    assert_eq!((page.width(), page.height()), (30, 20));
}

#[test]
fn a_web_page_comes_back_as_its_text() {
    let shown = preview::preview(
        &worker(),
        "login.html",
        "text/html",
        b"<h1>Anmelden</h1><script>fetch('https://evil.example')</script>",
        0,
        1,
    )
    .unwrap();
    match shown {
        SafePreview::Text { text } => {
            assert!(text.contains("Anmelden"), "{text}");
            assert!(!text.contains("evil.example"), "{text}");
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn a_worker_cannot_write_connect_start_or_read_anything() {
    let tried = preview::probe_worker(&worker()).unwrap();
    assert!(tried.len() >= 4, "{tried:?}");
    for (what, stopped) in &tried {
        assert!(stopped, "a locked-down worker could {what}");
    }
}

#[test]
#[cfg(unix)]
fn a_worker_that_hangs_is_stopped_on_time() {
    let hanging = Worker {
        program: "/bin/sleep".into(),
        args: vec!["30".into()],
        timeout: Duration::from_millis(500),
    };
    let started = Instant::now();
    let err = preview::preview(&hanging, "a.pdf", "application/pdf", b"%PDF", 0, 1).unwrap_err();
    assert!(err.contains("stopped"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
#[cfg(unix)]
fn a_worker_that_dies_or_talks_nonsense_shows_nothing() {
    for (program, args) in [
        ("/usr/bin/false", vec![]),
        ("/bin/echo", vec!["{\"kind\":\"text\"}"]),
    ] {
        let odd = Worker {
            program: program.into(),
            args: args.into_iter().map(String::from).collect(),
            timeout: Duration::from_secs(5),
        };
        assert!(
            preview::preview(&odd, "a.png", "image/png", &png(2, 2), 0, 1).is_err(),
            "{program}"
        );
    }
}

#[test]
fn what_the_assistant_reads_is_read_in_the_worker_too() {
    match preview::contents(
        &worker(),
        "notiz.txt",
        "text/plain",
        "Termin am Montag".as_bytes(),
    ) {
        core_rpc::readable::Contents::Text(text) => assert_eq!(text, "Termin am Montag"),
        other => panic!("expected text, got {other:?}"),
    }
    // A worker that is not there is a file that could not be read, not a
    // file read in this process instead.
    let missing = Worker {
        program: "/nonexistent/kuverta".into(),
        args: Vec::new(),
        timeout: Duration::from_secs(5),
    };
    assert!(matches!(
        preview::contents(&missing, "notiz.txt", "text/plain", b"x"),
        core_rpc::readable::Contents::Unreadable(_)
    ));
}
