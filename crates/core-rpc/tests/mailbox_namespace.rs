//! Always-on protocol regression: no Docker, credentials or hosted model needed.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

use core_accounts::EnvPassword;
use core_proto::{ImapClient, ImapConfig};
use core_rpc::{Core, Session};
use core_store::model::{ImapSecurity, Location, NewAccount, NewMessage};
use core_store::Store;

const EMAIL: &str = "namespace@kuverta.test";
const PASSWORD_VAR: &str = "KUVERTA_NAMESPACE_TEST_PASSWORD";

/// Exact wire commands matter here: a bare CREATE would fail on the user's
/// server, and a bare MOVE target would fail even if creation were fixed.
fn server(scripts: Vec<Vec<(String, String)>>) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = std::thread::spawn(move || {
        for script in scripts {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(b"* OK test server\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for (expected, answer) in script {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                assert_eq!(command, expected);
                stream
                    .write_all(answer.replace("{tag}", tag).as_bytes())
                    .unwrap();
            }
        }
    });
    (port, task)
}

fn step(command: impl Into<String>, response: impl Into<String>) -> (String, String) {
    (command.into(), response.into())
}

fn login() -> Vec<(String, String)> {
    vec![
        step(
            "LOGIN \"namespace@kuverta.test\" \"test-only\"",
            "{tag} OK login\r\n",
        ),
        step("ENABLE CONDSTORE", "{tag} BAD unsupported\r\n"),
    ]
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn create_and_file(prefix: &str, delimiter: &str, leaf: &str) {
    let full = format!("{prefix}{leaf}");
    let mut create = login();
    create.extend([
        step(
            "LIST \"\" *",
            format!("* LIST () {delimiter} \"INBOX\"\r\n{{tag}} OK list\r\n"),
        ),
        step(
            "LIST \"INBOX\" \"\"",
            format!("* LIST (\\Noselect) {delimiter} \"{prefix}\"\r\n{{tag}} OK root\r\n"),
        ),
        step(format!("CREATE \"{full}\""), "{tag} OK created\r\n"),
        step("LOGOUT", "* BYE done\r\n{tag} OK logout\r\n"),
    ]);
    let header = "Message-ID: <namespace-test@kuverta.test>\r\n\r\n";
    let mut filing = login();
    filing.extend([
        step("SELECT \"INBOX\"", "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n{tag} OK [READ-WRITE] selected\r\n"),
        step("UID FETCH 42 BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]", format!("* 1 FETCH (UID 42 BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n{header})\r\n{{tag}} OK fetched\r\n", header.len())),
        step("CAPABILITY", "* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK capabilities\r\n"),
        step(format!("UID MOVE 42 \"{full}\""), "* 1 EXPUNGE\r\n{tag} OK moved\r\n"),
        step("LOGOUT", "* BYE done\r\n{tag} OK logout\r\n"),
    ]);
    let (port, task) = server(vec![create, filing]);
    let dir = TempDir(
        std::env::temp_dir().join(format!("kuverta-namespace-{}-{port}", std::process::id())),
    );
    std::fs::create_dir_all(&dir.0).unwrap();
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: EMAIL.into(),
            email: EMAIL.into(),
            username: EMAIL.into(),
            imap_host: "127.0.0.1".into(),
            imap_port: port,
            imap_security: ImapSecurity::Plaintext,
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    // Each test uses this same dummy value; no actual credential is read.
    unsafe { std::env::set_var(PASSWORD_VAR, "test-only") };
    let session = Session::new(&dir.0).with_password_env(Some(PASSWORD_VAR.into()));
    let created = session.create_folder(EMAIL, leaf, None).await.unwrap();
    assert_eq!(created.name, full);
    assert_eq!(store.folder(created.id).unwrap().unwrap().name, full);

    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    store
        .set_folder_sync_state(inbox, Some(17), Some(43), None)
        .unwrap();
    let (message, _) = store
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some("namespace-test@kuverta.test".into()),
                subject: Some("Synthetic filing test".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: inbox,
                uid: 42,
                flags: String::new(),
            }),
        )
        .unwrap();
    // The same apply/queue path used by the assistant's move_messages tool.
    let core = Core::open(&dir.0).unwrap();
    core.apply(account, message, "move", Some(&created.name))
        .unwrap();
    let config = ImapConfig {
        host: "127.0.0.1".into(),
        port,
        security: ImapSecurity::Plaintext,
        username: EMAIL.into(),
    };
    let mut client = ImapClient::connect(&config, &EnvPassword::new(PASSWORD_VAR))
        .await
        .unwrap();
    let report = core_proto::flush_operations(&mut client, &store, account, i64::MAX)
        .await
        .unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(
        report.conflicted + report.retryable + report.non_atomic_moves,
        0
    );
    assert!(core.queue(account).unwrap().is_empty());
    client.logout().await.unwrap();
    task.join().unwrap();
}

#[tokio::test]
async fn prefixed_inbox_creation_and_assistant_filing_use_the_same_server_name() {
    create_and_file("INBOX.", "\".\"", "Wichtig").await;
}

#[tokio::test]
async fn ordinary_root_creation_and_filing_remain_unprefixed() {
    create_and_file("", "\"/\"", "Wichtig").await;
}

#[tokio::test]
async fn flat_namespace_allows_punctuation_in_folder_names() {
    create_and_file("", "NIL", "Invoices.2026/10").await;
}
