//! All inboxes: one list over several accounts, against a real store and
//! without a server. Newest first across the accounts, each row saying whose
//! it is, each account's own Sent left out, and the account found again from
//! a row's id — which is what every action from that list does.

use core_rpc::Core;
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn account(store: &Store, email: &str) -> AccountId {
    store
        .add_account(&NewAccount {
            label: email.into(),
            email: email.into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: email.into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap()
}

fn arrive(
    store: &Store,
    account: AccountId,
    folder: FolderId,
    uid: u32,
    subject: &str,
    from: &str,
    date: i64,
) -> MessageId {
    store
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some(format!("{subject}-{uid}@{from}")),
                subject: Some(subject.into()),
                from_addr: Some(from.into()),
                date_utc: Some(date),
                search_text: Some(subject.into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: folder,
                uid,
                flags: String::new(),
            }),
        )
        .unwrap()
        .0
}

#[test]
fn all_inboxes_is_one_list_over_the_accounts_newest_first_with_each_row_s_account() {
    let path = std::env::temp_dir().join(format!("kuverta-all-inboxes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
    let erika = account(&store, "erika@example.de");
    let work = account(&store, "erika@firma.example");
    let erika_inbox = store.upsert_folder(erika, "INBOX", None).unwrap();
    let erika_sent = store.upsert_folder(erika, "Sent", Some("\\Sent")).unwrap();
    let work_inbox = store.upsert_folder(work, "INBOX", None).unwrap();
    let private_mail = arrive(
        &store,
        erika,
        erika_inbox,
        1,
        "Fotos",
        "max@example.de",
        1_780_000_100,
    );
    let work_mail = arrive(
        &store,
        work,
        work_inbox,
        1,
        "Termin am Dienstag",
        "chef@firma.example",
        1_780_000_200,
    );
    let older_work = arrive(
        &store,
        work,
        work_inbox,
        2,
        "Protokoll",
        "chef@firma.example",
        1_780_000_050,
    );
    // What she sent from the private account is in Sent only: not mail that
    // arrived, so not in a list of inboxes.
    arrive(
        &store,
        erika,
        erika_sent,
        1,
        "Re: Fotos",
        "erika@example.de",
        1_780_000_300,
    );
    let core = Core::new(store, Blobs::new(dir.0.join("blobs")));

    let all = ListFilter {
        incoming_only: true,
        ..Default::default()
    };
    let page = core.messages_in(&[erika, work], 0, 50, &all).unwrap();
    assert_eq!(page.total, 3);
    let seen: Vec<(MessageId, AccountId)> = page.rows.iter().map(|r| (r.id, r.account)).collect();
    assert_eq!(
        seen,
        vec![(work_mail, work), (private_mail, erika), (older_work, work)]
    );

    // One account alone is the list it always was, and says whose it is too.
    let one = core.messages(erika, 0, 50, &all).unwrap();
    assert_eq!(one.rows.len(), 1);
    assert_eq!(one.rows[0].account, erika);

    // Oldest first turns the whole list round, not each account's part.
    let oldest = ListFilter {
        oldest_first: true,
        ..all.clone()
    };
    let ids: Vec<MessageId> = core
        .messages_in(&[erika, work], 0, 50, &oldest)
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![older_work, private_mail, work_mail]);

    // No accounts is nothing, not everything.
    assert_eq!(core.messages_in(&[], 0, 50, &all).unwrap().total, 0);

    // A row's id is enough to find its account again, which is what a move
    // or a read from the list does.
    assert_eq!(core.account_of_message(work_mail).unwrap(), work);
    assert_eq!(core.account_of_message(private_mail).unwrap(), erika);
    assert!(core.account_of_message(999_999).is_err());
    // And the message is read with the account found that way.
    assert_eq!(
        core.message(work, work_mail).unwrap().subject.as_deref(),
        Some("Termin am Dienstag")
    );
}
