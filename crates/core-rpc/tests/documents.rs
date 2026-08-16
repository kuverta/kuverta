//! Documents and the signature, against a real store: the assistant writes a
//! letter, signs it, signs a PDF that came as an attachment, hands documents
//! to a draft, lists and deletes them — without a model.

use core_ai::ToolCall;
use core_rpc::{AssistantEvent, Core};
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
    let path =
        std::env::temp_dir().join(format!("kuverta-documents-{}-{name}", std::process::id()));
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

/// A small picture of a signature: a black stroke on grey paper, as a
/// photograph has it.
fn signature_png() -> Vec<u8> {
    let mut image = image::RgbaImage::from_pixel(200, 60, image::Rgba([190, 190, 190, 255]));
    for x in 20..180 {
        for y in 28..34 {
            image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A one-page PDF saying `line`, as a simple PDF writer would write it.
fn pdf_saying(line: &str) -> Vec<u8> {
    pdf_of(&[format!("BT /F1 12 Tf 72 720 Td ({line}) Tj ET")])
}

/// A contract of two pages, the second ruling a line for each party with the
/// words that name it beneath, the way a word processor writes one.
fn contract() -> Vec<u8> {
    let rule = "_".repeat(45);
    pdf_of(&[
        "BT /F1 12 Tf 72 720 Td (Mietvertrag Seite 1) Tj ET".to_string(),
        format!(
            "BT /F1 11 Tf 70 400 Td ({rule}) Tj ET\n\
             BT /F1 11 Tf 70 386 Td (Unterschrift Vermieter) Tj ET\n\
             BT /F1 11 Tf 70 300 Td ({rule}) Tj ET\n\
             BT /F1 11 Tf 70 286 Td (Unterschrift Mieter) Tj ET"
        ),
    ])
}

/// A form with one line to sign and nothing written beneath it.
fn form() -> Vec<u8> {
    pdf_of(&[format!(
        "BT /F1 11 Tf 70 414 Td (Bitte hier unterschreiben:) Tj ET\n\
         BT /F1 11 Tf 70 400 Td ({}) Tj ET",
        "_".repeat(30)
    )])
}

/// A PDF of one page per content stream, letter-sized, with Helvetica as /F1.
fn pdf_of(streams: &[String]) -> Vec<u8> {
    let first = 4;
    let kids: Vec<String> = (0..streams.len())
        .map(|at| format!("{} 0 R", first + at * 2))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            streams.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    for (at, stream) in streams.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            first + at * 2 + 1
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
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
        "From: Vermieter <post@hausverwaltung.example>\r\n\
         To: dev@kuverta.test\r\n\
         Subject: Mietvertrag zur Unterschrift\r\n\
         Message-ID: <vertrag@hausverwaltung.example>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n\
         Bitte unterschrieben zurück.\r\n\
         --b\r\n\
         Content-Type: application/pdf; name=\"Mietvertrag.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"Mietvertrag.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n\
         {}\r\n\
         --b--\r\n",
        base64::engine::general_purpose::STANDARD.encode(pdf)
    );
    let path = world
        .core
        .blobs()
        .put(world.account, "vertrag", raw.as_bytes())
        .unwrap();
    let parsed = mail_parser::MessageParser::default()
        .parse(raw.as_bytes())
        .unwrap();
    world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some("vertrag@hausverwaltung.example".into()),
                subject: parsed.subject().map(str::to_string),
                from_addr: Some("post@hausverwaltung.example".into()),
                date_utc: Some(1_700_000_000),
                has_attachments: true,
                search_text: Some("Mietvertrag zur Unterschrift".into()),
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
fn the_assistant_writes_a_letter_and_signs_it_once_there_is_a_signature() {
    let world = world("write");
    let text = "# Kündigung\n\nErika Mustermann\nMusterstraße 1\n12345 Musterstadt\n\nSehr geehrte Damen und Herren,\n\nhiermit kündige ich meine Mitgliedschaft Nr. 4711 fristgerecht.\n\nMit freundlichen Grüßen\n\n[signature]\nErika Mustermann";

    // Asked for it signed before there is a signature: refused, and the
    // model is told where the person adds one.
    let refused = world.core.assistant_tool(
        world.account,
        &call(
            "write_pdf",
            json!({"name": "Kündigung", "text": text, "sign": true}),
        ),
    );
    assert!(refused.content.contains("Settings"), "{}", refused.content);
    assert!(matches!(refused.event, Some(AssistantEvent::Failed { .. })));
    assert!(world.core.documents(world.account).unwrap().is_empty());

    // Unsigned, it is written, with a gap where the signature goes.
    let written = world.core.assistant_tool(
        world.account,
        &call(
            "write_pdf",
            json!({"name": "Kündigung", "text": text, "note": "the cancellation"}),
        ),
    );
    let out = result(&written);
    assert_eq!(out["signed"], false);
    assert_eq!(out["pages"], 1);
    let id = out["document_id"].as_i64().unwrap();
    match &written.event {
        Some(AssistantEvent::Document { document, note }) => {
            assert_eq!(document.name, "Kündigung.pdf");
            assert_eq!(note.as_deref(), Some("the cancellation"));
        }
        other => panic!("{other:?}"),
    }
    let file = world.core.document(id).unwrap();
    let text_in = pdf_extract::extract_text_from_mem(&file.bytes).unwrap();
    assert!(text_in.contains("4711"), "{text_in}");
    assert!(text_in.contains("Musterstraße 1"), "{text_in}");

    // With a signature stored, signed.
    world
        .core
        .set_signature(&signature_png(), "image/png")
        .unwrap();
    let settings = world.core.signature().unwrap();
    assert!(settings.present);
    // Cut out of its paper and cropped to the stroke.
    assert!(
        settings.width.unwrap() < 200 && settings.height.unwrap() < 60,
        "{settings:?}"
    );
    assert!(settings.image.is_some());
    let signed = world.core.assistant_tool(
        world.account,
        &call(
            "write_pdf",
            json!({"name": "Kündigung", "text": text, "sign": "true"}),
        ),
    );
    assert_eq!(result(&signed)["signed"], true);
    let signed_id = result(&signed)["document_id"].as_i64().unwrap();
    let signed_text =
        pdf_extract::extract_text_from_mem(&world.core.document(signed_id).unwrap().bytes).unwrap();
    let today = chrono::Local::now().format("%d.%m.%Y").to_string();
    assert!(signed_text.contains(&today), "{signed_text}");
    // The settings show it cut out: a PNG with the paper transparent.
    let settings = world.core.signature().unwrap();
    assert_eq!(settings.content_type.as_deref(), Some("image/png"));
    assert!(!world.core.signature_sample().unwrap().is_empty());
    let listed = world.core.documents(world.account).unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|d| d.signed));

    // Read back by the model's tool: the listing names both.
    let list = world
        .core
        .assistant_tool(world.account, &call("list_documents", json!({})));
    assert_eq!(result(&list)["documents"].as_array().unwrap().len(), 2);
}

#[test]
fn the_signature_lands_on_the_line_the_document_rules_for_it() {
    let world = world("line");
    world.core.set_signature_place(Some("Musterstadt")).unwrap();
    world
        .core
        .set_signature(&signature_png(), "image/png")
        .unwrap();

    // One line, and nothing to say where: it is found, and the signature
    // sits on it rather than above the bottom edge of the last page.
    let message = file_with_pdf(&world, &form());
    let signed = world.core.assistant_tool(
        world.account,
        &call("sign_pdf", json!({"message_id": message, "attachment": 0})),
    );
    let out = result(&signed);
    assert!(
        out["where"]
            .as_str()
            .unwrap()
            .contains("on the line “Bitte hier unterschreiben:” on page 1"),
        "{out}"
    );
    let id = out["document_id"].as_i64().unwrap();
    let text = pdf_extract::extract_text_from_mem(&world.core.document(id).unwrap().bytes).unwrap();
    // Nothing is written under that line, so the place and the date go there.
    assert!(text.contains("Musterstadt, "), "{text}");

    // A contract that rules a line for each party is asked about, with both
    // named, rather than signed in the wrong place.
    let contract = file_with_pdf(&world, &contract());
    let asked = world.core.assistant_tool(
        world.account,
        &call("sign_pdf", json!({"message_id": contract, "attachment": 0})),
    );
    assert!(
        asked.content.contains("2 places to sign"),
        "{}",
        asked.content
    );
    assert!(
        asked.content.contains("Unterschrift Vermieter"),
        "{}",
        asked.content
    );
    assert!(
        asked.content.contains("Unterschrift Mieter"),
        "{}",
        asked.content
    );
    assert_eq!(world.core.documents(world.account).unwrap().len(), 1);

    // Said which, it signs that one — on page 2, where the line is, and
    // without writing the date over the words that name the line.
    let signed = world.core.assistant_tool(
        world.account,
        &call(
            "sign_pdf",
            json!({"message_id": contract, "attachment": 0, "near": "Mieter"}),
        ),
    );
    let out = result(&signed);
    let where_it_went = out["where"].as_str().unwrap();
    assert!(
        where_it_went.contains("on the line “Unterschrift Mieter” on page 2"),
        "{where_it_went}"
    );
    let id = out["document_id"].as_i64().unwrap();
    let text = pdf_extract::extract_text_from_mem(&world.core.document(id).unwrap().bytes).unwrap();
    assert!(text.contains("Unterschrift Mieter"), "{text}");
    assert!(!text.contains("Musterstadt, "), "{text}");

    // Numbers given by hand are still taken as given: the line is not
    // looked for, and nothing is asked.
    let moved = world.core.assistant_tool(
        world.account,
        &call(
            "sign_pdf",
            json!({"message_id": contract, "attachment": 0, "page": 2, "above_bottom_mm": 40}),
        ),
    );
    assert!(
        result(&moved)["where"]
            .as_str()
            .unwrap()
            .contains("signature on page 2"),
        "{}",
        moved.content
    );
}

#[test]
fn a_signature_is_moved_by_signing_the_original_again_into_the_same_document() {
    let world = world("place");
    world.core.set_signature_place(Some("Musterstadt")).unwrap();
    world
        .core
        .set_signature(&signature_png(), "image/png")
        .unwrap();
    let message = file_with_pdf(&world, &form());
    let signed = world.core.assistant_tool(
        world.account,
        &call("sign_pdf", json!({"message_id": message, "attachment": 0})),
    );
    let id = result(&signed)["document_id"].as_i64().unwrap();
    let was = world.core.document_view(id).unwrap();
    let placed = was.placed.clone().expect("where it was signed");
    assert_eq!(
        placed.source,
        core_rpc::DocumentSource::Attachment { message, index: 0 }
    );
    assert!(placed.with_date);
    let text = pdf_extract::extract_text_from_mem(&world.core.document(id).unwrap().bytes).unwrap();
    assert!(text.contains("Musterstadt, "), "{text}");

    // Moved by hand: the same document, and the original signed again —
    // so the date written under the first one is gone with it rather than
    // left behind on a page that was stamped twice.
    let moved = world
        .core
        .place_signature(
            world.account,
            id,
            &core_rpc::Placed {
                page: 1,
                x_mm: 120.0,
                above_bottom_mm: 60.0,
                width_mm: 40.0,
                with_date: false,
                ..placed
            },
        )
        .unwrap();
    assert_eq!(moved.id, id);
    assert_eq!(moved.name, was.name);
    assert_eq!(world.core.documents(world.account).unwrap().len(), 1);
    let now = moved.placed.expect("where it is now");
    assert_eq!((now.page, now.x_mm, now.above_bottom_mm), (1, 120.0, 60.0));
    assert!(moved.note.unwrap().contains("moved by hand to page 1"));
    let text = pdf_extract::extract_text_from_mem(&world.core.document(id).unwrap().bytes).unwrap();
    assert!(!text.contains("Musterstadt, "), "{text}");
    assert!(text.contains("Bitte hier unterschreiben"), "{text}");

    // A document written here has no original, so its signature stays where
    // the text put it.
    let letter = world
        .core
        .write_document(
            world.account,
            "Brief",
            "Hallo\n\n[signature]\nErika",
            None,
            true,
        )
        .unwrap();
    assert!(letter.placed.is_none());
    assert!(world
        .core
        .place_signature(world.account, letter.id, &now)
        .unwrap_err()
        .to_string()
        .contains("cannot be moved"));
}

#[test]
fn a_contract_that_came_as_an_attachment_is_signed_as_a_copy_and_handed_to_a_draft() {
    let world = world("sign");
    world.core.set_signature_place(Some("Musterstadt")).unwrap();
    world
        .core
        .set_signature(&signature_png(), "image/jpeg")
        .unwrap();
    let message = file_with_pdf(&world, &pdf_saying("Mietvertrag Seite 1"));

    // The wrong attachment, by its kind, is refused by name.
    let not_pdf = world.core.assistant_tool(
        world.account,
        &call("sign_pdf", json!({"message_id": message, "attachment": 5})),
    );
    assert!(
        not_pdf.content.contains("no attachment"),
        "{}",
        not_pdf.content
    );

    let signed = world.core.assistant_tool(
        world.account,
        &call(
            "sign_pdf",
            json!({"message_id": message, "attachment": 0, "position": "bottom_right", "above_bottom_mm": 40}),
        ),
    );
    let out = result(&signed);
    let id = out["document_id"].as_i64().unwrap();
    assert_eq!(out["name"], "Mietvertrag (signed).pdf");
    let file = world.core.document(id).unwrap();
    assert!(file.view.signed);
    assert!(file
        .view
        .note
        .as_deref()
        .unwrap()
        .contains("Mietvertrag zur Unterschrift"));
    let text = pdf_extract::extract_text_from_mem(&file.bytes).unwrap();
    assert!(text.contains("Mietvertrag Seite 1"), "{text}");
    assert!(text.contains("Musterstadt, "), "{text}");
    // The original attachment is as it was.
    let original = world.core.attachment(world.account, message, 0).unwrap();
    assert_eq!(original.bytes, pdf_saying("Mietvertrag Seite 1"));

    // Signing the signed copy again — moved — is a third document, named once.
    let again = world.core.assistant_tool(
        world.account,
        &call(
            "sign_pdf",
            json!({"document_id": id, "with_date": false, "caption": "Erika Mustermann"}),
        ),
    );
    assert_eq!(result(&again)["name"], "Mietvertrag (signed).pdf");

    // Handed to a draft, by id.
    let drafted = world.core.assistant_tool(
        world.account,
        &call(
            "draft_reply",
            json!({"id": message, "body": "Anbei der unterschriebene Vertrag.", "documents": [id]}),
        ),
    );
    match &drafted.event {
        Some(AssistantEvent::Draft { documents, .. }) => {
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].id, id);
        }
        other => panic!("{other:?}"),
    }
    let unknown = world.core.assistant_tool(
        world.account,
        &call(
            "draft_message",
            json!({"to": "a@example.com", "subject": "x", "body": "y", "documents": [999]}),
        ),
    );
    assert!(
        unknown.content.contains("no document 999"),
        "{}",
        unknown.content
    );

    // Renamed, read, and deleted.
    let renamed = world.core.assistant_tool(
        world.account,
        &call(
            "rename_document",
            json!({"id": id, "name": "Mietvertrag unterschrieben"}),
        ),
    );
    assert_eq!(
        result(&renamed)["renamed"],
        "Mietvertrag unterschrieben.pdf"
    );
    let deleted = world
        .core
        .assistant_tool(world.account, &call("delete_document", json!({"id": id})));
    assert_eq!(
        result(&deleted)["deleted"],
        "Mietvertrag unterschrieben.pdf"
    );
    assert!(world.core.document(id).is_err());

    // Without a signature there is nothing to sign with.
    world.core.clear_signature().unwrap();
    let none = world.core.assistant_tool(
        world.account,
        &call("sign_pdf", json!({"message_id": message, "attachment": 0})),
    );
    assert!(none.content.contains("no signature"), "{}", none.content);
    assert_eq!(
        world.core.signature().unwrap().place.as_deref(),
        Some("Musterstadt")
    );
}

#[tokio::test]
async fn a_document_is_read_by_the_model_like_an_attachment() {
    let world = world("read");
    let written = world.core.assistant_tool(
        world.account,
        &call(
            "write_pdf",
            json!({"name": "Notiz", "text": "Die Rechnung 4711 ist bezahlt."}),
        ),
    );
    let id = result(&written)["document_id"].as_i64().unwrap();
    let mut events = Vec::new();
    let read = core_rpc::assistant::read_document(
        &world.core,
        world.account,
        &call("read_document", json!({"id": id})),
        6_000,
        &mut |event: &AssistantEvent| events.push(event.clone()),
    )
    .await;
    let out = result(&read);
    assert_eq!(out["name"], "Notiz.pdf");
    assert!(
        out["text"].as_str().unwrap().contains("4711"),
        "{}",
        out["text"]
    );
    let missing = core_rpc::assistant::read_document(
        &world.core,
        world.account,
        &call("read_document", json!({"id": 404})),
        6_000,
        &mut |_: &AssistantEvent| {},
    )
    .await;
    assert!(missing.content.contains("no document 404"));
}

#[test]
fn chats_are_kept_with_their_turns_and_log() {
    let world = world("chats");
    let turns = vec![
        core_ai::Turn::User {
            content: "Find the eSIM".into(),
        },
        core_ai::Turn::Assistant {
            content: "Here it is.".into(),
            calls: Vec::new(),
        },
    ];
    let log = json!([{ "question": "Find the eSIM", "reply": "Here it is.", "events": [] }]);
    let saved = world
        .core
        .save_chat(None, world.account, &turns, &log)
        .unwrap();
    assert_eq!(saved.title, "Find the eSIM");
    let again = world
        .core
        .save_chat(Some(saved.id), world.account, &turns, &log)
        .unwrap();
    assert_eq!(again.id, saved.id);
    let opened = world.core.chat(saved.id).unwrap();
    assert_eq!(opened.turns, turns);
    assert_eq!(opened.log, log);
    assert_eq!(world.core.chats(world.account).unwrap().len(), 1);
    world.core.delete_chat(saved.id).unwrap();
    assert!(world.core.chat(saved.id).is_err());
}
