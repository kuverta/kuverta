//! The address book the To field suggests from: who is in it, under what
//! name, in what order — and who is left out.

use core_store::model::*;
use core_store::Store;

const ME: &str = "dev@kuverta.test";
const NOW: i64 = 1_790_000_000;
const DAY: i64 = 86_400;

struct Mailbox {
    store: Store,
    account: AccountId,
    inbox: FolderId,
    sent: FolderId,
    trash: FolderId,
    uid: u32,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mailbox(name: &str) -> Mailbox {
    let path = std::env::temp_dir().join(format!("kuverta-book-{}-{name}", std::process::id()));
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
    let trash = store
        .upsert_folder(account, "Trash", Some("\\Trash"))
        .unwrap();
    Mailbox {
        store,
        account,
        inbox,
        sent,
        trash,
        uid: 1,
        _dir: dir,
    }
}

impl Mailbox {
    /// A message, filed as `category` when one is given.
    fn mail(
        &mut self,
        folder: FolderId,
        from: (&str, &str),
        recipients: &str,
        days_ago: i64,
        category: Option<&str>,
    ) {
        let uid = self.uid;
        self.uid += 1;
        let (id, _) = self
            .store
            .upsert_message(
                self.account,
                &NewMessage {
                    rfc822_message_id: Some(format!("m{uid}@x")),
                    subject: Some(format!("Message {uid}")),
                    from_name: (!from.0.is_empty()).then(|| from.0.to_string()),
                    from_addr: Some(from.1.into()),
                    recipients: Some(recipients.into()),
                    date_utc: Some(NOW - days_ago * DAY),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: folder,
                    uid,
                    flags: String::new(),
                }),
            )
            .unwrap();
        if let Some(category) = category {
            self.store
                .record_verdict(
                    id,
                    &Verdict {
                        category: category.into(),
                        confidence: Some(0.9),
                        source: ClassifierSource::Rules,
                        model: None,
                        latency_ms: None,
                    },
                )
                .unwrap();
        }
    }

    fn book(&self) -> Vec<Contact> {
        self.store
            .address_book(self.account, ME, &["Trash".to_string()], NOW, 100)
            .unwrap()
    }
}

use core_store::Contact;

#[test]
fn who_is_written_to_comes_first_and_who_only_sends_bulk_is_not_there() {
    let mut m = mailbox("rank");
    let (inbox, sent, trash) = (m.inbox, m.sent, m.trash);
    // Written to twice, lately, and she writes back — under a name she has
    // changed how she signs.
    m.mail(
        sent,
        ("Dev", ME),
        "erika@example.de, bob@example.com",
        1,
        None,
    );
    m.mail(sent, ("", "DEV@kuverta.test"), "Erika@Example.de", 10, None);
    m.mail(
        inbox,
        ("Erika Mustermann", "erika@example.de"),
        ME,
        2,
        Some("personal"),
    );
    m.mail(
        inbox,
        ("E. Mustermann", "erika@example.de"),
        ME,
        400,
        Some("personal"),
    );
    // Wrote once, three years ago.
    m.mail(
        inbox,
        ("Max", "max@example.org"),
        ME,
        1100,
        Some("personal"),
    );
    // A shop's newsletter, a bank's no-reply, spam in the Trash, and a note
    // to self: none of them anyone to suggest.
    m.mail(
        inbox,
        ("Shop", "news@shop.example"),
        ME,
        1,
        Some("newsletter"),
    );
    m.mail(
        inbox,
        ("Bank", "noreply@bank.example"),
        ME,
        1,
        Some("transactional"),
    );
    m.mail(
        trash,
        ("Win", "prize@spam.example"),
        ME,
        1,
        Some("personal"),
    );
    m.mail(sent, ("Dev", ME), "dev@kuverta.test", 1, None);

    let book = m.book();
    let addresses: Vec<&str> = book.iter().map(|c| c.address.as_str()).collect();
    assert_eq!(
        addresses,
        ["erika@example.de", "bob@example.com", "max@example.org"]
    );

    let erika = &book[0];
    assert_eq!((erika.sent, erika.received), (2, 2));
    assert_eq!(erika.name.as_deref(), Some("Erika Mustermann"));
    // Only ever written to: no name the store could know.
    assert_eq!(book[1].name, None);
    assert_eq!((book[1].sent, book[1].received), (1, 0));
    assert!(book[1].score > book[2].score, "{book:?}");
}

#[test]
fn recipients_written_with_names_are_read_by_their_address() {
    let mut m = mailbox("names");
    let sent = m.sent;
    m.mail(
        sent,
        ("Dev", ME),
        "\"Mustermann, Erika\" <Erika@Example.de>, <carl@example.com>, not an address",
        1,
        None,
    );
    let addresses: Vec<String> = m.book().into_iter().map(|c| c.address).collect();
    assert!(
        addresses.contains(&"erika@example.de".to_string()),
        "{addresses:?}"
    );
    assert!(
        addresses.contains(&"carl@example.com".to_string()),
        "{addresses:?}"
    );
    // "Mustermann" on its own, cut from the quoted name, is not an address.
    assert_eq!(addresses.len(), 2, "{addresses:?}");
}
