//! A smart mailbox that takes its mail out of the Inbox, against a real store
//! and without a server: what leaves the Inbox when it is saved, what new
//! mail does, what stays where it was put, and that the mailbox's own task
//! stays the mailbox's.

use core_rpc::{Core, SmartMailboxInput};
use core_store::model::*;
use core_store::{Blobs, SmartField, SmartOp, SmartQuery, SmartRule, Store};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct World {
    core: Core,
    account: AccountId,
    inbox: FolderId,
    archive: FolderId,
    next_uid: u32,
    _dir: TempDir,
}

/// INBOX with two invoices and a letter from a person; Archive with an
/// invoice filed there on purpose; and a Rechnungen folder.
fn world(name: &str) -> World {
    let path =
        std::env::temp_dir().join(format!("kuverta-smart-inbox-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let dir = TempDir(path);
    let store = Store::open(dir.0.join("kuverta.db")).unwrap();
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
    let archive = store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();
    store.upsert_folder(account, "Rechnungen", None).unwrap();
    let mut world = World {
        core: Core::new(store, Blobs::new(dir.0.join("blobs"))),
        account,
        inbox,
        archive,
        next_uid: 1,
        _dir: dir,
    };
    world.arrive("Invoice 1001", inbox);
    world.arrive("Ihre Rechnung Juni", inbox);
    world.arrive("Termin am Dienstag", inbox);
    world.arrive("Invoice 0999", archive);
    world
}

impl World {
    /// A message as a sync stores it.
    fn arrive(&mut self, subject: &str, folder: FolderId) -> MessageId {
        let uid = self.next_uid;
        self.next_uid += 1;
        self.core
            .store()
            .upsert_message(
                self.account,
                &NewMessage {
                    rfc822_message_id: Some(format!("m{uid}@x")),
                    subject: Some(subject.into()),
                    from_addr: Some("someone@example.com".into()),
                    date_utc: Some(1_780_000_000 + uid as i64),
                    search_text: Some(format!("{subject} body")),
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

    fn subjects_in(&self, folder: FolderId) -> Vec<String> {
        self.listed(&ListFilter {
            folder: Some(folder),
            ..Default::default()
        })
    }

    fn listed(&self, filter: &ListFilter) -> Vec<String> {
        let mut subjects: Vec<String> = self
            .core
            .messages(self.account, 0, 100, filter)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.subject)
            .collect();
        subjects.sort();
        subjects
    }

    fn save(&self, id: Option<i64>, folder: Option<&str>) -> i64 {
        self.core
            .save_smart_mailbox(&SmartMailboxInput {
                id,
                account_id: self.account,
                name: "Rechnungen".into(),
                query: invoices(),
                folder: folder.map(String::from),
            })
            .unwrap()
    }
}

/// Any of two rules: the case where "and it is in the Inbox" cannot simply be
/// one more rule.
fn invoices() -> SmartQuery {
    SmartQuery {
        match_all: false,
        rules: vec![
            SmartRule {
                field: SmartField::Subject,
                op: SmartOp::Contains,
                value: "invoice".into(),
            },
            SmartRule {
                field: SmartField::Subject,
                op: SmartOp::Contains,
                value: "rechnung".into(),
            },
        ],
    }
}

#[test]
fn a_search_moves_nothing_and_one_with_a_folder_empties_the_inbox_of_its_mail() {
    let world = world("save");

    // A smart mailbox as it always was: a search. The Inbox keeps its mail.
    let id = world.save(None, None);
    assert_eq!(
        world
            .core
            .sort_into_smart_mailbox(world.account, id)
            .unwrap(),
        0
    );
    assert_eq!(world.subjects_in(world.inbox).len(), 3);

    // Given a folder, what it gathers in the Inbox leaves — and only that:
    // the invoice filed in Archive stays filed.
    world.save(Some(id), Some("rechnungen"));
    assert_eq!(
        world
            .core
            .sort_into_smart_mailbox(world.account, id)
            .unwrap(),
        2
    );
    assert_eq!(world.subjects_in(world.inbox), ["Termin am Dienstag"]);
    assert_eq!(world.subjects_in(world.archive), ["Invoice 0999"]);

    // The mailbox lists its folder now, spelt as the server spells it.
    let shown = world.core.smart_filter(id).unwrap();
    assert_eq!(shown.smart, None);
    assert_eq!(world.listed(&shown), ["Ihre Rechnung Juni", "Invoice 1001"]);
    let view = &world.core.smart_mailboxes(world.account).unwrap()[0];
    assert_eq!(view.folder.as_deref(), Some("Rechnungen"));
    assert_eq!(view.total, 2);
}

#[test]
fn new_mail_is_sorted_by_the_next_run_and_what_was_put_back_stays() {
    let mut world = world("new");
    let id = world.save(None, Some("Rechnungen"));
    world
        .core
        .sort_into_smart_mailbox(world.account, id)
        .unwrap();

    // Put back in the Inbox by hand: the move is undone.
    let undone = world.core.undo(world.account).unwrap();
    assert!(undone.is_some());
    assert_eq!(world.subjects_in(world.inbox).len(), 2);

    // A sync brings a new invoice; the run after it takes that one, and
    // leaves the one that was put back where it was put.
    world.arrive("Invoice 1002", world.inbox);
    let runs = world.core.run_rule_tasks(world.account).unwrap();
    assert_eq!(runs.iter().map(|run| run.done).sum::<usize>(), 1);
    let inbox = world.subjects_in(world.inbox);
    assert!(!inbox.contains(&"Invoice 1002".to_string()), "{inbox:?}");
    assert_eq!(inbox.len(), 2, "{inbox:?}");
}

#[test]
fn the_mailboxs_task_is_its_own_and_goes_with_it() {
    let world = world("task");
    let id = world.save(None, Some("Rechnungen"));

    // Not in the person's list of tasks: it is edited as the mailbox.
    assert!(world.core.tasks(world.account).unwrap().is_empty());
    assert!(world
        .core
        .store()
        .task_of_smart_mailbox(id)
        .unwrap()
        .is_some());

    // A search again: the task goes, and the mailbox lists by its rules.
    world.save(Some(id), None);
    assert!(world
        .core
        .store()
        .task_of_smart_mailbox(id)
        .unwrap()
        .is_none());
    assert!(world.core.smart_filter(id).unwrap().smart.is_some());

    // Deleted: its task with it, and the folder and its mail stay.
    world.save(Some(id), Some("Rechnungen"));
    world
        .core
        .sort_into_smart_mailbox(world.account, id)
        .unwrap();
    world.core.delete_smart_mailbox(id).unwrap();
    assert!(world
        .core
        .store()
        .task_of_smart_mailbox(id)
        .unwrap()
        .is_none());
    assert!(world.core.run_rule_tasks(world.account).unwrap().is_empty());
}

#[test]
fn a_folder_that_is_not_there_or_is_the_inbox_is_refused() {
    let world = world("refused");
    for folder in ["Nirgendwo", "inbox"] {
        let err = world
            .core
            .save_smart_mailbox(&SmartMailboxInput {
                id: None,
                account_id: world.account,
                name: "Rechnungen".into(),
                query: invoices(),
                folder: Some(folder.into()),
            })
            .unwrap_err()
            .to_string();
        assert!(err.contains(folder) || err.contains("Inbox"), "{err}");
    }
    // Nothing was saved half way.
    assert!(world
        .core
        .smart_mailboxes(world.account)
        .unwrap()
        .is_empty());
}
