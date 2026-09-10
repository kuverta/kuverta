//! End-to-end sync against the Dovecot dev server from `docker/`.
//!
//! Start it with `make dev-up` (or `docker compose -f docker/docker-compose.yml up -d`).
//!
//! When the server is not reachable these tests skip rather than fail, so
//! `cargo test` stays useful without Docker. CI sets
//! `FUCKMAIL_REQUIRE_DEV_SERVER=1`, which turns a missing server into a
//! failure — otherwise a broken compose file would look like a green build.

use core_accounts::EnvPassword;
use core_proto::{ImapClient, ImapConfig};
use core_store::model::{ImapSecurity, NewAccount};
use core_store::{Blobs, Store};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 10143;
const USER: &str = "dev@fuckmail.test";

/// Returns false (and explains) when the dev server is not up.
fn dev_server_available() -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{PORT}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();

    if !reachable {
        if std::env::var_os("FUCKMAIL_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev IMAP server on {HOST}:{PORT} is required but not reachable");
        }
        eprintln!("skipping: dev IMAP server not running (`make dev-up`)");
    }
    reachable
}

fn config() -> ImapConfig {
    ImapConfig {
        host: HOST.into(),
        port: PORT,
        security: ImapSecurity::Plaintext,
        username: USER.into(),
    }
}

fn auth() -> EnvPassword {
    // SAFETY: set before any thread reads it; the value is the dev password
    // baked into docker-compose.yml, not a real credential.
    unsafe { std::env::set_var("FUCKMAIL_DEV_PASSWORD", "devpass") };
    EnvPassword::new("FUCKMAIL_DEV_PASSWORD")
}

fn fixture_store() -> (Store, i64, Blobs, tempdir::TempDir) {
    let dir = tempdir::TempDir::new();
    let store = Store::open(dir.path().join("test.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: USER.into(),
            imap_host: HOST.into(),
            imap_port: PORT,
            imap_security: ImapSecurity::Plaintext,
            username: USER.into(),
            auth_method: "app_password".into(),
        })
        .unwrap();
    let blobs = Blobs::new(dir.path().join("blobs"));
    (store, account, blobs, dir)
}

#[tokio::test]
async fn connects_and_lists_the_seeded_folders() {
    if !dev_server_available() {
        return;
    }

    let mut client = ImapClient::connect(&config(), &auth()).await.unwrap();
    assert!(
        client.condstore_enabled(),
        "Dovecot should accept ENABLE CONDSTORE"
    );

    let folders = client.folders().await.unwrap();
    let names: Vec<_> = folders.iter().map(|f| f.name.as_str()).collect();

    assert!(names.contains(&"INBOX"), "got {names:?}");
    assert!(names.contains(&"Archive"), "got {names:?}");

    let archive = folders.iter().find(|f| f.name == "Archive").unwrap();
    assert_eq!(archive.special_use.as_deref(), Some("\\Archive"));

    client.logout().await.unwrap();
}

#[tokio::test]
async fn examine_reports_condstore_sync_state() {
    if !dev_server_available() {
        return;
    }

    let mut client = ImapClient::connect(&config(), &auth()).await.unwrap();
    let state = client.examine("INBOX").await.unwrap();

    assert!(state.uid_validity.is_some());
    assert!(state.uid_next.is_some());
    assert!(
        state.highest_modseq.is_some(),
        "CONDSTORE should supply HIGHESTMODSEQ"
    );
    assert_eq!(state.exists, 7, "the inbox fixtures");

    client.logout().await.unwrap();
}

#[tokio::test]
async fn full_sync_deduplicates_the_message_seen_in_two_folders() {
    if !dev_server_available() {
        return;
    }

    let (store, account, blobs, _dir) = fixture_store();
    let mut client = ImapClient::connect(&config(), &auth()).await.unwrap();

    let report = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    // 9 fixtures are seeded, but the newsletter appears in both INBOX and
    // Archive under one Message-ID, so it must land as 8 messages / 9 locations.
    assert_eq!(report.inserted, 8, "{report:?}");
    assert_eq!(report.deduplicated, 1, "{report:?}");
    assert_eq!(report.unparseable, 0, "{report:?}");
    assert_eq!(store.message_count(account).unwrap(), 8);
    assert_eq!(store.location_count(account).unwrap(), 9);

    client.logout().await.unwrap();
}

#[tokio::test]
async fn a_second_sync_fetches_nothing() {
    if !dev_server_available() {
        return;
    }

    // Guards the `N:*` quirk: a UID range starting past the end returns the
    // *last* message rather than nothing, so a naive implementation re-fetches
    // the newest mail on every single sync.
    let (store, account, blobs, _dir) = fixture_store();
    let mut client = ImapClient::connect(&config(), &auth()).await.unwrap();

    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    let second = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    assert_eq!(second.inserted, 0, "{second:?}");
    assert_eq!(second.deduplicated, 0, "{second:?}");
    assert_eq!(store.message_count(account).unwrap(), 8);

    client.logout().await.unwrap();
}

#[tokio::test]
async fn sync_decodes_headers_and_populates_search() {
    if !dev_server_available() {
        return;
    }

    let (store, account, blobs, _dir) = fixture_store();
    let mut client = ImapClient::connect(&config(), &auth()).await.unwrap();
    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    client.logout().await.unwrap();

    // RFC 2047 encoded German subject, decoded on the way in.
    let hits = store.search(account, "Abschlagszahlung", 10).unwrap();
    assert_eq!(hits.len(), 1, "encoded subject should be searchable");
    assert_eq!(
        hits[0].subject.as_deref(),
        Some("Ihre Abschlagszahlung für März wurde geändert")
    );

    // List-Id is extracted as a column for the rules baseline.
    let recent = store.recent(account, 50).unwrap();
    assert!(
        recent
            .iter()
            .any(|m| m.list_id.as_deref() == Some("news.rustweekly.example")),
        "List-Id should be stored"
    );
    assert!(
        recent.iter().any(|m| m.has_attachments),
        "the invoice fixture has a PDF attached"
    );
}

/// Minimal scratch directory helper; not worth a dependency.
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    pub struct TempDir(PathBuf);

    impl TempDir {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("fuckmail-it-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Incremental sync
//
// These use their own IMAP users. Dovecot's dev passdb accepts any username, so
// each test gets an empty mailbox of its own and cannot disturb the seeded
// fixtures — or another test running in parallel.
// ---------------------------------------------------------------------------

mod mutate {
    //! Test-only write access to the dev server.

    use futures::{StreamExt, TryStreamExt};
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

    /// Empties INBOX so a re-run starts from a known state.
    pub async fn reset_inbox(session: &mut Session) {
        session.select("INBOX").await.unwrap();
        let uids = session.uid_search("ALL").await.unwrap();
        if !uids.is_empty() {
            let set = uids
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");
            drain(session.uid_store(set, "+FLAGS (\\Deleted)").await.unwrap()).await;
            session.expunge().await.unwrap().collect::<Vec<_>>().await;
        }
    }

    pub async fn append(session: &mut Session, message_id: &str, subject: &str) {
        let raw = format!(
            "Message-ID: <{message_id}>\r\n\
             From: Test <test@example.com>\r\n\
             Subject: {subject}\r\n\
             Date: Mon, 07 Sep 2026 03:00:00 +0200\r\n\
             \r\n\
             body\r\n"
        );
        session
            .append("INBOX", None, None, raw.as_bytes())
            .await
            .unwrap();
    }

    pub async fn set_flag(session: &mut Session, uid: u32, flag: &str) {
        session.select("INBOX").await.unwrap();
        drain(
            session
                .uid_store(uid.to_string(), format!("+FLAGS ({flag})"))
                .await
                .unwrap(),
        )
        .await;
    }

    pub async fn expunge_uid(session: &mut Session, uid: u32) {
        set_flag(session, uid, "\\Deleted").await;
        session.expunge().await.unwrap().collect::<Vec<_>>().await;
    }

    pub async fn uids(session: &mut Session) -> Vec<u32> {
        session.select("INBOX").await.unwrap();
        let mut uids: Vec<u32> = session
            .uid_search("ALL")
            .await
            .unwrap()
            .into_iter()
            .collect();
        uids.sort_unstable();
        uids
    }

    /// Commands that return a stream must have it consumed before the session
    /// can be used again.
    async fn drain<S, T>(stream: S)
    where
        S: futures::Stream<Item = async_imap::error::Result<T>>,
    {
        let _ = stream.try_collect::<Vec<_>>().await;
    }
}

/// Builds a store plus a connected read-only client for a dedicated test user.
async fn isolated(user: &str) -> (Store, i64, Blobs, tempdir::TempDir, ImapClient) {
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
        })
        .unwrap();
    let blobs = Blobs::new(dir.path().join("blobs"));

    let config = ImapConfig {
        host: HOST.into(),
        port: PORT,
        security: ImapSecurity::Plaintext,
        username: user.into(),
    };
    let client = ImapClient::connect(&config, &auth()).await.unwrap();
    (store, account, blobs, dir, client)
}

#[tokio::test]
async fn an_unchanged_folder_is_skipped_entirely() {
    if !dev_server_available() {
        return;
    }
    let user = "skip-test@fuckmail.test";

    let mut writer = mutate::login(user).await;
    mutate::reset_inbox(&mut writer).await;
    mutate::append(&mut writer, "skip-1@example.com", "one").await;

    let (store, account, blobs, _dir, mut client) = isolated(user).await;
    let first = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(first.inserted, 1, "{first:?}");

    // Nothing has changed, so every folder's HIGHESTMODSEQ matches what was
    // stored and none of them should be fetched from at all.
    let second = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(second.folders_synced, 0, "{second:?}");
    assert!(second.folders_skipped >= 1, "{second:?}");
    assert_eq!(second.inserted, 0, "{second:?}");

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_flag_set_on_the_server_reaches_the_store() {
    if !dev_server_available() {
        return;
    }
    let user = "flag-test@fuckmail.test";

    let mut writer = mutate::login(user).await;
    mutate::reset_inbox(&mut writer).await;
    mutate::append(&mut writer, "flag-1@example.com", "unread message").await;

    let (store, account, blobs, _dir, mut client) = isolated(user).await;
    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    let message = store.recent(account, 1).unwrap().pop().unwrap();
    assert!(
        !store.locations_of(message.id).unwrap()[0]
            .flags
            .contains("\\Seen"),
        "should start unread"
    );

    let uid = mutate::uids(&mut writer).await[0];
    mutate::set_flag(&mut writer, uid, "\\Seen").await;

    let report = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(report.flag_updates, 1, "{report:?}");
    assert_eq!(report.inserted, 0, "a flag change must not re-insert");

    let flags = &store.locations_of(message.id).unwrap()[0].flags;
    assert!(flags.contains("\\Seen"), "got {flags:?}");

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_message_expunged_on_the_server_is_removed_locally() {
    if !dev_server_available() {
        return;
    }
    let user = "expunge-test@fuckmail.test";

    let mut writer = mutate::login(user).await;
    mutate::reset_inbox(&mut writer).await;
    mutate::append(&mut writer, "keep-1@example.com", "keep me").await;
    mutate::append(&mut writer, "drop-1@example.com", "delete me").await;

    let (store, account, blobs, _dir, mut client) = isolated(user).await;
    let first = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(first.inserted, 2, "{first:?}");

    let uids = mutate::uids(&mut writer).await;
    mutate::expunge_uid(&mut writer, uids[1]).await;

    let report = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(report.expunged, 1, "{report:?}");
    assert_eq!(store.message_count(account).unwrap(), 1);

    let remaining = store.recent(account, 10).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].subject.as_deref(), Some("keep me"));

    client.logout().await.unwrap();
    writer.logout().await.ok();
}
