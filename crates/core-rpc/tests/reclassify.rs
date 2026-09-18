//! Filing mail again when the rules change: a verdict the old rules got wrong
//! is replaced, what a person decided is not, and it happens once.

use core_rpc::Session;
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BANK: &str = "From: Sparkasse Musterstadt <k.meier@sparkasse-musterstadt.example>\r\n\
                    To: dev@kuverta.test\r\n\
                    Subject: Neue Nachricht im Postfach\r\n\
                    Message-ID: <bank@sparkasse-musterstadt.example>\r\n\r\n\
                    Sie haben eine neue Nachricht.\r\n";

const FRIEND: &str = "From: Anna Weber <anna@weber.example>\r\n\
                      To: dev@kuverta.test\r\n\
                      Subject: Fotos vom Wochenende\r\n\
                      Message-ID: <fotos@weber.example>\r\n\r\n\
                      Hier sind die Fotos.\r\n";

#[test]
fn mail_the_old_rules_filed_is_filed_again_once_and_corrections_stand() {
    let path = std::env::temp_dir().join(format!("kuverta-reclassify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let blobs = Blobs::new(dir.0.join("blobs"));
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
    let mut ids = Vec::new();
    for (uid, raw) in [BANK, FRIEND, BANK.replace("bank@", "bank2@").as_str()]
        .iter()
        .enumerate()
    {
        let body_path = blobs
            .put(account, &format!("m{uid}"), raw.as_bytes())
            .unwrap();
        let (id, _) = store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("m{uid}@x")),
                    body_path: Some(body_path),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: uid as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap();
        // What the first rules said: all of it personal.
        store
            .record_verdict(
                id,
                &Verdict {
                    category: "personal".into(),
                    confidence: Some(0.7),
                    source: ClassifierSource::Rules,
                    model: None,
                    latency_ms: Some(0),
                },
            )
            .unwrap();
        ids.push(id);
    }
    // The second bank message the person filed as personal themselves.
    store
        .record_verdict(
            ids[2],
            &Verdict {
                category: "personal".into(),
                confidence: Some(1.0),
                source: ClassifierSource::User,
                model: None,
                latency_ms: None,
            },
        )
        .unwrap();

    let session = Session::new(&dir.0);
    let preview = session.reclassify("dev@kuverta.test", 100, false).unwrap();
    assert_eq!(preview.changed, 2, "{preview:?}");
    assert_eq!(
        store.rules_category(ids[0]).unwrap().as_deref(),
        Some("personal"),
        "a preview writes nothing"
    );

    let done = session.reclassify("dev@kuverta.test", 100, true).unwrap();
    assert_eq!((done.examined, done.changed, done.remaining), (3, 2, 0));
    assert_ne!(
        store.current_category(ids[0]).unwrap().as_deref(),
        Some("personal")
    );
    assert_eq!(
        store.current_category(ids[1]).unwrap().as_deref(),
        Some("personal"),
        "Anna is a person"
    );
    assert_eq!(
        store.current_category(ids[2]).unwrap().as_deref(),
        Some("personal"),
        "what the person said stands"
    );

    // Once: nothing is behind any more.
    assert_eq!(
        session
            .reclassify("dev@kuverta.test", 100, true)
            .unwrap()
            .examined,
        0
    );
}
