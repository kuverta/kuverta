//! Who a letter went to.
//!
//! In Sent the sender is your own address on every letter, which tells one
//! from another not at all: the reading pane showed it and nothing else. The
//! recipients were on the row the whole time and on the message on disk, and
//! neither was reaching the window.

use core_rpc::Core;
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct World {
    core: Core,
    account: AccountId,
    _dir: TempDir,
}

fn world(name: &str) -> World {
    let path =
        std::env::temp_dir().join(format!("kuverta-recipients-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "erika@example.de".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "erika@example.de".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    World {
        core: Core::new(store, Blobs::new(dir.0.join("blobs"))),
        account,
        _dir: dir,
    }
}

/// A letter with its body on disk, as a full sync leaves one.
fn with_body(world: &World, raw: &str, recipients: &str) -> MessageId {
    let path = world
        .core
        .blobs()
        .put(world.account, "m1", raw.as_bytes())
        .unwrap();
    world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some("m1@example.de".into()),
                subject: Some("Rechnung coGIS".into()),
                from_addr: Some("erika@example.de".into()),
                recipients: Some(recipients.into()),
                date_utc: Some(1_756_620_000),
                body_path: Some(path),
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .0
}

/// And one without, as the first pass down a folder leaves one.
fn headers_only(world: &World, recipients: &str) -> MessageId {
    world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some("m2@example.de".into()),
                subject: Some("Abrechnung Gaza".into()),
                from_addr: Some("erika@example.de".into()),
                recipients: Some(recipients.into()),
                date_utc: Some(1_756_620_001),
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .0
}

#[test]
fn the_headers_say_who_it_went_to_when_the_letter_is_on_disk() {
    let world = world("headers");
    let raw = "From: Erika Mustermann <erika@example.de>\r\n\
               To: er-accounting@cadus.example, Ruben <ruben@cadus.example>\r\n\
               Cc: buchhaltung@cadus.example\r\n\
               Subject: Rechnung coGIS\r\n\r\n\
               anbei die Rechnung\r\n";
    let id = with_body(&world, raw, "er-accounting@cadus.example");

    let detail = world.core.message(world.account, id).unwrap();
    assert_eq!(
        detail.to,
        ["er-accounting@cadus.example", "ruben@cadus.example"]
    );
    assert_eq!(detail.cc, ["buchhaltung@cadus.example"]);
    // And the sender is still the sender.
    assert_eq!(detail.from.as_deref(), Some("erika@example.de"));
}

#[test]
fn a_letter_kept_without_its_body_still_knows_who_it_was_for() {
    // This is the case that showed nothing at all: no message on disk, so no
    // headers to read, so a reading pane with the sender's own address on it
    // and no other name anywhere.
    let world = world("no-body");
    let id = headers_only(&world, "er-accounting@cadus.example, ruben@cadus.example");

    let detail = world.core.message(world.account, id).unwrap();
    assert_eq!(
        detail.to,
        ["er-accounting@cadus.example", "ruben@cadus.example"],
        "the row's own record of the recipients should stand in"
    );
}

#[test]
fn a_comma_inside_somebody_s_name_does_not_split_them_in_two() {
    // The row keeps the recipients as one string, comma separated, the way
    // the headers had them — and a display name may hold a comma of its own.
    // A piece with no `@` in it cannot be an address, so it belongs to the
    // one before it.
    let world = world("comma");
    let id = headers_only(
        &world,
        "\"Zemke, Nicolas\" <nic@example.de>, ruben@cadus.example",
    );

    let detail = world.core.message(world.account, id).unwrap();
    assert_eq!(detail.to.len(), 2, "{:?}", detail.to);
    assert!(detail.to[0].contains("Zemke, Nicolas"), "{:?}", detail.to);
    assert!(detail.to[0].contains("nic@example.de"), "{:?}", detail.to);
    assert_eq!(detail.to[1], "ruben@cadus.example");
}

#[test]
fn a_letter_with_nobody_on_it_says_nothing_rather_than_an_empty_name() {
    let world = world("none");
    let id = world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some("m3@example.de".into()),
                subject: Some("Kein Empfänger".into()),
                from_addr: Some("erika@example.de".into()),
                date_utc: Some(1_756_620_002),
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .0;

    let detail = world.core.message(world.account, id).unwrap();
    assert!(detail.to.is_empty(), "{:?}", detail.to);
    assert!(detail.cc.is_empty(), "{:?}", detail.cc);
}
