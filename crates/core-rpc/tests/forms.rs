//! Forms, against a real store: the assistant reads a PDF form's fields and
//! fills them through its tools — a text field, a multiline one, checkboxes —
//! and what it may not do is refused: a flat PDF, a name the form does not
//! have, a guess at a group. Without a model.

use core_ai::ToolCall;
use core_rpc::{form, AssistantEvent, Core};
use core_store::model::*;
use core_store::{Blobs, Store};
use serde_json::{json, Value};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct World {
    core: Core,
    account: AccountId,
    inbox: FolderId,
    _dir: TempDir,
}

fn world(name: &str) -> World {
    let path = std::env::temp_dir().join(format!("kuverta-forms-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "dev@kuverta.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    World {
        core: Core::new(store, Blobs::new(dir.0.join("blobs"))),
        account,
        inbox,
        _dir: dir,
    }
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: "1".into(),
        name: name.into(),
        arguments,
    }
}

fn result(outcome: &core_rpc::assistant::ToolOutcome) -> Value {
    serde_json::from_str(&outcome.content).unwrap()
}

/// A one-page questionnaire the way a form writer writes one: an AcroForm
/// with two text fields, a multiline one, and a pair of checkboxes whose
/// ticked state is `On`, each field merged with its widget and carrying a
/// tooltip.
fn questionnaire() -> Vec<u8> {
    questionnaire_with(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    )
}

fn questionnaire_with(form_font: &str) -> Vec<u8> {
    let content = "BT /F1 11 Tf 72 726 Td (Personalfragebogen) Tj ET";
    let text_field = |name: &str, label: &str, rect: &str, extra: &str| {
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T ({name}) /TU ({label}) /Rect [{rect}] \
             /DA (/Helv 0 Tf 0 g) /F 4 /P 4 0 R{extra} >>"
        )
    };
    let checkbox = |name: &str, rect: &str, on: usize, off: usize| {
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Btn /T ({name}) /Rect [{rect}] /AS /Off \
             /DA (/ZaDb 0 Tf 0 g) /F 4 /P 4 0 R /AP << /N << /On {on} 0 R /Off {off} 0 R >> >> >>"
        )
    };
    let tick =
        "<< /Type /XObject /Subtype /Form /BBox [0 0 12 12] /Length 0 >>\nstream\n\nendstream";
    let objects = vec![
        // 1: the catalog, with the form naming its fields and its font.
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 6 0 R 7 0 R 8 0 R 9 0 R] \
         /DR << /Font << /Helv 3 0 R >> >> /DA (/Helv 0 Tf 0 g) >> >>"
            .to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_string(),
        form_font.to_string(),
        // 4: the page, its widgets listed as its annotations.
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 10 0 R \
         /Resources << /Font << /F1 3 0 R >> >> /Annots [5 0 R 6 0 R 7 0 R 8 0 R 9 0 R] >>"
            .to_string(),
        text_field("Vorname", "Vorname des Mitarbeiters", "100 700 300 716", ""),
        text_field("Name", "Name", "100 670 300 686", ""),
        checkbox("ja", "100 640 112 652", 11, 12),
        checkbox("nein", "160 640 172 652", 13, 14),
        // 9: notes, multiline.
        text_field("Anmerkungen", "Anmerkungen", "100 560 400 620", " /Ff 4096"),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        tick.to_string(),
        tick.to_string(),
        tick.to_string(),
        tick.to_string(),
    ];
    let mut out = b"%PDF-1.6\n".to_vec();
    let mut offsets = Vec::new();
    for (at, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", at + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A flat one-page PDF with no form at all.
fn flat() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 720 Td (Nur Text) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (at, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", at + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Files a message with a PDF attached, the way a sync does.
fn file_with_pdf(world: &World, pdf: &[u8]) -> MessageId {
    use base64::Engine;
    let raw = format!(
        "From: Steuerbuero <post@steuerbuero.example>\r\n\
         To: dev@kuverta.test\r\n\
         Subject: Personalfragebogen bitte ausgefüllt zurück\r\n\
         Message-ID: <fragebogen@steuerbuero.example>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n\
         Bitte ausgefüllt zurück.\r\n\
         --b\r\n\
         Content-Type: application/pdf; name=\"Personalfragebogen.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"Personalfragebogen.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n\
         {}\r\n\
         --b--\r\n",
        base64::engine::general_purpose::STANDARD.encode(pdf)
    );
    let path = world
        .core
        .blobs()
        .put(world.account, "fragebogen", raw.as_bytes())
        .unwrap();
    world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some("fragebogen@steuerbuero.example".into()),
                subject: Some("Personalfragebogen bitte ausgefüllt zurück".into()),
                from_addr: Some("post@steuerbuero.example".into()),
                date_utc: Some(1_700_000_000),
                has_attachments: true,
                search_text: Some("Personalfragebogen".into()),
                body_path: Some(path),
                ..Default::default()
            },
            Some(&Location {
                folder_id: world.inbox,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap()
        .0
}

#[test]
fn the_fields_are_read_in_the_order_the_page_shows_them() {
    let fields = form::fields(&questionnaire()).unwrap();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Vorname", "Name", "ja", "nein", "Anmerkungen"]);
    assert_eq!(fields[0].kind, "text");
    assert_eq!(fields[0].label.as_deref(), Some("Vorname des Mitarbeiters"));
    assert_eq!(fields[0].page, Some(1));
    assert_eq!(fields[2].kind, "checkbox");
    assert_eq!(fields[2].options, ["On"]);
    assert!(fields[4].multiline);
    // The merged label — the same words as the name — says nothing, and is
    // left out.
    assert_eq!(fields[1].label, None);

    // A flat PDF has none.
    assert!(form::fields(&flat()).unwrap().is_empty());
}

#[test]
fn filling_writes_the_values_and_every_reader_sees_them() {
    let mut values = serde_json::Map::new();
    values.insert("Vorname".into(), json!("Erika"));
    values.insert("Name".into(), json!("Mustermann"));
    values.insert("ja".into(), json!(true));
    values.insert("nein".into(), json!(false));
    values.insert(
        "Anmerkungen".into(),
        json!("Eintritt zum 01.11.2026.\nGesellschafter-Geschäftsführerin."),
    );
    let filled = form::fill(&questionnaire(), &values).unwrap();
    assert_eq!(filled.filled, 5);

    // Read back: the values stand, the checkbox is ticked.
    let fields = form::fields(&filled.pdf).unwrap();
    let by_name = |name: &str| fields.iter().find(|f| f.name == name).unwrap();
    assert_eq!(by_name("Vorname").value.as_deref(), Some("Erika"));
    assert_eq!(by_name("Name").value.as_deref(), Some("Mustermann"));
    assert_eq!(by_name("ja").value.as_deref(), Some("On"));
    assert_eq!(by_name("nein").value, None);
    assert!(by_name("Anmerkungen")
        .value
        .as_deref()
        .unwrap()
        .contains("Geschäftsführerin"));
    // The appearance streams drawn for the readers that ignore the values:
    // the text is in the bytes as WinAnsi literals.
    let bytes = String::from_utf8_lossy(&filled.pdf);
    assert!(bytes.contains("Erika"), "no appearance was drawn");
    // Not set: a reader honouring it would throw the drawn appearances away
    // and lay the text out again with the form's own font.
    assert!(!bytes.contains("NeedAppearances"));

    // The original is a form still empty.
    assert!(form::fields(&questionnaire())
        .unwrap()
        .iter()
        .all(|f| f.value.is_none()));
}

#[test]
fn umlauts_survive_a_form_whose_font_names_no_encoding() {
    // The usual form writer's /Helv carries no /Encoding, and text drawn
    // with it as WinAnsi loses every umlaut. Such a font is passed over for
    // one of ours that says WinAnsi.
    let bare = questionnaire_with("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut values = serde_json::Map::new();
    values.insert("Name".into(), json!("Müller-Lüdenscheidt"));
    let filled = form::fill(&bare, &values).unwrap();
    let bytes = String::from_utf8_lossy(&filled.pdf);
    // The appearance writes ü as the WinAnsi octal escape, and the font it
    // points at declares that encoding.
    assert!(
        bytes.contains("M\\374ller-L\\374denscheidt"),
        "no appearance with the umlauts"
    );
    assert!(
        bytes.contains("WinAnsiEncoding"),
        "no WinAnsi font was added"
    );
    assert_eq!(
        form::fields(&filled.pdf).unwrap()[1].value.as_deref(),
        Some("Müller-Lüdenscheidt")
    );
}

#[test]
fn what_the_form_does_not_have_is_refused_whole() {
    let mut values = serde_json::Map::new();
    values.insert("Vorname".into(), json!("Erika"));
    values.insert("Geburtsort".into(), json!("Musterstadt"));
    let err = form::fill(&questionnaire(), &values).unwrap_err();
    assert!(err.contains("Geburtsort"), "{err}");
    assert!(err.contains("read_form"), "{err}");

    // A flat PDF cannot be filled at all.
    let mut values = serde_json::Map::new();
    values.insert("Vorname".into(), json!("Erika"));
    let err = form::fill(&flat(), &values).unwrap_err();
    assert!(err.contains("no form fields"), "{err}");
}

#[test]
fn the_assistant_reads_and_fills_a_form_through_its_tools() {
    let world = world("tools");
    let message = file_with_pdf(&world, &questionnaire());

    // read_form lists the fields of the attachment.
    let read = world.core.assistant_tool(
        world.account,
        &call("read_form", json!({"message_id": message, "attachment": 0})),
    );
    let out = result(&read);
    assert_eq!(out["count"], 5, "{out}");
    assert_eq!(out["fields"][0]["name"], "Vorname");
    assert!(matches!(read.event, Some(AssistantEvent::Looked { .. })));

    // fill_form writes the values into a copy, shown as a card.
    let filled = world.core.assistant_tool(
        world.account,
        &call(
            "fill_form",
            json!({"message_id": message, "attachment": 0, "values": {
                "Vorname": "Erika", "Name": "Mustermann", "ja": "ja"
            }, "note": "der Fragebogen"}),
        ),
    );
    let out = result(&filled);
    assert_eq!(out["name"], "Personalfragebogen (filled).pdf", "{out}");
    assert_eq!(out["filled"], 3);
    let id = out["document_id"].as_i64().unwrap();
    match &filled.event {
        Some(AssistantEvent::Document { document, note }) => {
            assert!(!document.signed);
            assert_eq!(note.as_deref(), Some("der Fragebogen"));
        }
        other => panic!("{other:?}"),
    }
    let file = world.core.document(id).unwrap();
    let fields = form::fields(&file.bytes).unwrap();
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "ja")
            .unwrap()
            .value
            .as_deref(),
        Some("On")
    );
    let listed = world.core.documents(world.account).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].note.as_deref().unwrap().contains("filled copy"));

    // A flat PDF: read_form says so instead of listing nothing by accident.
    let flat_message = {
        let w = &world;
        use base64::Engine;
        let raw = format!(
            "From: a@b.test\r\nTo: dev@kuverta.test\r\nSubject: flach\r\n\
             Message-ID: <flach@b.test>\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nflach\r\n\
             --b\r\nContent-Type: application/pdf; name=\"flach.pdf\"\r\n\
             Content-Disposition: attachment; filename=\"flach.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--b--\r\n",
            base64::engine::general_purpose::STANDARD.encode(flat())
        );
        let path = w
            .core
            .blobs()
            .put(w.account, "flach", raw.as_bytes())
            .unwrap();
        w.core
            .store()
            .upsert_message(
                w.account,
                &NewMessage {
                    rfc822_message_id: Some("flach@b.test".into()),
                    subject: Some("flach".into()),
                    has_attachments: true,
                    body_path: Some(path),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: w.inbox,
                    uid: 2,
                    flags: String::new(),
                }),
            )
            .unwrap()
            .0
    };
    let read = world.core.assistant_tool(
        world.account,
        &call(
            "read_form",
            json!({"message_id": flat_message, "attachment": 0}),
        ),
    );
    assert!(result(&read)["note"]
        .as_str()
        .unwrap()
        .contains("no form fields"));

    // And filling it is refused, with the error on the event for the person.
    let refused = world.core.assistant_tool(
        world.account,
        &call(
            "fill_form",
            json!({"message_id": flat_message, "attachment": 0, "values": {"Vorname": "Erika"}}),
        ),
    );
    assert!(
        refused.content.contains("no form fields"),
        "{}",
        refused.content
    );
    assert!(matches!(refused.event, Some(AssistantEvent::Failed { .. })));

    // A fill without values is told what is missing.
    let missing = world.core.assistant_tool(
        world.account,
        &call("fill_form", json!({"message_id": message, "attachment": 0})),
    );
    assert!(missing.content.contains("values"), "{}", missing.content);
}

#[test]
fn a_filled_form_can_then_be_signed_as_a_document() {
    let world = world("sign");
    let message = file_with_pdf(&world, &questionnaire());
    let filled = world.core.assistant_tool(
        world.account,
        &call(
            "fill_form",
            json!({"message_id": message, "attachment": 0, "values": {"Vorname": "Erika"}}),
        ),
    );
    let id = result(&filled)["document_id"].as_i64().unwrap();

    // A signature stored, the filled copy signed by its document id: the
    // flow the answer button takes for "ausgefüllt und unterschrieben".
    let mut image = image::RgbaImage::from_pixel(200, 60, image::Rgba([255, 255, 255, 255]));
    for x in 20..180 {
        for y in 28..34 {
            image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    world
        .core
        .set_signature(&png.into_inner(), "image/png")
        .unwrap();
    let signed = world
        .core
        .assistant_tool(world.account, &call("sign_pdf", json!({"document_id": id})));
    let out = result(&signed);
    assert_eq!(
        out["name"], "Personalfragebogen (filled) (signed).pdf",
        "{out}"
    );
    // The signed copy still carries the filled values.
    let signed_id = out["document_id"].as_i64().unwrap();
    let bytes = world.core.document(signed_id).unwrap().bytes;
    let fields = form::fields(&bytes).unwrap();
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "Vorname")
            .unwrap()
            .value
            .as_deref(),
        Some("Erika")
    );
}
