//! What the list says for mail the account sent: who it went to, by the
//! names those people sign with — not the account's own name, over and over.

use core_rpc::Core;
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ME: &str = "dev@kuverta.test";

#[test]
fn a_sent_row_names_who_it_went_to_and_a_received_one_its_sender() {
    let path = std::env::temp_dir().join(format!("kuverta-sent-rows-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: ME.into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: ME.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let sent = store
        .upsert_folder(account, "Sent", Some("\\Sent"))
        .unwrap();
    let mail = |uid: u32, folder, from: (&str, &str), recipients: &str, subject: &str| {
        store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("m{uid}@x")),
                    subject: Some(subject.into()),
                    from_name: Some(from.0.into()),
                    from_addr: Some(from.1.into()),
                    recipients: Some(recipients.into()),
                    date_utc: Some(1_780_000_000 + uid as i64),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: folder,
                    uid,
                    flags: "\\Seen".into(),
                }),
            )
            .unwrap();
    };
    // Erika has written, so her name is known; Bob has only been written to.
    mail(
        1,
        inbox,
        ("Erika Mustermann", "erika@example.de"),
        ME,
        "Termin",
    );
    mail(
        2,
        sent,
        ("Dev", "Dev@Kuverta.test"),
        "erika@example.de, bob@example.com, dev@kuverta.test",
        "Re: Termin",
    );
    let core = Core::new(store, Blobs::new(dir.0.join("blobs")));

    let rows = core
        .messages(
            account,
            0,
            10,
            &ListFilter {
                folder: Some(sent),
                ..Default::default()
            },
        )
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1);
    // The account's own address, among the recipients, is not news.
    assert_eq!(
        rows[0].to,
        Some(vec![
            "Erika Mustermann".to_string(),
            "bob@example.com".to_string()
        ])
    );

    let inbox_rows = core
        .messages(
            account,
            0,
            10,
            &ListFilter {
                folder: Some(inbox),
                ..Default::default()
            },
        )
        .unwrap()
        .rows;
    assert_eq!(inbox_rows[0].to, None);
    assert_eq!(inbox_rows[0].from, "Erika Mustermann");
}
