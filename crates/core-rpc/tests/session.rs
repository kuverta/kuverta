//! The orchestration both the CLI and the window run.
//!
//! Worth its own test because it is the one place the two halves meet: queued
//! changes go out, then the sync pass observes what the server actually did.
//! Getting that order wrong is invisible in a unit test and obvious here.

use core_rpc::{Core, Session};
use core_store::model::*;
use core_store::{Blobs, Store};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 10143;

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

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("fuckmail-session-{}-{name}", std::process::id()));
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

/// A data directory with one account pointed at the dev server.
fn registered(name: &str, user: &str) -> TempDir {
    // SAFETY: set before any thread reads it; this is the dev password from
    // docker-compose.yml, not a credential.
    unsafe { std::env::set_var("FUCKMAIL_SESSION_PASSWORD", "devpass") };

    let dir = TempDir::new(name);
    let store = Store::open(dir.0.join("fuckmail.db")).unwrap();
    store
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
    dir
}

fn session(dir: &TempDir) -> Session {
    Session::new(&dir.0).with_password_env(Some("FUCKMAIL_SESSION_PASSWORD".into()))
}

fn core(dir: &TempDir) -> Core {
    Core::new(
        Store::open(dir.0.join("fuckmail.db")).unwrap(),
        Blobs::new(dir.0.join("blobs")),
    )
}

#[tokio::test]
async fn a_sync_fills_the_store_and_the_second_one_finds_nothing_to_do() {
    if !dev_server_available() {
        return;
    }
    let user = "dev@fuckmail.test";
    let dir = registered("basic", user);
    let session = session(&dir);

    let first = session.sync_account(user).await.unwrap();
    assert_eq!(first.email, user);
    assert!(first.inserted > 0, "{first:?}");
    assert!(first.folders_synced > 0, "{first:?}");

    // CONDSTORE means an unchanged mailbox costs one EXAMINE per folder and no
    // fetches at all.
    let second = session.sync_account(user).await.unwrap();
    assert_eq!(second.inserted, 0, "{second:?}");
    assert_eq!(second.folders_synced, 0, "{second:?}");
    assert!(second.folders_skipped > 0, "{second:?}");
}

#[tokio::test]
async fn a_queued_change_goes_out_and_the_same_pass_shows_the_result() {
    // The order the whole facade exists to get right. If the sync pass ran
    // first, the change would sit unsent until the next one and the window
    // would show a row it had just been told to archive.
    if !dev_server_available() {
        return;
    }
    let user = "session-order@fuckmail.test";
    let dir = registered("order", user);
    let session = session(&dir);

    // A mailbox of its own, seeded through the same path the app uses.
    session.sync_account(user).await.unwrap();

    let core = core(&dir);
    let account = core.accounts().unwrap()[0].id;
    let before = core
        .messages(account, 0, 50, &ListFilter::default())
        .unwrap();
    if before.total == 0 {
        eprintln!("skipping: this dev user has no mail to move");
        return;
    }

    let target = "Archive";
    let row = &before.rows[0];
    core.move_to(account, row.id, target, 0).unwrap();
    assert_eq!(core.queue(account).unwrap().len(), 1);

    let summary = session.sync_account(user).await.unwrap();
    assert_eq!(summary.changes_sent, 1, "{summary:?}");
    assert_eq!(summary.changes_refused, 0, "{summary:?}");

    // The queue is empty and the message has moved — both visible from the
    // same connection the window reads through, not the one the sync wrote on.
    assert!(core.queue(account).unwrap().is_empty());
    let detail = core.message(account, row.id).unwrap();
    assert_eq!(detail.folders, vec![target], "{:?}", detail.folders);
}

#[tokio::test]
async fn an_unknown_account_is_named_in_the_error() {
    if !dev_server_available() {
        return;
    }
    let dir = registered("unknown", "dev@fuckmail.test");
    let err = session(&dir)
        .sync_account("nobody@example.com")
        .await
        .expect_err("there is no such account");
    assert!(err.to_string().contains("nobody@example.com"), "{err}");
}

#[tokio::test]
async fn a_window_reading_while_a_sync_writes_is_not_blocked() {
    // The reason a sync opens its own connection. WAL is what makes this safe;
    // without it the reader would be locked out for the length of the sync,
    // which is exactly as long as the window would be frozen.
    if !dev_server_available() {
        return;
    }
    let user = "session-concurrent@fuckmail.test";
    let dir = registered("concurrent", user);
    let session = session(&dir);
    let core = core(&dir);

    let syncing = session.sync_account(user);
    let reading = async {
        let mut reads = 0;
        for _ in 0..50 {
            // Whatever the sync is doing, the list keeps answering.
            core.accounts().expect("a read during a sync must not fail");
            reads += 1;
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        reads
    };

    let (summary, reads) = tokio::join!(syncing, reading);
    summary.unwrap();
    assert_eq!(reads, 50);
}

// -- sending ---------------------------------------------------------------

const SINK_PORT: u16 = 1025;

fn sink_available() -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{SINK_PORT}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();
    if !reachable {
        if std::env::var_os("FUCKMAIL_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev SMTP sink on {HOST}:{SINK_PORT} is required but not reachable");
        }
        eprintln!("skipping: dev SMTP sink not running (`make dev-up`)");
    }
    reachable
}

/// As `registered`, but able to send.
fn registered_sender(name: &str, user: &str) -> TempDir {
    let dir = registered(name, user);
    let store = Store::open(dir.0.join("fuckmail.db")).unwrap();
    let account = store.account_by_email(user).unwrap().unwrap();
    store
        .set_smtp(
            account.id,
            Some(&SmtpConfig {
                host: HOST.into(),
                port: SINK_PORT,
                security: SmtpSecurity::Plaintext,
            }),
        )
        .unwrap();
    dir
}

#[tokio::test]
async fn a_preview_shows_the_envelope_without_sending_anything() {
    if !dev_server_available() {
        return;
    }
    let user = "dev@fuckmail.test";
    let dir = registered_sender("preview", user);

    let preview = session(&dir)
        .preview(
            user,
            &core_rpc::DraftInput {
                to: vec!["jane@example.com".into()],
                bcc: vec!["secret@example.com".into()],
                subject: "Grüße".into(),
                body: "Hallo.".into(),
                ..Default::default()
            },
        )
        .unwrap();

    // The envelope is the only place a blind recipient appears, which is why
    // compose shows it: two names here, one of them invisible in the message.
    assert_eq!(preview.recipients.len(), 2);
    assert!(preview
        .recipients
        .contains(&"secret@example.com".to_string()));
    assert!(!preview.rfc822.contains("secret@example.com"));
    assert!(preview.rfc822.contains("To: <jane@example.com>"));
    // Non-ASCII survives as an encoded word rather than raw 8-bit.
    assert!(preview.rfc822.contains("Subject: =?utf-8?"));
}

#[tokio::test]
async fn a_reply_built_from_a_stored_message_threads_and_quotes() {
    // Compose asks for this before drawing anything, so reply-all's rules —
    // drop yourself, honour Reply-To — are applied in one place rather than
    // re-implemented in JavaScript.
    if !dev_server_available() {
        return;
    }
    let user = "dev@fuckmail.test";
    let dir = registered_sender("reply", user);
    let session = session(&dir);
    session.sync_account(user).await.unwrap();

    let core = core(&dir);
    let account = core.accounts().unwrap()[0].id;
    let target = core
        .messages(account, 0, 50, &ListFilter::default())
        .unwrap()
        .rows
        .into_iter()
        .find(|row| row.subject.contains("Termin"))
        .expect("the seeded German thread");

    let preview = session
        .preview(
            user,
            &core_rpc::DraftInput {
                body: "Passt.".into(),
                reply_to: Some(target.id),
                ..Default::default()
            },
        )
        .unwrap();

    // Re: is not stacked on a subject that already has one.
    assert!(preview.subject.starts_with("Re: "), "{}", preview.subject);
    assert!(!preview.subject[4..].to_lowercase().contains("re:"));
    // The typed text goes above the quote, which is where a reply is read from.
    assert!(preview.body.starts_with("Passt."), "{}", preview.body);
    assert!(preview.body.contains("\n> "), "{}", preview.body);
    assert!(preview.rfc822.contains("In-Reply-To: <"));
}

#[tokio::test]
async fn sending_files_a_copy_and_says_where() {
    if !dev_server_available() || !sink_available() {
        return;
    }
    let user = "session-send@fuckmail.test";
    let dir = registered_sender("send", user);
    let session = session(&dir);

    let sent = session
        .send(
            user,
            &core_rpc::DraftInput {
                to: vec!["jane@example.com".into()],
                subject: "from the session".into(),
                body: "hello".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(sent.recipients, vec!["jane@example.com"]);
    assert!(sent.filing_error.is_none(), "{:?}", sent.filing_error);
    assert_eq!(sent.filed_in.as_deref(), Some("Sent"));

    // And it comes back on the next sync, which is the loop closing.
    session.sync_account(user).await.unwrap();
    let core = core(&dir);
    let account = core.accounts().unwrap()[0].id;
    let found = core
        .messages(account, 0, 50, &ListFilter::default())
        .unwrap()
        .rows
        .into_iter()
        .find(|row| row.subject == "from the session");
    assert!(found.is_some(), "the sent copy should sync back");
}

#[tokio::test]
async fn an_account_with_no_submission_endpoint_says_so_before_composing() {
    if !dev_server_available() {
        return;
    }
    // `registered`, not `registered_sender`: no SMTP configured.
    let dir = registered("nosmtp", "dev@fuckmail.test");
    let err = session(&dir)
        .send(
            "dev@fuckmail.test",
            &core_rpc::DraftInput {
                to: vec!["jane@example.com".into()],
                ..Default::default()
            },
        )
        .await
        .expect_err("there is nowhere to submit to");
    assert!(err.to_string().contains("set-smtp"), "{err}");
}
