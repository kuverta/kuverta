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
            ..Default::default()
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
        reset_folder(session, "INBOX").await;
    }

    /// Empties one folder.
    ///
    /// Mutation tests move mail *out* of INBOX, so resetting INBOX alone is
    /// not enough to make them repeatable: the copy left in the destination
    /// would still be there on the next run, and a queued operation would pick
    /// up that stale location instead of the fresh one.
    pub async fn reset_folder(session: &mut Session, folder: &str) {
        session.select(folder).await.unwrap();
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

    /// Appends a message with no `Message-ID` header — unusual but legal, and
    /// the case that made the executor's identity check ambiguous.
    pub async fn append_without_message_id(session: &mut Session, subject: &str) {
        let raw = format!(
            "From: Test <test@example.com>\r\n\
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
            ..Default::default()
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

#[tokio::test]
async fn a_sent_message_is_appended_to_the_sent_folder() {
    // Stage 1 of the write capability (plan section 1a): the only write this
    // client performs. It has to work against a real server, and it has to
    // leave the read-only property intact — nothing here selects a folder.
    if !dev_server_available() {
        return;
    }
    let user = "append-test@fuckmail.test";
    let (store, account, blobs, _dir, mut client) = isolated(user).await;

    let folders = client.folders().await.unwrap();
    let sent = core_proto::client::find_sent(&folders).expect("dev server declares \\Sent");
    assert_eq!(sent.name, "Sent");
    let sent_name = sent.name.clone();

    // Unique per run: APPEND is additive and this test does not tear its
    // mailbox down, so a fixed id would either accumulate rows or collapse
    // into one deduplicated message and make the assertions meaningless.
    let message_id = format!(
        "appended-{}@fuckmail.test",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let raw = format!(
        "Message-ID: <{message_id}>\r\n\
         From: Erika <{user}>\r\n\
         To: Jane <jane@example.com>\r\n\
         Subject: filed under sent\r\n\
         Date: Thu, 10 Sep 2026 12:00:00 +0000\r\n\
         \r\n\
         body\r\n"
    );

    client
        .append(&sent_name, &["\\Seen"], raw.as_bytes())
        .await
        .expect("append");

    // Read it back the way the next sync will, rather than trusting the OK.
    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    let stored = store
        .message_by_rfc822_id(account, &message_id)
        .unwrap()
        .expect("the appended message should sync back");
    assert_eq!(stored.subject.as_deref(), Some("filed under sent"));

    // \Seen matters: without it every client shows Sent as full of unread mail.
    let locations = store.locations_of(stored.id).unwrap();
    assert_eq!(locations.len(), 1);
    assert!(
        locations[0].flags.contains("\\Seen"),
        "flags were {:?}",
        locations[0].flags
    );

    client.logout().await.unwrap();
}

// -- the mutation queue (plan section 1a, stage 2) -------------------------

use core_store::model::{NewOperation, OperationKind};

/// Puts a single message in INBOX with every folder a mutation test might
/// move it to emptied first, so the run starts from a known state.
async fn seeded_inbox(user: &str, message_id: &str, subject: &str) -> mutate::Session {
    let mut writer = mutate::login(user).await;
    for folder in ["INBOX", "Archive", "Trash"] {
        mutate::reset_folder(&mut writer, folder).await;
    }
    mutate::append(&mut writer, message_id, subject).await;
    writer
}

/// Queues an operation on the message with `message_id`, as the CLI would.
fn enqueue(
    store: &Store,
    account: i64,
    message_id: &str,
    kind: OperationKind,
    expect: Option<&str>,
) -> i64 {
    let message = store
        .message_by_rfc822_id(account, message_id)
        .unwrap()
        .expect("message should be in the store");
    let location = store.locations_of(message.id).unwrap().remove(0);
    let folder = store.folder(location.folder_id).unwrap().unwrap();

    store
        .enqueue_operation(&NewOperation {
            account_id: account,
            message_id: message.id,
            kind,
            source_folder_id: location.folder_id,
            source_uid: location.uid,
            source_uid_validity: folder.uid_validity,
            expect_message_id: expect.map(str::to_string),
            // Due immediately: the undo window is the CLI's concern, and a
            // test that slept through it would only be slower.
            execute_after: 0,
        })
        .unwrap()
}

fn state_of(store: &Store, op: i64) -> (String, Option<String>) {
    store
        .connection()
        .query_row(
            "SELECT state, last_error FROM operation WHERE id = ?1",
            [op],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

async fn synced(user: &str) -> (Store, i64, Blobs, tempdir::TempDir, ImapClient) {
    let (store, account, blobs, dir, mut client) = isolated(user).await;
    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    (store, account, blobs, dir, client)
}

#[tokio::test]
async fn a_queued_move_reaches_the_server_and_the_next_sync_sees_it() {
    if !dev_server_available() {
        return;
    }
    let user = "move-op@fuckmail.test";
    let message_id = "moveable-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "archive me").await;

    let (store, account, blobs, _dir, mut client) = synced(user).await;
    let op = enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Archive".into(),
        },
        Some(message_id),
    );

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(state_of(&store, op).0, "done");

    // The executor does not touch the local store; the sync that follows is
    // what makes the move visible, by observing the server.
    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    let message = store
        .message_by_rfc822_id(account, message_id)
        .unwrap()
        .unwrap();
    let locations = store.locations_of(message.id).unwrap();
    assert_eq!(locations.len(), 1, "{locations:?}");
    let folder = store.folder(locations[0].folder_id).unwrap().unwrap();
    assert_eq!(folder.name, "Archive");

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_queued_flag_change_reaches_the_server() {
    if !dev_server_available() {
        return;
    }
    let user = "flag-op@fuckmail.test";
    let message_id = "flaggable-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "mark me read").await;

    let (store, account, blobs, _dir, mut client) = synced(user).await;
    enqueue(
        &store,
        account,
        message_id,
        OperationKind::Flag {
            flag: "\\Seen".into(),
            set: true,
        },
        Some(message_id),
    );

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.applied, 1, "{report:?}");

    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    let message = store
        .message_by_rfc822_id(account, message_id)
        .unwrap()
        .unwrap();
    let locations = store.locations_of(message.id).unwrap();
    assert!(
        locations[0].flags.contains("\\Seen"),
        "flags were {:?}",
        locations[0].flags
    );

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn an_operation_is_refused_when_the_uid_no_longer_holds_the_expected_message() {
    // The check the whole executor exists for. If this regresses, fuckmail
    // archives whatever happens to sit at a stale UID — the exact failure the
    // read-only design used to make impossible.
    if !dev_server_available() {
        return;
    }
    let user = "conflict-op@fuckmail.test";
    let message_id = "conflicted-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "do not touch me").await;

    let (store, account, _blobs, _dir, mut client) = synced(user).await;
    // Queued expecting a message that is not the one at this UID, which is
    // what a stale queue entry looks like from the executor's side.
    let op = enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Archive".into(),
        },
        Some("something-else@example.com"),
    );

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.conflicted, 1, "{report:?}");
    assert_eq!(report.applied, 0);

    let (state, error) = state_of(&store, op);
    assert_eq!(state, "failed");
    assert!(
        error
            .as_deref()
            .unwrap()
            .contains("something-else@example.com"),
        "the reason should name what was expected, got {error:?}"
    );

    // And the message really was left where it was.
    let mut inbox = mutate::login(user).await;
    inbox.select("INBOX").await.unwrap();
    assert_eq!(inbox.uid_search("ALL").await.unwrap().len(), 1);

    client.logout().await.unwrap();
    inbox.logout().await.ok();
    writer.logout().await.ok();
}

#[tokio::test]
async fn an_operation_on_a_message_someone_else_removed_is_obsolete_not_an_error() {
    if !dev_server_available() {
        return;
    }
    let user = "obsolete-op@fuckmail.test";
    let message_id = "vanishing-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "gone by the time we get there").await;

    let (store, account, _blobs, _dir, mut client) = synced(user).await;
    let op = enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Archive".into(),
        },
        Some(message_id),
    );

    // Another client gets there first — the ordinary case of a phone and a
    // laptop touching the same mailbox.
    let uids = mutate::uids(&mut writer).await;
    mutate::expunge_uid(&mut writer, uids[0]).await;

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.obsolete, 1, "{report:?}");
    assert_eq!(report.conflicted, 0);
    assert_eq!(state_of(&store, op).0, "obsolete");

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_renumbered_folder_fails_its_operations_rather_than_acting_on_stale_uids() {
    if !dev_server_available() {
        return;
    }
    let user = "uidvalidity-op@fuckmail.test";
    let message_id = "renumbered-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "renumber me").await;

    let (store, account, _blobs, _dir, mut client) = synced(user).await;
    let message = store
        .message_by_rfc822_id(account, message_id)
        .unwrap()
        .unwrap();
    let location = store.locations_of(message.id).unwrap().remove(0);

    // A UIDVALIDITY that never was. Forcing the server to renumber is awkward
    // and slow; what matters is that a mismatch is caught and refused.
    let op = store
        .enqueue_operation(&NewOperation {
            account_id: account,
            message_id: message.id,
            kind: OperationKind::Move {
                target_folder: "Archive".into(),
            },
            source_folder_id: location.folder_id,
            source_uid: location.uid,
            source_uid_validity: Some(1),
            expect_message_id: Some(message_id.into()),
            execute_after: 0,
        })
        .unwrap();

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.conflicted, 1, "{report:?}");

    let (state, error) = state_of(&store, op);
    assert_eq!(state, "failed");
    assert!(
        error.as_deref().unwrap().contains("UIDVALIDITY"),
        "got {error:?}"
    );

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn an_operation_cancelled_before_the_flush_never_reaches_the_server() {
    if !dev_server_available() {
        return;
    }
    let user = "undo-op@fuckmail.test";
    let message_id = "undone-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "changed my mind").await;

    let (store, account, _blobs, _dir, mut client) = synced(user).await;
    let op = enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Trash".into(),
        },
        Some(message_id),
    );

    assert!(store.cancel_operation(op).unwrap());

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert!(report.is_empty(), "{report:?}");

    // Still in INBOX, and Trash never heard about it.
    let mut check = mutate::login(user).await;
    check.select("INBOX").await.unwrap();
    assert_eq!(check.uid_search("ALL").await.unwrap().len(), 1);
    check.select("Trash").await.unwrap();
    assert_eq!(check.uid_search("ALL").await.unwrap().len(), 0);

    client.logout().await.unwrap();
    check.logout().await.ok();
    writer.logout().await.ok();
}

#[tokio::test]
async fn operations_on_one_message_are_applied_in_the_order_they_were_made() {
    // A move followed by a flag change: if the flag went first against the
    // post-move state, or the two were reordered, the flag would be applied in
    // a folder the message had already left.
    if !dev_server_available() {
        return;
    }
    let user = "ordered-op@fuckmail.test";
    let message_id = "ordered-1@example.com";

    let mut writer = seeded_inbox(user, message_id, "flag then move").await;

    let (store, account, blobs, _dir, mut client) = synced(user).await;
    enqueue(
        &store,
        account,
        message_id,
        OperationKind::Flag {
            flag: "\\Seen".into(),
            set: true,
        },
        Some(message_id),
    );
    enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Archive".into(),
        },
        Some(message_id),
    );

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.applied, 2, "{report:?}");

    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();

    let message = store
        .message_by_rfc822_id(account, message_id)
        .unwrap()
        .unwrap();
    let locations = store.locations_of(message.id).unwrap();
    assert_eq!(locations.len(), 1, "{locations:?}");
    let folder = store.folder(locations[0].folder_id).unwrap().unwrap();
    assert_eq!(folder.name, "Archive");
    // The flag was set before the move, and MOVE preserves flags.
    assert!(
        locations[0].flags.contains("\\Seen"),
        "flags were {:?}",
        locations[0].flags
    );

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_message_without_a_message_id_can_still_be_filed() {
    // Caught by dogfooding, not by the tests above: a message with no
    // Message-ID header is legal and is in the fixtures, and reading "no
    // Message-ID came back" as "no message here" made such mail impossible to
    // move — silently, as an operation marked obsolete.
    if !dev_server_available() {
        return;
    }
    let user = "no-msgid-op@fuckmail.test";

    let mut writer = mutate::login(user).await;
    for folder in ["INBOX", "Archive", "Trash"] {
        mutate::reset_folder(&mut writer, folder).await;
    }
    mutate::append_without_message_id(&mut writer, "anonymous but filable").await;

    let (store, account, blobs, _dir, mut client) = synced(user).await;
    let stored = store.recent(account, 10).unwrap();
    assert_eq!(stored.len(), 1);
    let message = store.message_by_id(account, stored[0].id).unwrap().unwrap();
    assert!(
        message.rfc822_message_id.is_none(),
        "the fixture should have no Message-ID"
    );

    let location = store.locations_of(message.id).unwrap().remove(0);
    let folder = store.folder(location.folder_id).unwrap().unwrap();
    let op = store
        .enqueue_operation(&NewOperation {
            account_id: account,
            message_id: message.id,
            kind: OperationKind::Move {
                target_folder: "Archive".into(),
            },
            source_folder_id: location.folder_id,
            source_uid: location.uid,
            source_uid_validity: folder.uid_validity,
            // None here means "expect a message with no Message-ID", which is
            // exactly what is there.
            expect_message_id: None,
            execute_after: 0,
        })
        .unwrap();

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(state_of(&store, op).0, "done");

    core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    let locations = store.locations_of(message.id).unwrap();
    let folder = store.folder(locations[0].folder_id).unwrap().unwrap();
    assert_eq!(folder.name, "Archive");

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn expecting_no_message_id_and_finding_one_is_a_conflict() {
    // The other half of comparing absence: absence must not act as a wildcard,
    // or every message without an id would match every UID.
    if !dev_server_available() {
        return;
    }
    let user = "msgid-mismatch-op@fuckmail.test";
    let message_id = "has-an-id@example.com";
    let mut writer = seeded_inbox(user, message_id, "this one has an id").await;

    let (store, account, _blobs, _dir, mut client) = synced(user).await;
    let op = enqueue(
        &store,
        account,
        message_id,
        OperationKind::Move {
            target_folder: "Archive".into(),
        },
        None,
    );

    let report = core_proto::flush_operations(&mut client, &store, account, 0)
        .await
        .unwrap();
    assert_eq!(report.conflicted, 1, "{report:?}");

    let (state, error) = state_of(&store, op);
    assert_eq!(state, "failed");
    assert!(
        error.as_deref().unwrap().contains("no Message-ID"),
        "got {error:?}"
    );

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn a_sparse_uid_range_fetches_exactly_the_messages_that_remain() {
    // Batching turned the fetch from one `lo:*` range into an explicit UID
    // set, so gaps left by expunges now have to be carried through
    // uid_sizes -> plan_batches -> fetch_uids without dropping or inventing
    // anything.
    if !dev_server_available() {
        return;
    }
    let user = "sparse-fetch@fuckmail.test";

    let mut writer = mutate::login(user).await;
    mutate::reset_inbox(&mut writer).await;
    for n in 1..=5 {
        mutate::append(
            &mut writer,
            &format!("sparse-{n}@example.com"),
            &format!("msg {n}"),
        )
        .await;
    }

    // Punch holes before the first sync, so the client never sees the gaps
    // being made — it just meets a folder with non-contiguous UIDs.
    let uids = mutate::uids(&mut writer).await;
    mutate::expunge_uid(&mut writer, uids[1]).await;
    mutate::expunge_uid(&mut writer, uids[3]).await;

    let (store, account, blobs, _dir, mut client) = isolated(user).await;
    let report = core_proto::sync_account(&mut client, &store, &blobs, account)
        .await
        .unwrap();
    assert_eq!(report.inserted, 3, "{report:?}");

    let mut subjects: Vec<String> = store
        .recent(account, 10)
        .unwrap()
        .into_iter()
        .filter_map(|m| m.subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, vec!["msg 1", "msg 3", "msg 5"]);

    client.logout().await.unwrap();
    writer.logout().await.ok();
}

#[tokio::test]
async fn asking_for_uids_past_the_end_of_a_folder_returns_nothing() {
    // The `lo:*` quirk: `*` is the highest existing UID, so a `lo` past the end
    // makes the server return the last message rather than nothing. Left
    // unhandled, every sync would re-fetch the newest mail forever.
    if !dev_server_available() {
        return;
    }
    let user = "star-quirk@fuckmail.test";

    let mut writer = mutate::login(user).await;
    mutate::reset_inbox(&mut writer).await;
    mutate::append(&mut writer, "only-one@example.com", "the only message").await;

    let (_store, _account, _blobs, _dir, mut client) = isolated(user).await;
    let state = client.examine("INBOX").await.unwrap();
    let highest = state.uid_next.unwrap() - 1;

    assert_eq!(client.uid_sizes(highest).await.unwrap().len(), 1);
    assert!(
        client.uid_sizes(highest + 1).await.unwrap().is_empty(),
        "a UID past the end must fetch nothing, not the last message"
    );

    client.logout().await.unwrap();
    writer.logout().await.ok();
}
