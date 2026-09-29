//! OpenPGP through the surface the window uses: compose flags on
//! `DraftInput`, and a stored encrypted message opened by `Core::message`.
//!
//! No server: a preview is built by the same construction `send` uses, so
//! what it shows is what would be submitted and filed in Sent. Passphrases
//! are held in memory — a keychain read from a test would raise a dialog.

use std::sync::Arc;

use core_pgp::MemoryPassphrases;
use core_rpc::{Core, DraftAttachment, DraftInput, Keyring, RpcError, Session, SignatureState};
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("kuverta-rpc-pgp-{}-{name}", std::process::id()));
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

const ERIKA: &str = "erika@example.com";
const MAX: &str = "max@example.org";

struct World {
    dir: TempDir,
    erika: Keyring,
    account: AccountId,
    inbox: FolderId,
}

/// Erika's data directory, her account, her key pair, and Max's certificate.
fn world(name: &str) -> World {
    let dir = TempDir::new(name);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Erika Mustermann".into(),
            email: ERIKA.into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: ERIKA.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    let erika = Keyring::new(dir.0.join("pgp"), Arc::new(MemoryPassphrases::default()));
    erika
        .generate("Erika Mustermann", ERIKA, Some("correct horse"))
        .unwrap();

    let max = Keyring::new(
        dir.0.join("max-pgp"),
        Arc::new(MemoryPassphrases::default()),
    );
    let max_key = max.generate("Max Mustermann", MAX, None).unwrap();
    erika
        .import(&max.export_public(&max_key.fingerprint).unwrap())
        .unwrap();

    World {
        dir,
        erika,
        account,
        inbox,
    }
}

fn session(world: &World) -> Session {
    Session::new(&world.dir.0).with_keyring(world.erika.clone())
}

fn core(world: &World) -> Core {
    Core::new(
        Store::open(world.dir.0.join("kuverta.db")).unwrap(),
        Blobs::new(world.dir.0.join("blobs")),
    )
    .with_keyring(world.erika.clone())
}

fn draft(sign: bool, encrypt: bool) -> DraftInput {
    DraftInput {
        to: vec![format!("Max Mustermann <{MAX}>")],
        subject: "Vertrag".into(),
        body: "Der Vertrag liegt bei.\n".into(),
        sign,
        encrypt,
        ..Default::default()
    }
}

/// Files raw bytes in INBOX the way a sync would.
fn store_message(world: &World, core: &Core, key: &str, raw: &[u8]) -> MessageId {
    let path = core.blobs().put(world.account, key, raw).unwrap();
    core.store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some(format!("{key}@example.com")),
                subject: Some("Vertrag".into()),
                from_addr: Some(ERIKA.into()),
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
fn a_draft_without_the_flags_still_deserialises() {
    // Scheduled sends will store DraftInput as JSON; ones stored before these
    // fields existed must still read, as plain mail.
    let input: DraftInput = serde_json::from_str(
        r#"{"to":["a@example.com"],"cc":[],"bcc":[],"subject":"s","body":"b",
            "reply_to":null,"reply_all":false,"forward":null}"#,
    )
    .unwrap();
    assert!(!input.sign && !input.encrypt);
}

#[test]
fn a_preview_shows_the_message_as_it_will_go_out() {
    let world = world("preview");
    let session = session(&world);

    let plain = session.preview(ERIKA, &draft(false, false)).unwrap();
    assert!(!plain.signed && !plain.encrypted);
    assert!(plain.rfc822.contains("Der Vertrag liegt bei."));

    let signed = session.preview(ERIKA, &draft(true, false)).unwrap();
    assert!(signed.signed && !signed.encrypted);
    assert!(signed.rfc822.contains("multipart/signed"));
    assert!(signed.rfc822.contains("-----BEGIN PGP SIGNATURE-----"));

    let sealed = session.preview(ERIKA, &draft(true, true)).unwrap();
    assert!(sealed.signed && sealed.encrypted);
    assert!(sealed.rfc822.contains("multipart/encrypted"));
    assert!(
        !sealed.rfc822.contains("Vertrag liegt"),
        "the body must not leak"
    );
    // The envelope is Draft::build's, untouched.
    assert_eq!(sealed.recipients, vec![MAX]);
    // What the compose window shows is still the text that was typed.
    assert_eq!(sealed.body, "Der Vertrag liegt bei.\n");
}

#[test]
fn encryption_refuses_bcc_and_names_recipients_without_keys() {
    let world = world("refusals");
    let session = session(&world);

    let with_bcc = DraftInput {
        bcc: vec!["hidden@example.net".into()],
        ..draft(false, true)
    };
    match session.preview(ERIKA, &with_bcc).unwrap_err() {
        RpcError::Rejected(message) => assert!(message.contains("Bcc"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    let unknown = DraftInput {
        cc: vec!["Anna <anna@example.net>".into()],
        ..draft(false, true)
    };
    match session.preview(ERIKA, &unknown).unwrap_err() {
        RpcError::Rejected(message) => assert!(message.contains("anna@example.net"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Signing alone discloses nobody, Bcc or not.
    let signed_bcc = DraftInput {
        bcc: vec!["hidden@example.net".into()],
        ..draft(true, false)
    };
    let preview = session.preview(ERIKA, &signed_bcc).unwrap();
    assert!(!preview.rfc822.contains("hidden@example.net"));
}

#[test]
fn an_encrypted_message_opens_decrypted_with_its_status() {
    let world = world("open");
    let session = session(&world);
    // The copy that would be filed in Sent: encrypted to Max and to Erika.
    let sealed = session.preview(ERIKA, &draft(true, true)).unwrap();

    let core = core(&world);
    let id = store_message(&world, &core, "sealed", sealed.rfc822.as_bytes());
    let detail = core.message(world.account, id).unwrap();

    let security = detail.security.expect("protected");
    assert!(security.encrypted && security.decrypted, "{security:?}");
    assert!(security.signed);
    let signature = security.signature.unwrap();
    assert_eq!(signature.state, SignatureState::Valid);
    assert_eq!(signature.signer_matches_sender, Some(true));
    assert_eq!(
        detail.body_text.map(|t| t.replace("\r\n", "\n")).as_deref(),
        Some("Der Vertrag liegt bei.\n")
    );
    assert!(!detail.pgp_keys_attached);

    // Without a keyring the message is still shown — as encrypted, and
    // without pretending the preamble is its text.
    let bare = Core::new(
        Store::open(world.dir.0.join("kuverta.db")).unwrap(),
        Blobs::new(world.dir.0.join("blobs")),
    );
    let detail = bare.message(world.account, id).unwrap();
    let security = detail.security.unwrap();
    assert!(security.encrypted && !security.decrypted);
    assert!(security.error.is_some());
    assert!(detail.body_text.is_none());
}

#[test]
fn ordinary_mail_carries_no_security() {
    let world = world("plain");
    let core = core(&world);
    let id = store_message(
        &world,
        &core,
        "plain",
        b"From: anna@example.net\r\nTo: erika@example.com\r\nSubject: hi\r\n\r\nhello\r\n",
    );
    let detail = core.message(world.account, id).unwrap();
    assert!(detail.security.is_none());
    assert_eq!(detail.body_text.as_deref().map(str::trim), Some("hello"));
}

#[test]
fn a_key_attached_to_a_message_can_be_imported() {
    let world = world("attached");
    let other = Keyring::new(
        world.dir.0.join("anna-pgp"),
        Arc::new(MemoryPassphrases::default()),
    );
    let anna = other.generate("Anna", "anna@example.net", None).unwrap();
    let armored = other.export_public(&anna.fingerprint).unwrap();
    let raw = format!(
        "From: anna@example.net\r\nTo: {ERIKA}\r\nSubject: my key\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nhere it is\r\n\
         --b\r\nContent-Type: application/pgp-keys\r\n\
         Content-Disposition: attachment; filename=\"anna.asc\"\r\n\r\n{}\r\n--b--\r\n",
        armored.replace('\n', "\r\n")
    );

    let core = core(&world);
    let id = store_message(&world, &core, "key", raw.as_bytes());
    let detail = core.message(world.account, id).unwrap();
    assert!(detail.pgp_keys_attached);
    assert!(detail.security.is_none());

    let report = core.pgp_import_from_message(world.account, id).unwrap();
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.imported[0].fingerprint, anna.fingerprint);

    let status = core
        .pgp_recipients(ERIKA, &["anna@example.net".into(), MAX.into()])
        .unwrap();
    assert!(status.can_encrypt && status.can_sign, "{status:?}");
    assert_eq!(core.pgp_keys().unwrap().len(), 3);
}

#[test]
fn an_encrypted_message_lists_the_attachments_inside_it() {
    let world = world("attachments");
    // A message with a file, encrypted to Erika as a whole: its only outer
    // part is ciphertext, and the file is inside.
    let plain = "From: Max Mustermann <max@example.org>\r\n\
                 To: erika@example.com\r\n\
                 Subject: Vertrag\r\n\
                 MIME-Version: 1.0\r\n\
                 Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
                 --b\r\nContent-Type: text/plain\r\n\r\nAnbei.\r\n\
                 --b\r\nContent-Type: application/pdf\r\n\
                 Content-Disposition: attachment; filename=\"Vertrag.pdf\"\r\n\r\n\
                 %PDF-1.4 geheim\r\n\
                 --b--\r\n";
    let sealed = core_pgp::protect(
        &world.erika,
        plain.as_bytes(),
        ERIKA,
        &[ERIKA.to_string()],
        &[],
        core_pgp::Protection {
            sign: false,
            encrypt: true,
        },
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("geheim"));

    let core = core(&world);
    let id = store_message(&world, &core, "sealed-file", &sealed);
    let detail = core.message(world.account, id).unwrap();
    assert_eq!(detail.attachments.len(), 1, "{:?}", detail.attachments);
    assert_eq!(detail.attachments[0].name, "Vertrag.pdf");
    let fetched = core.attachment(world.account, id, 0).unwrap();
    assert!(String::from_utf8_lossy(&fetched.bytes).starts_with("%PDF-1.4 geheim"));

    // Without the key: nothing to list, and nothing to fetch.
    let bare = Core::new(
        Store::open(world.dir.0.join("kuverta.db")).unwrap(),
        Blobs::new(world.dir.0.join("blobs")),
    );
    assert!(bare
        .message(world.account, id)
        .unwrap()
        .attachments
        .is_empty());
    assert!(bare.attachment(world.account, id, 0).is_err());
}

fn with_file(sign: bool, encrypt: bool) -> DraftInput {
    DraftInput {
        attachments: vec![DraftAttachment {
            name: "Vertrag unterschrieben.pdf".into(),
            // As a window with no type for it would send it.
            content_type: String::new(),
            data: b"%PDF-1.4 unterschrieben".to_vec(),
        }],
        ..draft(sign, encrypt)
    }
}

#[test]
fn a_file_sent_encrypted_is_inside_the_encryption_and_comes_back_out() {
    let world = world("file-sealed");
    let sealed = session(&world)
        .preview(ERIKA, &with_file(true, true))
        .unwrap();
    assert!(sealed.encrypted);
    // Neither its name nor its bytes are on the outside.
    assert!(
        !sealed.rfc822.contains("unterschrieben"),
        "{}",
        sealed.rfc822
    );
    assert!(!sealed.rfc822.contains("Vertrag unterschrieben"));

    // The copy filed in Sent, opened again: text and file both there.
    let core = core(&world);
    let id = store_message(&world, &core, "file-sealed", sealed.rfc822.as_bytes());
    let detail = core.message(world.account, id).unwrap();
    assert!(detail.security.unwrap().decrypted);
    assert_eq!(detail.attachments.len(), 1, "{:?}", detail.attachments);
    assert_eq!(detail.attachments[0].name, "Vertrag unterschrieben.pdf");
    // No type was given; the name said PDF.
    assert_eq!(detail.attachments[0].content_type, "application/pdf");
    let fetched = core.attachment(world.account, id, 0).unwrap();
    assert_eq!(fetched.bytes, b"%PDF-1.4 unterschrieben");
}

#[test]
fn a_file_sent_plain_is_a_second_part_after_the_text() {
    let world = world("file-plain");
    let plain = session(&world)
        .preview(ERIKA, &with_file(false, false))
        .unwrap();
    assert!(plain
        .rfc822
        .to_ascii_lowercase()
        .contains("multipart/mixed"));
    assert!(plain.rfc822.contains("Der Vertrag liegt bei."));
    // What compose shows is still only what was typed.
    assert_eq!(plain.body, "Der Vertrag liegt bei.\n");
}

#[test]
fn a_scheduled_draft_keeps_its_files_as_base64() {
    // A message scheduled for Monday is stored as its DraftInput in JSON; the
    // file it was written with must still be in it on Monday.
    let input = with_file(false, false);
    let json = serde_json::to_string(&input).unwrap();
    assert!(
        json.contains("\"data\":\"JVBERi0xLjQgdW50ZXJzY2hyaWViZW4=\""),
        "{json}"
    );
    let back: DraftInput = serde_json::from_str(&json).unwrap();
    assert_eq!(back.attachments, input.attachments);
    // And the debug form, which is what a log line gets, holds no bytes.
    let logged = format!("{input:?}");
    assert!(logged.contains("size: 23"), "{logged}");
    assert!(!logged.contains("37, 80, 68, 70"), "{logged}");
}

#[test]
fn more_files_than_a_server_takes_are_refused_before_anything_is_built() {
    let world = world("file-too-big");
    let mut input = with_file(false, false);
    input.attachments[0].data = vec![0; core_rpc::MAX_ATTACHMENT_BYTES + 1];
    match session(&world).preview(ERIKA, &input).unwrap_err() {
        RpcError::Rejected(message) => assert!(message.contains("25 MB"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}
