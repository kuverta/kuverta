//! Sync against a mailbox shaped like Gmail's.
//!
//! Gmail is the provider most likely to break this client, and the reasons are
//! structural: mail lives under a `[Gmail]/` hierarchy, archiving means the
//! folder marked `\All` rather than one marked `\Archive`, and — the one the
//! store was designed around — labels are exposed as folders, so the same
//! message is delivered under several names at once.
//!
//! All three can be reproduced without a Google account, which is what
//! `docker/dovecot/dovecot-gmail.conf` is for. What cannot: Gmail has no
//! CONDSTORE, its All Mail holds a copy of everything, and expunging from All
//! Mail deletes permanently. Those still need the real thing.

mod common;
use common as tempdir;

use core_accounts::EnvPassword;
use core_proto::client::{find_archive, find_sent, find_trash};
use core_proto::{ImapClient, ImapConfig};
use core_store::model::{ImapSecurity, NewAccount};
use core_store::{Blobs, Store};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 10144;

fn server_available() -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{PORT}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();

    if !reachable {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("Gmail-shaped server on {HOST}:{PORT} is required but not reachable");
        }
        eprintln!("skipping: Gmail-shaped dev server not running (`make dev-up`)");
    }
    reachable
}

fn auth() -> EnvPassword {
    // SAFETY: set before any thread reads it; the value is the dev password
    // from docker-compose.yml, not a real credential.
    unsafe { std::env::set_var("KUVERTA_GMAIL_PASSWORD", "devpass") };
    EnvPassword::new("KUVERTA_GMAIL_PASSWORD")
}

/// Test-only write access, for putting mail where the test needs it.
mod label {
    use futures::StreamExt;
    use tokio::net::TcpStream;

    pub type Session = async_imap::Session<TcpStream>;

    pub async fn login(user: &str) -> Session {
        let tcp = TcpStream::connect((super::HOST, super::PORT))
            .await
            .unwrap();
        let mut client = async_imap::Client::new(tcp);
        client.read_response().await.unwrap().expect("greeting");
        client
            .login(user, "devpass")
            .await
            .map_err(|(err, _)| err)
            .expect("login")
    }

    pub async fn clear(session: &mut Session, folder: &str) {
        session.select(folder).await.unwrap();
        let uids = session.uid_search("ALL").await.unwrap();
        if !uids.is_empty() {
            let set = uids
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut store = session
                .uid_store(set, "+FLAGS (\\Deleted)")
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            store.clear();
            session.expunge().await.unwrap().collect::<Vec<_>>().await;
        }
    }

    /// Puts the *same* message in a folder. Called twice with the same id, this
    /// is what a Gmail label looks like over IMAP.
    pub async fn put(session: &mut Session, folder: &str, message_id: &str, subject: &str) {
        let raw = format!(
            "Message-ID: <{message_id}>\r\n\
             From: Test <test@example.com>\r\n\
             To: You <you@gmail.com>\r\n\
             Subject: {subject}\r\n\
             Date: Mon, 07 Sep 2026 03:00:00 +0200\r\n\
             \r\n\
             body\r\n"
        );
        session
            .append(folder, None, None, raw.as_bytes())
            .await
            .unwrap();
    }
}

const ALL_MAIL: &str = "[Gmail]/All Mail";

struct Fixture {
    store: Store,
    account: i64,
    blobs: Blobs,
    client: ImapClient,
    _dir: tempdir::TempDir,
}

async fn fixture(user: &str) -> Fixture {
    let dir = tempdir::TempDir::new();
    let store = Store::open(dir.path().join("test.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: user.into(),
            email: user.into(),
            imap_host: HOST.into(),
            imap_port: PORT,
            imap_security: ImapSecurity::Plaintext,
            username: user.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();

    let config = ImapConfig {
        host: HOST.into(),
        port: PORT,
        security: ImapSecurity::Plaintext,
        username: user.into(),
    };
    let client = ImapClient::connect(&config, &auth()).await.unwrap();
    Fixture {
        store,
        account,
        blobs: Blobs::new(dir.path().join("blobs")),
        client,
        _dir: dir,
    }
}

#[tokio::test]
async fn the_gmail_layout_resolves_to_the_right_folders() {
    if !server_available() {
        return;
    }
    let mut fx = fixture("resolve@gmail.test").await;
    let folders = fx.client.folders().await.unwrap();
    let pairs: Vec<(&str, Option<&str>)> = folders
        .iter()
        .map(|f| (f.name.as_str(), f.special_use.as_deref()))
        .collect();

    // The whole point: no folder here is called Archive, and archiving still
    // has somewhere to go.
    assert!(!folders.iter().any(|f| f.name == "Archive"));
    assert_eq!(find_archive(pairs.iter().copied()), Some(ALL_MAIL));
    assert_eq!(find_trash(pairs.iter().copied()), Some("[Gmail]/Trash"));
    assert_eq!(
        find_sent(&folders).map(|f| f.name.as_str()),
        Some("[Gmail]/Sent Mail")
    );

    fx.client.logout().await.unwrap();
}

#[tokio::test]
async fn one_message_under_two_labels_is_stored_once() {
    // The invariant the whole store was designed around, in its native
    // habitat. Gmail delivers the same message under every label it carries,
    // so without Message-ID dedup every count the UI shows is inflated.
    if !server_available() {
        return;
    }
    let user = "labels@gmail.test";
    let message_id = "labelled-1@example.com";

    let mut writer = label::login(user).await;
    label::clear(&mut writer, "INBOX").await;
    label::clear(&mut writer, ALL_MAIL).await;
    label::put(&mut writer, "INBOX", message_id, "two labels").await;
    label::put(&mut writer, ALL_MAIL, message_id, "two labels").await;

    let mut fx = fixture(user).await;
    let report = core_proto::sync_account(&mut fx.client, &fx.store, &fx.blobs, fx.account)
        .await
        .unwrap();
    assert_eq!(report.inserted, 1, "{report:?}");
    assert_eq!(report.deduplicated, 1, "{report:?}");

    assert_eq!(fx.store.message_count(fx.account).unwrap(), 1);
    assert_eq!(fx.store.location_count(fx.account).unwrap(), 2);

    let message = fx
        .store
        .message_by_rfc822_id(fx.account, message_id)
        .unwrap()
        .unwrap();
    let mut folders: Vec<String> = fx
        .store
        .locations_of(message.id)
        .unwrap()
        .into_iter()
        .map(|l| fx.store.folder(l.folder_id).unwrap().unwrap().name)
        .collect();
    folders.sort();
    assert_eq!(folders, vec!["INBOX", ALL_MAIL]);

    // And exactly one body on disk, because the blob is keyed by the same
    // identity the store dedups on.
    let blobs = std::fs::read_dir(fx.blobs.root())
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten())
                .filter_map(|e| e.ok())
                .count()
        })
        .unwrap_or(0);
    assert_eq!(blobs, 1, "one message should keep one body");

    fx.client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn archiving_a_labelled_message_keeps_it_and_its_history() {
    // Archiving on Gmail removes the INBOX label; the copy in All Mail stays.
    // The message is therefore never in no folder at all, which is the case
    // that used to destroy the row mid-sync. Worth pinning here as well as on
    // the plain server, because this is the layout it will actually meet.
    if !server_available() {
        return;
    }
    let user = "archive@gmail.test";
    let message_id = "archive-me@example.com";

    let mut writer = label::login(user).await;
    label::clear(&mut writer, "INBOX").await;
    label::clear(&mut writer, ALL_MAIL).await;
    label::put(&mut writer, "INBOX", message_id, "archive me").await;
    label::put(&mut writer, ALL_MAIL, message_id, "archive me").await;

    let mut fx = fixture(user).await;
    core_proto::sync_account(&mut fx.client, &fx.store, &fx.blobs, fx.account)
        .await
        .unwrap();

    let before = fx
        .store
        .message_by_rfc822_id(fx.account, message_id)
        .unwrap()
        .unwrap();
    fx.store
        .record_correction(before.id, Some("notification"), "transactional")
        .unwrap();

    // Queue the archive exactly as the CLI would: from the INBOX copy, into
    // the folder the resolver picked.
    let folders = fx.store.folders(fx.account).unwrap();
    let inbox = folders.iter().find(|f| f.name == "INBOX").unwrap();
    let location = fx
        .store
        .locations_of(before.id)
        .unwrap()
        .into_iter()
        .find(|l| l.folder_id == inbox.id)
        .expect("a copy in INBOX");

    fx.store
        .enqueue_operation(&core_store::model::NewOperation {
            account_id: fx.account,
            message_id: before.id,
            kind: core_store::model::OperationKind::Move {
                target_folder: ALL_MAIL.into(),
            },
            source_folder_id: inbox.id,
            source_uid: location.uid,
            source_uid_validity: inbox.uid_validity,
            expect_message_id: Some(message_id.into()),
            execute_after: 0,
        })
        .unwrap();

    let flushed = core_proto::flush_operations(&mut fx.client, &fx.store, fx.account, 0)
        .await
        .unwrap();
    assert_eq!(flushed.applied, 1, "{flushed:?}");

    core_proto::sync_account(&mut fx.client, &fx.store, &fx.blobs, fx.account)
        .await
        .unwrap();

    let after = fx
        .store
        .message_by_rfc822_id(fx.account, message_id)
        .unwrap()
        .expect("the message should still be in the store");
    assert_eq!(after.id, before.id, "the row was rebuilt on the way");

    let corrections: i64 = fx
        .store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM correction WHERE message_id = ?1",
            [before.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(corrections, 1, "the correction was lost with the row");

    // Out of the inbox, still in All Mail — which is what archiving means here.
    let names: Vec<String> = fx
        .store
        .locations_of(after.id)
        .unwrap()
        .into_iter()
        .map(|l| fx.store.folder(l.folder_id).unwrap().unwrap().name)
        .collect();
    assert!(!names.contains(&"INBOX".to_string()), "got {names:?}");
    assert!(names.contains(&ALL_MAIL.to_string()), "got {names:?}");

    fx.client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn excluding_all_mail_fetches_the_mailbox_once_instead_of_twice() {
    // The reason folder exclusion exists. Gmail's All Mail holds a copy of
    // every message, so a mailbox whose mail carries two labels crosses the
    // wire twice over. Deduplication already keeps one row and one body — this
    // is about the bytes, which dedup cannot give back.
    if !server_available() {
        return;
    }
    let user = "exclude@gmail.test";

    let mut writer = label::login(user).await;
    label::clear(&mut writer, "INBOX").await;
    label::clear(&mut writer, ALL_MAIL).await;
    for n in 1..=3 {
        let id = format!("excluded-{n}@example.com");
        label::put(&mut writer, "INBOX", &id, &format!("msg {n}")).await;
        label::put(&mut writer, ALL_MAIL, &id, &format!("msg {n}")).await;
    }

    // Without the exclusion: every message is fetched twice, and the second
    // copy is thrown away as a duplicate.
    let mut plain = fixture(user).await;
    let report =
        core_proto::sync_account(&mut plain.client, &plain.store, &plain.blobs, plain.account)
            .await
            .unwrap();
    assert_eq!(report.inserted, 3, "{report:?}");
    assert_eq!(
        report.deduplicated, 3,
        "the wasted half of the download: {report:?}"
    );
    assert_eq!(report.folders_excluded, 0);
    plain.client.logout().await.unwrap();

    // With it: fetched once, and nothing is discarded.
    let mut trimmed = fixture(user).await;
    trimmed
        .store
        .exclude_folder(trimmed.account, "\\All")
        .unwrap();
    let report = core_proto::sync_account(
        &mut trimmed.client,
        &trimmed.store,
        &trimmed.blobs,
        trimmed.account,
    )
    .await
    .unwrap();
    assert_eq!(report.inserted, 3, "{report:?}");
    assert_eq!(
        report.deduplicated, 0,
        "nothing should be fetched twice: {report:?}"
    );
    assert_eq!(report.folders_excluded, 1, "{report:?}");

    // Same mail either way; one location each rather than two.
    assert_eq!(trimmed.store.message_count(trimmed.account).unwrap(), 3);
    assert_eq!(trimmed.store.location_count(trimmed.account).unwrap(), 3);

    // And the excluded folder is not even recorded, so nothing later mistakes
    // it for a folder that happens to be empty.
    let names: Vec<String> = trimmed
        .store
        .folders(trimmed.account)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert!(!names.contains(&ALL_MAIL.to_string()), "got {names:?}");

    trimmed.client.logout().await.unwrap();
    writer.logout().await.ok();
}
