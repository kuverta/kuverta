//! Store behaviour that is expensive to get wrong, exercised end to end.

use core_store::model::*;
use core_store::{Store, Upsert};

fn store_with_account() -> (Store, AccountId) {
    let store = Store::open_in_memory().expect("open store");
    let id = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@fuckmail.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 10143,
            imap_security: ImapSecurity::Plaintext,
            username: "dev@fuckmail.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .expect("add account");
    (store, id)
}

fn newsletter() -> NewMessage {
    NewMessage {
        rfc822_message_id: Some("newsletter-2026-w36@news.rustweekly.example".into()),
        subject: Some("Rust Weekly #612".into()),
        from_addr: Some("hello@news.rustweekly.example".into()),
        date_utc: Some(1_756_620_000),
        size_bytes: Some(685),
        list_id: Some("news.rustweekly.example".into()),
        search_text: Some("async fn in traits stabilises".into()),
        ..Default::default()
    }
}

#[test]
fn one_message_in_two_folders_is_stored_once_with_two_locations() {
    // This is the Gmail labels-as-folders case. If it regresses, every count
    // shown in the UI is inflated.
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();

    let (first_id, first) = store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();

    let (second_id, second) = store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: archive,
                uid: 1,
                flags: "\\Seen".into(),
            }),
        )
        .unwrap();

    assert_eq!(first, Upsert::Inserted);
    assert_eq!(second, Upsert::Deduplicated);
    assert_eq!(first_id, second_id, "same message must collapse to one row");

    assert_eq!(store.message_count(account).unwrap(), 1);
    assert_eq!(store.location_count(account).unwrap(), 2);

    let locations = store.locations_of(first_id).unwrap();
    assert_eq!(locations.len(), 2);
}

#[test]
fn the_same_message_on_two_accounts_stays_separate() {
    // Two accounts really did each receive a copy; collapsing them would lose
    // the fact that it arrived twice. Dedup is per account, never global.
    let (store, first_account) = store_with_account();
    let second_account = store
        .add_account(&NewAccount {
            label: "Work".into(),
            email: "work@fuckmail.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 10143,
            imap_security: ImapSecurity::Plaintext,
            username: "work@fuckmail.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();

    store
        .upsert_message(first_account, &newsletter(), None)
        .unwrap();
    let (_, outcome) = store
        .upsert_message(second_account, &newsletter(), None)
        .unwrap();

    assert_eq!(outcome, Upsert::Inserted);
    assert_eq!(store.message_count(first_account).unwrap(), 1);
    assert_eq!(store.message_count(second_account).unwrap(), 1);
}

#[test]
fn messages_without_a_message_id_do_not_collapse_into_each_other() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    let mut first = NewMessage {
        subject: Some("nightly backup completed".into()),
        from_addr: Some("noreply@legacy.example.org".into()),
        date_utc: Some(1_757_020_267),
        size_bytes: Some(342),
        ..Default::default()
    };
    let mut second = first.clone();
    second.date_utc = Some(1_757_106_667); // the next night's run
    second.size_bytes = Some(344);

    store
        .upsert_message(
            account,
            &first,
            Some(&Location {
                folder_id: inbox,
                uid: 6,
                flags: String::new(),
            }),
        )
        .unwrap();
    let (_, outcome) = store
        .upsert_message(
            account,
            &second,
            Some(&Location {
                folder_id: inbox,
                uid: 9,
                flags: String::new(),
            }),
        )
        .unwrap();

    assert_eq!(outcome, Upsert::Inserted);
    assert_eq!(store.message_count(account).unwrap(), 2);

    // ...but re-syncing the identical message must still deduplicate.
    first.snippet = Some("recomputed on resync".into());
    let (_, repeat) = store
        .upsert_message(
            account,
            &first,
            Some(&Location {
                folder_id: inbox,
                uid: 6,
                flags: "\\Seen".into(),
            }),
        )
        .unwrap();
    assert_eq!(repeat, Upsert::Deduplicated);
    assert_eq!(store.message_count(account).unwrap(), 2);
}

#[test]
fn resyncing_a_folder_updates_flags_rather_than_failing() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    let location = Location {
        folder_id: inbox,
        uid: 2,
        flags: String::new(),
    };
    let (id, _) = store
        .upsert_message(account, &newsletter(), Some(&location))
        .unwrap();

    let seen = Location {
        folder_id: inbox,
        uid: 2,
        flags: "\\Seen".into(),
    };
    store
        .upsert_message(account, &newsletter(), Some(&seen))
        .unwrap();

    let locations = store.locations_of(id).unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].flags, "\\Seen");
}

#[test]
fn uid_validity_change_drops_cached_uids_but_keeps_shared_messages() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store.upsert_folder(account, "Archive", None).unwrap();

    // One message in both folders, plus one that only lives in the inbox.
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: archive,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();

    let mut inbox_only = newsletter();
    inbox_only.rfc822_message_id = Some("inbox-only@example.com".into());
    store
        .upsert_message(
            account,
            &inbox_only,
            Some(&Location {
                folder_id: inbox,
                uid: 3,
                flags: String::new(),
            }),
        )
        .unwrap();

    assert_eq!(store.message_count(account).unwrap(), 2);

    store.invalidate_folder(inbox).unwrap();

    // The shared message survives because Archive still references it; the
    // inbox-only one is gone along with its stale UID.
    assert_eq!(store.message_count(account).unwrap(), 1);
    assert_eq!(store.location_count(account).unwrap(), 1);
    assert_eq!(store.max_uid(inbox).unwrap(), None);
}

#[test]
fn max_uid_reports_the_high_water_mark_per_folder() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    assert_eq!(store.max_uid(inbox).unwrap(), None);

    for uid in [1u32, 7, 4] {
        let mut m = newsletter();
        m.rfc822_message_id = Some(format!("m{uid}@example.com"));
        store
            .upsert_message(
                account,
                &m,
                Some(&Location {
                    folder_id: inbox,
                    uid,
                    flags: String::new(),
                }),
            )
            .unwrap();
    }

    assert_eq!(store.max_uid(inbox).unwrap(), Some(7));
}

#[test]
fn search_finds_by_subject_and_body_and_is_scoped_to_the_account() {
    let (store, account) = store_with_account();
    store.upsert_message(account, &newsletter(), None).unwrap();

    assert_eq!(store.search(account, "Rust Weekly", 10).unwrap().len(), 1);
    assert_eq!(store.search(account, "stabilises", 10).unwrap().len(), 1);
    assert_eq!(
        store.search(account, "Steuererklärung", 10).unwrap().len(),
        0
    );
}

#[test]
fn search_treats_fts5_operators_as_literal_text() {
    // Unescaped, a query like `foo OR bar` or a stray quote is FTS5 syntax and
    // either matches the wrong rows or raises an error mid-search.
    let (store, account) = store_with_account();
    store.upsert_message(account, &newsletter(), None).unwrap();

    for hostile in ["\"", "foo OR bar", "NEAR(a b)", "*", "subject:"] {
        let result = store.search(account, hostile, 10);
        assert!(
            result.is_ok(),
            "query {hostile:?} failed: {:?}",
            result.err()
        );
    }
}

#[test]
fn rules_and_model_verdicts_coexist_and_disagreements_are_queryable() {
    // The baseline comparison from the plan: both classifiers record a verdict
    // for every message, so "did the model beat the rules" stays answerable.
    let (store, account) = store_with_account();
    let (id, _) = store.upsert_message(account, &newsletter(), None).unwrap();

    store
        .record_verdict(
            id,
            &Verdict {
                category: "newsletter".into(),
                confidence: Some(0.95),
                source: ClassifierSource::Rules,
                model: None,
                latency_ms: Some(0),
            },
        )
        .unwrap();
    store
        .record_verdict(
            id,
            &Verdict {
                category: "marketing".into(),
                confidence: Some(0.71),
                source: ClassifierSource::Model,
                model: Some("qwen3:8b".into()),
                latency_ms: Some(412),
            },
        )
        .unwrap();

    let disagreements = store.disagreements(account).unwrap();
    assert_eq!(disagreements.len(), 1);
    assert_eq!(disagreements[0].rules_category, "newsletter");
    assert_eq!(disagreements[0].model_category, "marketing");

    store
        .record_correction(id, Some("marketing"), "newsletter")
        .unwrap();
}

#[test]
fn reopening_a_database_is_a_no_op_migration() {
    let dir = std::env::temp_dir().join(format!("fuckmail-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");

    let account = {
        let store = Store::open(&path).unwrap();
        store
            .add_account(&NewAccount {
                label: "Dev".into(),
                email: "dev@fuckmail.test".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: 10143,
                imap_security: ImapSecurity::Plaintext,
                username: "dev@fuckmail.test".into(),
                auth_method: "app_password".into(),
                ..Default::default()
            })
            .unwrap()
    };

    let store = Store::open(&path).unwrap();
    assert_eq!(store.accounts().unwrap().len(), 1);
    assert_eq!(store.message_count(account).unwrap(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flag_updates_report_whether_anything_changed() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();

    assert!(store.set_location_flags(inbox, 2, "\\Seen").unwrap());
    // Re-reporting the same flags is not a change; counting it as one would
    // make every sync look like it did work.
    assert!(!store.set_location_flags(inbox, 2, "\\Seen").unwrap());
    assert!(store
        .set_location_flags(inbox, 2, "\\Seen \\Flagged")
        .unwrap());

    let locations = store.locations_of(1).unwrap();
    assert_eq!(locations[0].flags, "\\Seen \\Flagged");
}

#[test]
fn expunging_from_one_folder_keeps_a_message_that_lives_in_another() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store.upsert_folder(account, "Archive", None).unwrap();

    let (id, _) = store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: archive,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();

    // Unlabelled in Gmail terms: gone from the inbox, still archived.
    assert_eq!(store.remove_locations(inbox, &[2]).unwrap(), 1);
    assert_eq!(store.message_count(account).unwrap(), 1);
    assert_eq!(store.locations_of(id).unwrap().len(), 1);

    // Removing the last location leaves the message behind, on purpose: a
    // message being moved is out of its old folder before it appears in the
    // new one, and dropping it in between would take its history with it.
    assert_eq!(store.remove_locations(archive, &[1]).unwrap(), 1);
    assert_eq!(store.location_count(account).unwrap(), 0);
    assert_eq!(store.message_count(account).unwrap(), 1);

    // Collecting orphans is a separate step, for the end of a sync.
    assert_eq!(store.delete_orphaned_messages().unwrap(), 1);
    assert_eq!(store.message_count(account).unwrap(), 0);
}

#[test]
fn removing_no_locations_is_a_no_op() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();

    assert_eq!(store.remove_locations(inbox, &[]).unwrap(), 0);
    assert_eq!(store.message_count(account).unwrap(), 1);
}

#[test]
fn folder_uids_lists_what_is_cached() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    for uid in [3u32, 1, 9] {
        let mut m = newsletter();
        m.rfc822_message_id = Some(format!("m{uid}@example.com"));
        store
            .upsert_message(
                account,
                &m,
                Some(&Location {
                    folder_id: inbox,
                    uid,
                    flags: String::new(),
                }),
            )
            .unwrap();
    }

    let mut uids = store.folder_uids(inbox).unwrap();
    uids.sort_unstable();
    assert_eq!(uids, vec![1, 3, 9]);
}

#[test]
fn folder_round_trips_its_sync_state() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    assert!(store
        .folder(inbox)
        .unwrap()
        .unwrap()
        .highest_modseq
        .is_none());

    store
        .set_folder_sync_state(inbox, Some(1789), Some(42), Some(97))
        .unwrap();

    let folder = store.folder(inbox).unwrap().unwrap();
    assert_eq!(folder.uid_validity, Some(1789));
    assert_eq!(folder.uid_next, Some(42));
    assert_eq!(folder.highest_modseq, Some(97));
    assert_eq!(folder.name, "INBOX");

    assert!(store.folder(9999).unwrap().is_none());
}

#[test]
fn an_oauth_account_round_trips_its_app_registration() {
    let store = Store::open_in_memory().unwrap();
    store
        .add_account(&NewAccount {
            label: "Work".into(),
            email: "work@example.com".into(),
            imap_host: "outlook.office365.com".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "work@example.com".into(),
            auth_method: "oauth2".into(),
            oauth_client_id: Some("11111111-2222-3333-4444-555555555555".into()),
            oauth_tenant: Some("common".into()),
            smtp: None,
        })
        .unwrap();

    let account = store.account_by_email("work@example.com").unwrap().unwrap();
    assert_eq!(account.auth_method, "oauth2");
    assert_eq!(account.oauth_tenant.as_deref(), Some("common"));
    assert_eq!(
        account.oauth_client_id.as_deref(),
        Some("11111111-2222-3333-4444-555555555555")
    );

    // A password account leaves them unset rather than storing empty strings.
    store
        .add_account(&NewAccount {
            label: "Personal".into(),
            email: "me@example.de".into(),
            imap_host: "imap.example.de".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "me@example.de".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();

    let plain = store.account_by_email("me@example.de").unwrap().unwrap();
    assert!(plain.oauth_client_id.is_none());
    assert!(plain.oauth_tenant.is_none());
}

#[test]
fn an_account_round_trips_its_submission_endpoint() {
    let store = Store::open_in_memory().unwrap();
    store
        .add_account(&NewAccount {
            label: "Personal".into(),
            email: "me@example.de".into(),
            imap_host: "imap.example.de".into(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            username: "me@example.de".into(),
            auth_method: "app_password".into(),
            smtp: Some(SmtpConfig {
                host: "smtp.example.de".into(),
                port: 587,
                security: SmtpSecurity::StartTls,
            }),
            ..Default::default()
        })
        .unwrap();

    let account = store.account_by_email("me@example.de").unwrap().unwrap();
    let smtp = account.smtp.expect("submission endpoint");
    assert_eq!(smtp.host, "smtp.example.de");
    assert_eq!(smtp.port, 587);
    assert_eq!(smtp.security, SmtpSecurity::StartTls);
}

#[test]
fn an_account_registered_before_send_existed_can_gain_an_endpoint() {
    // Every account created up to schema v2 is in this state: it syncs, but
    // has nowhere to submit to. It must not need re-creating to send.
    let (store, account_id) = store_with_account();
    assert!(store
        .account_by_email("dev@fuckmail.test")
        .unwrap()
        .unwrap()
        .smtp
        .is_none());

    store
        .set_smtp(
            account_id,
            Some(&SmtpConfig {
                host: "127.0.0.1".into(),
                port: 1025,
                security: SmtpSecurity::Plaintext,
            }),
        )
        .unwrap();

    let account = store
        .account_by_email("dev@fuckmail.test")
        .unwrap()
        .unwrap();
    assert_eq!(account.smtp.as_ref().map(|s| s.port), Some(1025));

    // And can lose it again, which is how sending gets turned off.
    store.set_smtp(account_id, None).unwrap();
    assert!(store
        .account_by_email("dev@fuckmail.test")
        .unwrap()
        .unwrap()
        .smtp
        .is_none());
}

#[test]
fn setting_an_endpoint_on_an_unknown_account_is_an_error() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.set_smtp(4242, None).is_err());
}

#[test]
fn a_message_can_be_found_by_its_message_id_with_or_without_brackets() {
    // How a reply finds the message it is answering.
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();

    let bare = "newsletter-2026-w36@news.rustweekly.example";
    let found = store.message_by_rfc822_id(account, bare).unwrap().unwrap();
    assert_eq!(found.subject.as_deref(), Some("Rust Weekly #612"));
    assert!(found.body_path.is_none());

    let bracketed = store
        .message_by_rfc822_id(account, &format!("<{bare}>"))
        .unwrap()
        .unwrap();
    assert_eq!(bracketed.id, found.id);

    assert!(store
        .message_by_rfc822_id(account, "nope@example.com")
        .unwrap()
        .is_none());
    // Scoped to the account, like every other lookup here.
    assert!(store.message_by_rfc822_id(4242, bare).unwrap().is_none());
}

fn queued_move(account: AccountId, message: MessageId, folder: FolderId) -> NewOperation {
    NewOperation {
        account_id: account,
        message_id: message,
        kind: OperationKind::Move {
            target_folder: "Archive".into(),
        },
        source_folder_id: folder,
        source_uid: 2,
        source_uid_validity: Some(1234),
        expect_message_id: Some("newsletter-2026-w36@news.rustweekly.example".into()),
        execute_after: 0,
    }
}

/// A store with one message in INBOX, which is what a mutation needs a handle on.
fn store_with_message() -> (Store, AccountId, MessageId, FolderId) {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let (message, _) = store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: String::new(),
            }),
        )
        .unwrap();
    (store, account, message, inbox)
}

#[test]
fn a_queued_operation_records_what_it_expects_to_find() {
    // The source coordinates are the whole point of the queue: without them
    // the executor would be acting on a UID it cannot verify.
    let (store, account, message, inbox) = store_with_message();
    store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();

    let pending = store.pending_operations(account).unwrap();
    assert_eq!(pending.len(), 1);
    let op = &pending[0];
    assert_eq!(
        op.kind,
        OperationKind::Move {
            target_folder: "Archive".into()
        }
    );
    assert_eq!(op.source_uid, 2);
    assert_eq!(op.source_uid_validity, Some(1234));
    assert_eq!(
        op.expect_message_id.as_deref(),
        Some("newsletter-2026-w36@news.rustweekly.example")
    );
    assert_eq!(op.state, OperationState::Pending);
    assert_eq!(op.attempts, 0);
}

#[test]
fn an_operation_is_not_due_until_its_undo_window_has_elapsed() {
    let (store, account, message, inbox) = store_with_message();
    store
        .enqueue_operation(&NewOperation {
            execute_after: 1_000,
            ..queued_move(account, message, inbox)
        })
        .unwrap();

    // Visible as queued straight away — but not yet something to send.
    assert_eq!(store.pending_operations(account).unwrap().len(), 1);
    assert!(store.due_operations(account, 999).unwrap().is_empty());
    assert_eq!(store.due_operations(account, 1_000).unwrap().len(), 1);
}

#[test]
fn undo_cancels_the_last_thing_queued_and_only_while_it_is_pending() {
    let (store, account, message, inbox) = store_with_message();
    let first = store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();
    let second = store
        .enqueue_operation(&NewOperation {
            kind: OperationKind::Flag {
                flag: "\\Seen".into(),
                set: true,
            },
            ..queued_move(account, message, inbox)
        })
        .unwrap();

    let undone = store.cancel_latest_operation(account).unwrap().unwrap();
    assert_eq!(undone.id, second);

    let remaining = store.pending_operations(account).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, first);

    // Once it has been sent, undo can no longer reach it — the server has
    // already been told, and pretending otherwise would be a lie.
    store
        .settle_operation(first, OperationState::Done, None)
        .unwrap();
    assert!(!store.cancel_operation(first).unwrap());
    assert!(store.cancel_latest_operation(account).unwrap().is_none());
}

#[test]
fn re_flagging_a_message_supersedes_the_pending_change_rather_than_queueing_both() {
    // Mark read, change your mind, mark unread. Two operations racing to the
    // server would leave the outcome down to ordering luck.
    let (store, account, message, inbox) = store_with_message();
    let mark_read = store
        .enqueue_operation(&NewOperation {
            kind: OperationKind::Flag {
                flag: "\\Seen".into(),
                set: true,
            },
            ..queued_move(account, message, inbox)
        })
        .unwrap();
    let mark_unread = store
        .enqueue_operation(&NewOperation {
            kind: OperationKind::Flag {
                flag: "\\Seen".into(),
                set: false,
            },
            ..queued_move(account, message, inbox)
        })
        .unwrap();

    let pending = store.pending_operations(account).unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].id, mark_unread);
    assert_eq!(
        pending[0].kind,
        OperationKind::Flag {
            flag: "\\Seen".into(),
            set: false
        }
    );

    // A different flag is a different operation and is left alone.
    store
        .enqueue_operation(&NewOperation {
            kind: OperationKind::Flag {
                flag: "\\Flagged".into(),
                set: true,
            },
            ..queued_move(account, message, inbox)
        })
        .unwrap();
    assert_eq!(store.pending_operations(account).unwrap().len(), 2);

    // And the superseded one is settled, not silently deleted.
    let settled: String = store
        .connection()
        .query_row(
            "SELECT state FROM operation WHERE id = ?1",
            [mark_read],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(settled, "cancelled");
}

#[test]
fn a_failed_attempt_is_counted_and_can_stay_pending_for_a_retry() {
    let (store, account, message, inbox) = store_with_message();
    let op = store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();

    store
        .settle_operation(op, OperationState::Pending, Some("connection reset"))
        .unwrap();

    let pending = store.pending_operations(account).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(pending[0].last_error.as_deref(), Some("connection reset"));
}

#[test]
fn a_message_that_leaves_the_store_takes_its_queued_operations_with_it() {
    // If sync finds the message gone from the server, acting on it is moot.
    let (store, account, message, inbox) = store_with_message();
    store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();

    store
        .connection()
        .execute("DELETE FROM message WHERE id = ?1", [message])
        .unwrap();
    assert!(store.pending_operations(account).unwrap().is_empty());
}
