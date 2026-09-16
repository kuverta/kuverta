//! Saving drafts against the Dovecot dev servers from `docker/`.
//!
//! `drafts.rs` checks everything that happens before a connection. This checks
//! the part only a server can: that the draft lands in the folder the server
//! calls Drafts, flagged so other clients show it as one, and that the next sync
//! reads back the message that was built rather than something like it.
//!
//! Skips when the servers are down; `KUVERTA_REQUIRE_DEV_SERVER=1` makes that a
//! failure, as in `core-proto/tests/dev_server.rs`.

use core_rpc::{DraftInput, Session};
use core_store::model::{ImapSecurity, NewAccount};
use core_store::Store;

const HOST: &str = "127.0.0.1";
const DEV_PORT: u16 = 10143;
const GMAIL_PORT: u16 = 10144;
const PASSWORD_VAR: &str = "KUVERTA_DEV_PASSWORD";

fn available(port: u16) -> bool {
    let reachable = std::net::TcpStream::connect_timeout(
        &format!("{HOST}:{port}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    )
    .is_ok();
    if !reachable {
        if std::env::var_os("KUVERTA_REQUIRE_DEV_SERVER").is_some() {
            panic!("dev IMAP server on {HOST}:{port} is required but not reachable");
        }
        eprintln!("skipping: dev IMAP server on {port} not running (`make dev-up`)");
    }
    reachable
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("kuverta-drafts-dev-{}-{name}", std::process::id()));
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

/// A fresh mailbox per run: the dev servers accept any user, and APPEND is
/// additive, so a fixed user would accumulate drafts across runs.
fn unique_user(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("draft-{tag}-{nanos}@kuverta.test")
}

fn session_with_account(dir: &TempDir, user: &str, port: u16) -> Session {
    // SAFETY: the dev password baked into docker-compose.yml, not a real
    // credential; every test sets the same value.
    unsafe { std::env::set_var(PASSWORD_VAR, "devpass") };
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    store
        .add_account(&NewAccount {
            label: user.into(),
            email: user.into(),
            imap_host: HOST.into(),
            imap_port: port,
            imap_security: ImapSecurity::Plaintext,
            username: user.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    Session::new(dir.0.clone()).with_password_env(Some(PASSWORD_VAR.into()))
}

fn message_id_of(rfc822: &str) -> String {
    rfc822
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("message-id")
                .then(|| value.trim().trim_matches(['<', '>']).to_string())
        })
        .expect("a built draft carries a Message-ID")
}

async fn saves_and_reads_back(tag: &str, port: u16, expected_folder: &str) {
    let dir = TempDir::new(tag);
    let user = unique_user(tag);
    let session = session_with_account(&dir, &user, port);

    let saved = session
        .save_draft(
            &user,
            &DraftInput {
                to: vec!["jane@example.com".into()],
                cc: vec!["  ".into()],
                subject: "Angebot für nächste Woche".into(),
                body: "Hallo Jane,\n\nanbei der Entwurf.\n".into(),
                ..Default::default()
            },
        )
        .await
        .expect("save_draft");
    assert_eq!(saved.folder, expected_folder);

    // Read it back the way the app will, rather than trusting the OK.
    session.sync_account(&user).await.expect("sync");

    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store.account_by_email(&user).unwrap().unwrap().id;
    let message_id = message_id_of(&saved.preview.rfc822);
    let stored = store
        .message_by_rfc822_id(account, &message_id)
        .unwrap()
        .expect("the draft should sync back");
    assert_eq!(stored.subject.as_deref(), Some("Angebot für nächste Woche"));

    let locations = store.locations_of(stored.id).unwrap();
    assert_eq!(locations.len(), 1, "{locations:?}");
    let flags = &locations[0].flags;
    assert!(flags.contains("\\Draft"), "flags were {flags:?}");
    assert!(flags.contains("\\Seen"), "flags were {flags:?}");
}

#[tokio::test]
async fn a_draft_lands_in_drafts_flagged_as_one() {
    if !available(DEV_PORT) {
        return;
    }
    saves_and_reads_back("dev", DEV_PORT, "Drafts").await;
}

#[tokio::test]
async fn on_a_gmail_shaped_server_the_draft_goes_where_gmail_keeps_them() {
    // The Gmail layout declares \Drafts on "[Gmail]/Drafts" and strips it from
    // the plain "Drafts" the image also creates. Choosing by name would file
    // the draft somewhere Gmail's own clients never look.
    if !available(GMAIL_PORT) {
        return;
    }
    saves_and_reads_back("gmail", GMAIL_PORT, "[Gmail]/Drafts").await;
}
