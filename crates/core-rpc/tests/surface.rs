//! The shape the shell sees.
//!
//! These run against a real store rather than a mock, because the interesting
//! part is the translation — which copy of a message a change acts on, what a
//! row looks like once the store's `Option`s have been resolved — and a mock
//! would only assert that the translation matches itself.

use core_rpc::{Core, RpcError};
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("fuckmail-rpc-{}-{name}", std::process::id()));
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

/// An account with INBOX and Archive, and `count` messages in INBOX.
fn core_with(name: &str, count: usize) -> (Core, AccountId, FolderId, FolderId, TempDir) {
    let dir = TempDir::new(name);
    let store = Store::open(dir.0.join("fuckmail.db")).unwrap();
    let blobs = Blobs::new(dir.0.join("blobs"));

    let account = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@fuckmail.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "dev@fuckmail.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();

    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();

    for n in 0..count {
        store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("row-{n}@example.com")),
                    subject: Some(format!("message {n}")),
                    from_name: Some("Anna Weber".into()),
                    from_addr: Some("anna@example.de".into()),
                    date_utc: Some(1_700_000_000 + n as i64),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: n as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap();
    }

    (Core::new(store, blobs), account, inbox, archive, dir)
}

#[test]
fn a_page_carries_the_total_so_the_list_can_size_itself() {
    let (core, account, _, _, _dir) = core_with("page", 50);

    let page = core
        .messages(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(page.total, 50);
    assert_eq!(page.offset, 0);
    assert_eq!(page.rows.len(), 10);
    // Newest first.
    assert_eq!(page.rows[0].subject, "message 49");

    let later = core
        .messages(account, 40, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(
        later.total, 50,
        "the total describes the filter, not the page"
    );
    assert_eq!(later.rows.len(), 10);
    assert_eq!(later.rows[0].subject, "message 9");
}

#[test]
fn a_row_has_no_absent_fields_for_the_shell_to_second_guess() {
    // The store models "no subject" and "no sender" honestly as `None`. A list
    // has to draw something either way, so the decision is made once here
    // rather than in every renderer.
    let (core, account, inbox, _, _dir) = core_with("row", 0);
    core.store()
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some("bare@example.com".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: inbox,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();

    let page = core
        .messages(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(page.rows[0].subject, "(no subject)");
    assert_eq!(page.rows[0].from, "(unknown)");

    // And a display name is preferred over the address when there is one.
    let (core, account, _, _, _dir) = core_with("row2", 1);
    let page = core
        .messages(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(page.rows[0].from, "Anna Weber");
}

#[test]
fn moving_a_message_queues_it_from_the_inbox_copy() {
    // One message in two folders is the Gmail case and the reason this is a
    // choice rather than a lookup: acting on the wrong copy would archive
    // something that is already archived and leave the inbox untouched.
    let (core, account, inbox, archive, _dir) = core_with("move", 1);
    let id = core
        .messages(account, 0, 1, &ListFilter::default())
        .unwrap()
        .rows[0]
        .id;

    core.store()
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some("row-0@example.com".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: archive,
                uid: 99,
                flags: String::new(),
            }),
        )
        .unwrap();
    assert_eq!(core.store().locations_of(id).unwrap().len(), 2);

    core.move_to(account, id, "Archive", 0).unwrap();

    let queued = core.store().pending_operations(account).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].source_folder_id, inbox,
        "should act on the inbox copy"
    );
    assert_eq!(queued[0].source_uid, 1);
    // And it records what it expects to find, which is what the executor
    // checks before touching anything.
    assert_eq!(
        queued[0].expect_message_id.as_deref(),
        Some("row-0@example.com")
    );
}

#[test]
fn moving_a_message_that_is_already_there_is_refused_with_a_reason() {
    let (core, account, _, _, _dir) = core_with("already", 1);
    let id = core
        .messages(account, 0, 1, &ListFilter::default())
        .unwrap()
        .rows[0]
        .id;

    match core.move_to(account, id, "INBOX", 0) {
        Err(RpcError::Rejected(reason)) => assert!(reason.contains("already in INBOX"), "{reason}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(core.store().pending_operations(account).unwrap().is_empty());
}

#[test]
fn the_queue_counts_down_the_undo_window_and_undo_takes_the_last_one() {
    let (core, account, _, _, _dir) = core_with("queue", 2);
    let rows = core
        .messages(account, 0, 2, &ListFilter::default())
        .unwrap()
        .rows;

    core.move_to(account, rows[0].id, "Archive", 600).unwrap();
    let second = core.set_read(account, rows[1].id, true, 600).unwrap();

    let queue = core.queue(account).unwrap();
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0].what, "move to Archive");
    assert_eq!(queue[1].what, "set \\Seen");
    assert!(
        queue[0].holds_for > 0,
        "a change inside its window still holds"
    );

    let undone = core.undo(account).unwrap().expect("something to undo");
    assert_eq!(undone.id, second, "undo takes the most recent");
    assert_eq!(core.queue(account).unwrap().len(), 1);

    core.undo(account).unwrap().expect("one left");
    // Nothing left is an answer, not a failure.
    assert!(core.undo(account).unwrap().is_none());
    assert!(core.queue(account).unwrap().is_empty());
}

#[test]
fn an_unknown_message_is_an_error_rather_than_an_empty_answer() {
    let (core, account, _, _, _dir) = core_with("unknown", 1);
    assert!(matches!(
        core.message(account, 9999),
        Err(RpcError::UnknownMessage(9999))
    ));
    assert!(matches!(
        core.move_to(account, 9999, "Archive", 0),
        Err(RpcError::UnknownMessage(9999))
    ));
}

#[test]
fn a_message_whose_body_is_missing_still_opens() {
    // The row is worth showing even when the blob has gone: the alternative is
    // a list that cannot open a message because of a file the user can do
    // nothing about.
    let (core, account, inbox, _, _dir) = core_with("nobody", 0);
    core.store()
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some("gone@example.com".into()),
                subject: Some("body went missing".into()),
                body_path: Some("nowhere/at/all".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: inbox,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();

    let id = core
        .messages(account, 0, 1, &ListFilter::default())
        .unwrap()
        .rows[0]
        .id;
    let detail = core.message(account, id).unwrap();
    assert_eq!(detail.subject.as_deref(), Some("body went missing"));
    assert!(detail.raw.is_none());
    assert_eq!(detail.folders, vec!["INBOX"]);
}

#[test]
fn accounts_say_whether_they_can_send() {
    // So the shell can grey out compose rather than discover it at send time.
    let (core, _, _, _, _dir) = core_with("send", 0);
    let accounts = core.accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(!accounts[0].can_send);

    core.store()
        .set_smtp(
            accounts[0].id,
            Some(&SmtpConfig {
                host: "smtp.example.com".into(),
                port: 587,
                security: SmtpSecurity::StartTls,
            }),
        )
        .unwrap();
    assert!(core.accounts().unwrap()[0].can_send);
}
