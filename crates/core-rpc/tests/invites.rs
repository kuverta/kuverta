//! Invitations in real mail: that one is found wherever the sender's program
//! put it, that answering is withheld from a stranger, and that the ledger
//! stops the second copy asking again.

use core_rpc::Core;
use core_store::model::*;
use core_store::{Blobs, Handoff, Store};

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
    let path = std::env::temp_dir().join(format!("kuverta-invites-{}-{name}", std::process::id()));
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

/// Mail is CRLF on the wire, and the parser should see it that way.
fn wire(lines: &[&str]) -> String {
    format!("{}\r\n", lines.join("\r\n"))
}

const EVENT: &[&str] = &[
    "BEGIN:VCALENDAR",
    "PRODID:-//Example//Kalender//DE",
    "VERSION:2.0",
    "METHOD:REQUEST",
    "BEGIN:VEVENT",
    "UID:4f8e-2026-11",
    "SEQUENCE:1",
    "DTSTART;TZID=Europe/Berlin:20261104T140000",
    "DTEND;TZID=Europe/Berlin:20261104T150000",
    "SUMMARY:Quartalsbesprechung",
    "LOCATION:Besprechungsraum 2",
    "ORGANIZER;CN=Erika Mustermann:mailto:erika@example.de",
    "ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:dev@kuverta.test",
    "END:VEVENT",
    "END:VCALENDAR",
];

/// An invitation the way a calendar program sends one: inside the
/// alternative, beside the text.
fn invitation() -> String {
    let mut lines = vec![
        "From: Erika Mustermann <erika@example.de>",
        "To: dev@kuverta.test",
        "Subject: Quartalsbesprechung",
        "MIME-Version: 1.0",
        "Content-Type: multipart/alternative; boundary=\"b1\"",
        "",
        "--b1",
        "Content-Type: text/plain; charset=utf-8",
        "",
        "Bitte um Rueckmeldung.",
        "",
        "--b1",
        "Content-Type: text/calendar; method=REQUEST; charset=utf-8",
        "",
    ];
    lines.extend_from_slice(EVENT);
    lines.extend_from_slice(&["", "--b1--", ""]);
    wire(&lines)
}

fn file(world: &World, uid: u32, raw: &str) -> MessageId {
    let key = format!("m{uid}");
    let path = world
        .core
        .blobs()
        .put(world.account, &key, raw.as_bytes())
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
                rfc822_message_id: Some(format!("{key}@example.de")),
                subject: parsed.subject().map(str::to_string),
                from_addr: parsed
                    .from()
                    .and_then(|f| f.first())
                    .and_then(|a| a.address())
                    .map(str::to_string),
                date_utc: Some(1_700_000_000 + i64::from(uid)),
                body_path: Some(path),
                ..Default::default()
            },
            Some(&Location {
                folder_id: world.inbox,
                uid,
                flags: String::new(),
            }),
        )
        .unwrap()
        .0
}

/// A letter this mailbox sent to `to`, which is what makes them a
/// correspondent rather than a stranger.
fn sent_to(world: &World, uid: u32, to: &str) {
    world
        .core
        .store()
        .upsert_message(
            world.account,
            &NewMessage {
                rfc822_message_id: Some(format!("sent{uid}@kuverta.test")),
                subject: Some("Re: Termine".into()),
                from_addr: Some("dev@kuverta.test".into()),
                recipients: Some(to.to_string()),
                date_utc: Some(1_600_000_000),
                ..Default::default()
            },
            Some(&Location {
                folder_id: world.inbox,
                uid,
                flags: String::new(),
            }),
        )
        .unwrap();
}

#[test]
fn an_invitation_inside_the_alternative_is_found() {
    let world = world("found");
    let id = file(&world, 1, &invitation());

    let detail = world.core.message(world.account, id).unwrap();
    assert_eq!(detail.offers.len(), 1, "{:#?}", detail.offers);
    let offer = &detail.offers[0].offer;
    assert_eq!(offer.title, "Quartalsbesprechung");
    assert_eq!(offer.key, "invite:4f8e-2026-11");
    assert!(!detail.offers[0].handled);

    let core_offers::Detail::Invite(invite) = &offer.detail;
    assert_eq!(invite.location.as_deref(), Some("Besprechungsraum 2"));
    assert_eq!(
        invite.organizer.as_ref().unwrap().address.as_deref(),
        Some("erika@example.de")
    );
    let begins = invite.begins.as_ref().unwrap();
    assert_eq!(begins.written, "2026-11-04 14:00");
    assert_eq!(begins.zone.as_deref(), Some("Europe/Berlin"));
    assert_eq!(
        begins.at, None,
        "a wall-clock time is not turned into an instant"
    );
}

#[test]
fn ordinary_mail_offers_nothing() {
    let world = world("plain");
    let raw = wire(&[
        "From: Erika Mustermann <erika@example.de>",
        "To: dev@kuverta.test",
        "Subject: Kein Termin",
        "",
        "Nur ein Brief.",
    ]);
    let id = file(&world, 1, &raw);
    assert!(world
        .core
        .message(world.account, id)
        .unwrap()
        .offers
        .is_empty());
}

#[test]
fn a_strangers_invitation_is_shown_without_an_answer() {
    let world = world("stranger");
    let id = file(&world, 1, &invitation());

    let detail = world.core.message(world.account, id).unwrap();
    let offer = &detail.offers[0].offer;
    assert_eq!(offer.standing, core_offers::Standing::Stranger);
    assert_eq!(offer.withheld, Some(core_offers::Withheld::Stranger));
    assert!(!offer.answerable());
    assert_eq!(
        offer.title, "Quartalsbesprechung",
        "it is still read and shown"
    );
}

#[test]
fn an_invitation_from_someone_written_to_may_be_answered() {
    let world = world("known");
    sent_to(&world, 1, "erika@example.de");
    let id = file(&world, 2, &invitation());

    let offer = &world.core.message(world.account, id).unwrap().offers[0].offer;
    assert_eq!(offer.standing, core_offers::Standing::Known);
    assert!(
        offer.answerable(),
        "a letter sent is what nobody else can fake"
    );
}

#[test]
fn a_newsletter_that_has_written_often_is_still_a_stranger() {
    let world = world("newsletter");
    // Fifty letters from them, none to them.
    for uid in 1..=5 {
        let raw = invitation().replace("UID:4f8e-2026-11", &format!("UID:bulk-{uid}"));
        file(&world, uid, &raw);
    }
    let id = file(&world, 6, &invitation());

    let offer = &world.core.message(world.account, id).unwrap().offers[0].offer;
    assert_eq!(
        offer.standing,
        core_offers::Standing::Stranger,
        "inbound volume is the sender's to manufacture"
    );
}

#[test]
fn the_same_invitation_twice_in_one_message_is_one_offer() {
    let world = world("twice");
    // Once in the alternative and once attached, which is what several
    // calendar programs send.
    let mut lines = vec![
        "From: Erika Mustermann <erika@example.de>",
        "To: dev@kuverta.test",
        "Subject: Quartalsbesprechung",
        "MIME-Version: 1.0",
        "Content-Type: multipart/mixed; boundary=\"outer\"",
        "",
        "--outer",
        "Content-Type: text/calendar; method=REQUEST; charset=utf-8",
        "",
    ];
    lines.extend_from_slice(EVENT);
    lines.extend_from_slice(&[
        "",
        "--outer",
        "Content-Type: application/ics; name=\"invite.ics\"",
        "Content-Disposition: attachment; filename=\"invite.ics\"",
        "",
    ]);
    lines.extend_from_slice(EVENT);
    lines.extend_from_slice(&["", "--outer--", ""]);
    let id = file(&world, 1, &wire(&lines));

    let detail = world.core.message(world.account, id).unwrap();
    assert_eq!(detail.offers.len(), 1, "{:#?}", detail.offers);
}

#[test]
fn an_invitation_already_answered_does_not_ask_again() {
    let world = world("ledger");
    let id = file(&world, 1, &invitation());
    world
        .core
        .store()
        .record_handoff(&Handoff {
            key: "invite:4f8e-2026-11".into(),
            revision: 1,
            answer: "accepted".into(),
            sink: None,
            reference: None,
            at: 0,
        })
        .unwrap();

    let detail = world.core.message(world.account, id).unwrap();
    assert!(
        detail.offers[0].handled,
        "the ledger goes by the event, not the mail"
    );

    // The organiser moves the meeting: same UID, higher SEQUENCE, and that
    // is a new question.
    let moved = invitation().replace("SEQUENCE:1", "SEQUENCE:2");
    let id = file(&world, 2, &moved);
    let detail = world.core.message(world.account, id).unwrap();
    assert!(!detail.offers[0].handled);
}
