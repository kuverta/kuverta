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
    let shown = preview::preview(
        &worker(),
        "Foto.jpg",
        "image/jpeg",
        &png(30, 20),
        0,
        1,
        None,
    )
    .unwrap();
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
        None,
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
    let err =
        preview::preview(&hanging, "a.pdf", "application/pdf", b"%PDF", 0, 1, None).unwrap_err();
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
            preview::preview(&odd, "a.png", "image/png", &png(2, 2), 0, 1, None).is_err(),
            "{program}"
        );
    }
}

/// A one-page PDF locked with `password`: RC4 128-bit under the standard
/// security handler, as a bank's statement or a payslip comes locked.
fn locked_pdf(password: &str) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
    use lopdf::{dictionary, Document, Object, Stream};

    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id =
        document.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let drawn = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 36.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("Kontoauszug")]),
            Operation::new("ET", vec![]),
        ],
    };
    let drawn_id = document.add_object(Stream::new(dictionary! {}, drawn.encode().unwrap()));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => drawn_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    // The file identifier goes into the encryption key, so it must be there.
    let id = Object::string_literal("kuverta test file identifier");
    document.trailer.set("ID", vec![id.clone(), id]);
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner-of-the-file",
        user_password: password,
        key_length: 128,
        permissions: Permissions::all(),
    })
    .unwrap();
    document.encrypt(&state).unwrap();
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

/// A locked PDF asks for its password and is drawn once it has it — through
/// a real worker, which does the decrypting after it has been locked down.
#[test]
fn a_locked_pdf_is_opened_with_the_password_the_window_sends() {
    let locked = locked_pdf("Hagebuttentee");
    let draw = |password| {
        preview::preview(
            &worker(),
            "Kontoauszug.pdf",
            "application/pdf",
            &locked,
            0,
            1,
            password,
        )
        .unwrap()
    };
    assert_eq!(draw(None), SafePreview::Password { wrong: false });
    assert_eq!(
        draw(Some("Kamillentee")),
        SafePreview::Password { wrong: true }
    );
    let SafePreview::Pages { total, pages, .. } = draw(Some("Hagebuttentee")) else {
        panic!("expected pages")
    };
    assert_eq!((total, pages.len()), (1, 1));
    let page = decoded(&pages[0]);
    // White paper with ink on it: the page was really decrypted and drawn.
    assert!(page.width() > 100 && page.height() > 100);
    assert!(
        page.to_luma8().iter().any(|&shade| shade < 80),
        "no ink drawn"
    );
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
