//! Folders, sending later and unsubscribing, against the dev servers from
//! `docker/`.
//!
//! The unit tests check what happens before a connection. These check what
//! only a server can say: that a folder made here exists there, that one
//! holding mail is not deleted, that a scheduled message really goes, and that
//! an unsubscribe by mail is sent.
//!
//! Skips when the servers are down; `KUVERTA_REQUIRE_DEV_SERVER=1` makes that a
//! failure, as in `core-proto/tests/dev_server.rs`.

use core_accounts::EnvPassword;
use core_rpc::{DraftInput, Session};
use core_store::model::{ImapSecurity, NewAccount, SmtpConfig, SmtpSecurity};
use core_store::Store;

const HOST: &str = "127.0.0.1";
const IMAP_PORT: u16 = 10143;
const SINK_PORT: u16 = 1025;
const PASSWORD_VAR: &str = "KUVERTA_DEV_PASSWORD";

fn available(port: u16) -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{port}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();
    if !reachable {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev server on {HOST}:{port} is required but not reachable");
        }
        eprintln!("skipping: dev server on {port} not running (`make dev-up`)");
    }
    reachable
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kuverta-features-dev-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> Store {
        Store::open(self.0.join("kuverta.db")).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh mailbox per run: the dev server accepts any user, and what these
/// tests create would otherwise pile up across runs.
fn unique_user(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("features-{tag}-{nanos}@kuverta.test")
}

fn session_with_account(dir: &TempDir, user: &str, can_send: bool) -> Session {
    // SAFETY: the dev password baked into docker-compose.yml, not a real
    // credential; every test sets the same value.
    unsafe { std::env::set_var(PASSWORD_VAR, "devpass") };
    let store = dir.store();
    let id = store
        .add_account(&NewAccount {
            label: user.into(),
            email: user.into(),
            imap_host: HOST.into(),
            imap_port: IMAP_PORT,
            imap_security: ImapSecurity::Plaintext,
            username: user.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    if can_send {
        store
            .set_smtp(
                id,
                Some(&SmtpConfig {
                    host: HOST.into(),
                    port: SINK_PORT,
                    security: SmtpSecurity::Plaintext,
                }),
            )
            .unwrap();
    }
    Session::new(dir.0.clone()).with_password_env(Some(PASSWORD_VAR.into()))
}

/// Puts a message straight into a folder, the way another client would.
async fn append(user: &str, folder: &str, raw: &str) {
    let config = core_proto::ImapConfig {
        host: HOST.into(),
        port: IMAP_PORT,
        security: ImapSecurity::Plaintext,
        username: user.into(),
    };
    let auth = EnvPassword::new(PASSWORD_VAR);
    let mut client = core_proto::ImapClient::connect(&config, &auth)
        .await
        .unwrap();
    // The dev server makes a user's INBOX on first login; other folders have
    // to be asked for.
    if folder != "INBOX" {
        let _ = client.create_folder(folder, None).await;
    }
    client
        .append(folder, &[], raw.replace('\n', "\r\n").as_bytes())
        .await
        .unwrap();
    client.logout().await.ok();
}

fn folder_names(dir: &TempDir, user: &str) -> Vec<String> {
    let store = dir.store();
    let account = store.account_by_email(user).unwrap().unwrap().id;
    store
        .folders(account)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect()
}

#[tokio::test]
async fn a_folder_is_made_on_the_server_and_deleted_only_when_empty() {
    if !available(IMAP_PORT) {
        return;
    }
    let dir = TempDir::new("folders");
    let user = unique_user("folders");
    let session = session_with_account(&dir, &user, false);
    session.sync_account(&user).await.unwrap();

    let created = session
        .create_folder(&user, "Rechnungen", None)
        .await
        .unwrap();
    assert!(folder_names(&dir, &user).contains(&"Rechnungen".to_string()));
    let again = session.create_folder(&user, "Rechnungen", None).await;
    assert!(again.is_err(), "the same name twice is refused");

    // One inside it, named with the server's own separator.
    let inner = session
        .create_folder(&user, "2026", Some("Rechnungen"))
        .await
        .unwrap();
    assert!(inner.name.starts_with("Rechnungen") && inner.name.ends_with("2026"));

    // A folder with one inside it is not deleted.
    let refused = session.delete_folder(&user, created.id).await;
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("folders inside it"),
        "a parent is refused"
    );

    // Nor is one holding mail — as the server says, not the store, which has
    // not synced it yet.
    append(
        &user,
        &inner.name,
        "Message-ID: <held@example>\nSubject: kept\n\nbody\n",
    )
    .await;
    let refused = session.delete_folder(&user, inner.id).await;
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("still holds 1 message"),
        "a folder with mail is refused"
    );

    // Nor the Inbox.
    let store = dir.store();
    let account = store.account_by_email(&user).unwrap().unwrap().id;
    let inbox = store
        .folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "INBOX")
        .unwrap();
    assert!(session.delete_folder(&user, inbox.id).await.is_err());

    // An empty folder is.
    let empty = session.create_folder(&user, "Leer", None).await.unwrap();
    assert_eq!(
        session.delete_folder(&user, empty.id).await.unwrap(),
        "Leer"
    );
    session.sync_account(&user).await.unwrap();
    assert!(!folder_names(&dir, &user).contains(&"Leer".to_string()));
}

#[tokio::test]
async fn a_scheduled_message_goes_when_it_is_due_and_only_once() {
    if !available(IMAP_PORT) || !available(SINK_PORT) {
        return;
    }
    let dir = TempDir::new("outbox");
    let user = unique_user("outbox");
    let session = session_with_account(&dir, &user, true);
    session.sync_account(&user).await.unwrap();

    let draft = DraftInput {
        to: vec!["erika@example.de".into()],
        subject: "Später gesendet".into(),
        body: "Hallo Erika,\n\nwie besprochen.\n".into(),
        ..Default::default()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // A bad address is refused when it is scheduled, not when it is due.
    let bad = DraftInput {
        to: vec!["not an address".into()],
        ..draft.clone()
    };
    assert!(session.schedule(&user, &bad, now + 60).is_err());

    let later = session.schedule(&user, &draft, now + 3600).unwrap();
    let due = session.schedule(&user, &draft, now - 1).unwrap();

    let sent = session.send_due().await.unwrap();
    assert_eq!(sent.len(), 1, "only the one that is due");
    assert_eq!(sent[0].id, due);
    assert!(sent[0].error.is_none(), "{:?}", sent[0].error);
    assert!(session.send_due().await.unwrap().is_empty(), "never twice");

    let store = dir.store();
    assert_eq!(store.outbox_entry(due).unwrap().unwrap().state, "sent");
    assert_eq!(
        store.outbox_entry(later).unwrap().unwrap().state,
        "scheduled"
    );
}

#[tokio::test]
async fn unsubscribing_by_mail_sends_the_mail_the_sender_asked_for() {
    if !available(IMAP_PORT) || !available(SINK_PORT) {
        return;
    }
    let dir = TempDir::new("unsubscribe");
    let user = unique_user("unsubscribe");
    let session = session_with_account(&dir, &user, true);
    append(
        &user,
        "INBOX",
        "Message-ID: <issue-1@list.example>\n\
         From: Weekly <digest@list.example>\n\
         List-Id: Weekly <weekly.list.example>\n\
         List-Unsubscribe: <mailto:leave@list.example?subject=unsubscribe%20me>\n\
         Subject: Issue 1\n\
         \n\
         Read all about it.\n",
    )
    .await;
    session.sync_account(&user).await.unwrap();

    let results = session
        .unsubscribe(&user, &["weekly.list.example".to_string()])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].state, "done", "{:?}", results[0].detail);

    let store = dir.store();
    let account = store.account_by_email(&user).unwrap().unwrap().id;
    let recorded = store.unsubscriptions(account).unwrap();
    assert_eq!(recorded[0].method, "mailto");
    assert_eq!(recorded[0].target, "mailto:leave@list.example");
}
