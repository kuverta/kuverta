//! Store behaviour that is expensive to get wrong, exercised end to end.

use core_store::model::*;
use core_store::{Store, Upsert};

fn store_with_account() -> (Store, AccountId) {
    let store = Store::open_in_memory().expect("open store");
    let id = store
        .add_account(&NewAccount {
            label: "Dev".into(),
            email: "dev@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 10143,
            imap_security: ImapSecurity::Plaintext,
            username: "dev@kuverta.test".into(),
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
            email: "work@kuverta.test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 10143,
            imap_security: ImapSecurity::Plaintext,
            username: "work@kuverta.test".into(),
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
    let dir = std::env::temp_dir().join(format!("kuverta-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");

    let account = {
        let store = Store::open(&path).unwrap();
        store
            .add_account(&NewAccount {
                label: "Dev".into(),
                email: "dev@kuverta.test".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: 10143,
                imap_security: ImapSecurity::Plaintext,
                username: "dev@kuverta.test".into(),
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
            oauth_provider: Some("microsoft".into()),
        })
        .unwrap();

    let account = store.account_by_email("work@example.com").unwrap().unwrap();
    assert_eq!(account.auth_method, "oauth2");
    // Which grant to use is not guessable from the rest of the row: Microsoft
    // takes the device flow and Google cannot.
    assert_eq!(account.oauth_provider.as_deref(), Some("microsoft"));
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
        .account_by_email("dev@kuverta.test")
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

    let account = store.account_by_email("dev@kuverta.test").unwrap().unwrap();
    assert_eq!(account.smtp.as_ref().map(|s| s.port), Some(1025));

    // And can lose it again, which is how sending gets turned off.
    store.set_smtp(account_id, None).unwrap();
    assert!(store
        .account_by_email("dev@kuverta.test")
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

#[test]
fn exclusions_match_a_folder_by_name_or_by_attribute() {
    let patterns = vec!["\\All".to_string(), "Report spam".to_string()];

    // By attribute, whatever the provider calls the folder.
    assert!(folder_is_excluded(
        &patterns,
        "[Gmail]/All Mail",
        Some("\\All")
    ));
    assert!(folder_is_excluded(
        &patterns,
        "Alle Nachrichten",
        Some("\\All")
    ));

    // By name, case-insensitively, as every other folder comparison here is.
    assert!(folder_is_excluded(&patterns, "report spam", None));

    // And nothing else.
    assert!(!folder_is_excluded(&patterns, "INBOX", None));
    assert!(!folder_is_excluded(
        &patterns,
        "[Gmail]/Trash",
        Some("\\Trash")
    ));
    // A name that looks like the attribute's folder must not match the
    // attribute pattern: `\All` means the attribute, not the word.
    assert!(!folder_is_excluded(&["\\All".to_string()], "All", None));
}

#[test]
fn excluding_a_folder_is_idempotent_and_reversible() {
    let (store, account) = store_with_account();
    assert!(store.folder_exclusions(account).unwrap().is_empty());

    assert!(store.exclude_folder(account, "\\All").unwrap());
    assert!(!store.exclude_folder(account, "\\All").unwrap());
    assert_eq!(store.folder_exclusions(account).unwrap(), vec!["\\All"]);

    assert!(store.include_folder(account, "\\All").unwrap());
    assert!(!store.include_folder(account, "\\All").unwrap());
    assert!(store.folder_exclusions(account).unwrap().is_empty());
}

/// Five messages, alternating read/unread, with categories on two of them.
fn store_with_a_list() -> (Store, AccountId) {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();

    for n in 0..5 {
        let (id, _) = store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("list-{n}@example.com")),
                    subject: Some(format!("message {n}")),
                    from_addr: Some("sender@example.com".into()),
                    date_utc: Some(1_700_000_000 + n as i64),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: n as u32 + 1,
                    // Even ones have been read.
                    flags: if n % 2 == 0 {
                        "\\Seen".into()
                    } else {
                        String::new()
                    },
                }),
            )
            .unwrap();

        if n < 2 {
            store
                .record_verdict(
                    id,
                    &Verdict {
                        category: "transactional".into(),
                        confidence: Some(0.9),
                        source: ClassifierSource::Rules,
                        model: None,
                        latency_ms: None,
                    },
                )
                .unwrap();
        }
    }
    (store, account)
}

#[test]
fn a_message_window_pages_newest_first_and_reports_the_total() {
    // The total has to come back with the window: it is what a virtualized
    // list sizes its scrollbar against, and a second round trip for it could
    // disagree with the rows it was meant to describe.
    let (store, account) = store_with_a_list();
    let all = ListFilter::default();

    let first = store.message_window(account, 0, 2, &all).unwrap();
    assert_eq!(first.total, 5);
    assert_eq!(first.offset, 0);
    let subjects: Vec<&str> = first
        .messages
        .iter()
        .map(|m| m.summary.subject.as_deref().unwrap())
        .collect();
    assert_eq!(subjects, vec!["message 4", "message 3"]);

    let second = store.message_window(account, 2, 2, &all).unwrap();
    assert_eq!(second.total, 5, "the total does not shrink with the window");
    let subjects: Vec<&str> = second
        .messages
        .iter()
        .map(|m| m.summary.subject.as_deref().unwrap())
        .collect();
    assert_eq!(subjects, vec!["message 2", "message 1"]);

    // Past the end is empty rather than an error, because a list can be
    // scrolled while a sync is removing rows underneath it.
    let past = store.message_window(account, 99, 2, &all).unwrap();
    assert!(past.messages.is_empty());
    assert_eq!(past.total, 5);
}

#[test]
fn the_window_reports_unread_from_the_flags_of_every_copy() {
    let (store, account) = store_with_a_list();
    let window = store
        .message_window(account, 0, 10, &ListFilter::default())
        .unwrap();

    let unread: Vec<bool> = window.messages.iter().map(|m| m.unread).collect();
    // Newest first: 4 (read), 3 (unread), 2 (read), 1 (unread), 0 (read).
    assert_eq!(unread, vec![false, true, false, true, false]);
}

#[test]
fn a_message_read_in_one_folder_is_not_unread_because_of_another() {
    // The Gmail case: one message, several folders. Two answers to "is this
    // unread" would show as a count that never settles.
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store.upsert_folder(account, "Archive", None).unwrap();

    let (id, _) = store
        .upsert_message(
            account,
            &newsletter(),
            Some(&Location {
                folder_id: inbox,
                uid: 1,
                flags: "\\Seen".into(),
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
    assert_eq!(store.locations_of(id).unwrap().len(), 2);

    let window = store
        .message_window(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(window.total, 1);
    assert!(
        !window.messages[0].unread,
        "read anywhere means read everywhere"
    );
}

#[test]
fn the_window_can_be_narrowed_by_category_and_by_unread() {
    let (store, account) = store_with_a_list();

    let filed = store
        .message_window(
            account,
            0,
            10,
            &ListFilter {
                category: Some("transactional".into()),
                unread_only: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(filed.total, 2);
    assert!(filed
        .messages
        .iter()
        .all(|m| m.category.as_deref() == Some("transactional")));

    let unread = store
        .message_window(
            account,
            0,
            10,
            &ListFilter {
                category: None,
                unread_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(unread.total, 2);
    assert!(unread.messages.iter().all(|m| m.unread));

    // Filters combine rather than override: message 1 is the only unread one
    // that was also classified.
    let both = store
        .message_window(
            account,
            0,
            10,
            &ListFilter {
                category: Some("transactional".into()),
                unread_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(both.total, 1);
    assert_eq!(
        both.messages[0].summary.subject.as_deref(),
        Some("message 1")
    );
}

#[test]
fn folder_summaries_count_messages_and_read_in_the_conventional_order() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();
    let trash = store
        .upsert_folder(account, "Trash", Some("\\Trash"))
        .unwrap();
    let project = store.upsert_folder(account, "Projekte", None).unwrap();

    // One message in INBOX and Archive at once — the Gmail case. It counts
    // once in each, never twice in either.
    let (id, _) = store
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
    assert_eq!(store.locations_of(id).unwrap().len(), 2);

    // A second message, already read.
    store
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some("read@example.com".into()),
                subject: Some("already read".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: inbox,
                uid: 2,
                flags: "\\Seen".into(),
            }),
        )
        .unwrap();

    let folders = store.folder_summaries(account).unwrap();
    let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
    // INBOX first, the user's own folder above the bins, Trash last.
    assert_eq!(names, vec!["INBOX", "Archive", "Projekte", "Trash"]);

    // And the same order when the server declares nothing — which is what a
    // folder created from inside this app looks like, because Dovecot will not
    // let a client set a special-use attribute. Without the name fallback,
    // Archive would sort wherever the alphabet put it.
    let bare = store.upsert_folder(account, "Aardvark", None).unwrap();
    let _ = bare;
    let unmarked = FolderSummary {
        id: 0,
        name: "Archive".into(),
        special_use: None,
        total: 0,
        unread: 0,
    };
    assert_eq!(unmarked.rank(), 3, "an unmarked Archive still ranks as one");
    assert_eq!(
        FolderSummary {
            name: "[Gmail]/Sent Mail".into(),
            ..unmarked.clone()
        }
        .rank(),
        2,
        "the leaf is what is recognised, so Gmail's nesting still matches"
    );
    assert_eq!(
        FolderSummary {
            name: "Projekte".into(),
            ..unmarked.clone()
        }
        .rank(),
        4,
        "a folder of the user's own is not plumbing"
    );

    let by = |name: &str| folders.iter().find(|f| f.name == name).unwrap();
    assert_eq!(by("INBOX").total, 2);
    assert_eq!(by("INBOX").unread, 1, "one of the two has been read");
    assert_eq!(by("Archive").total, 1);
    assert_eq!(by("Archive").unread, 1);
    // An empty folder is still listed: it is somewhere to file to.
    assert_eq!(by("Trash").total, 0);
    assert_eq!(by("Trash").unread, 0);
    let _ = trash;
    let _ = project;
}

#[test]
fn the_window_can_be_narrowed_to_one_folder_and_combined_with_the_rest() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store.upsert_folder(account, "Archive", None).unwrap();

    for (n, folder) in [(0, inbox), (1, inbox), (2, archive)] {
        store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("f-{n}@example.com")),
                    subject: Some(format!("message {n}")),
                    date_utc: Some(1_700_000_000 + n),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: folder,
                    uid: n as u32 + 1,
                    flags: if n == 0 {
                        "\\Seen".into()
                    } else {
                        String::new()
                    },
                }),
            )
            .unwrap();
    }

    let in_folder = |folder| ListFilter {
        folder: Some(folder),
        ..Default::default()
    };
    assert_eq!(
        store
            .message_window(account, 0, 10, &in_folder(inbox))
            .unwrap()
            .total,
        2
    );
    assert_eq!(
        store
            .message_window(account, 0, 10, &in_folder(archive))
            .unwrap()
            .total,
        1
    );

    // Folder and unread narrow together rather than one replacing the other.
    let unread_in_inbox = store
        .message_window(
            account,
            0,
            10,
            &ListFilter {
                folder: Some(inbox),
                unread_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(unread_in_inbox.total, 1);
    assert_eq!(
        unread_in_inbox.messages[0].summary.subject.as_deref(),
        Some("message 1")
    );
}

#[test]
fn an_account_can_be_edited_without_restating_what_did_not_change() {
    // How a settings form works: load, change one field, write back.
    let (store, account) = store_with_account();
    let before = store.account_by_email("dev@kuverta.test").unwrap().unwrap();

    let mut edit = NewAccount::from(&before);
    edit.imap_port = 993;
    edit.imap_security = ImapSecurity::Tls;
    store.update_account(account, &edit).unwrap();

    let after = store.account_by_email("dev@kuverta.test").unwrap().unwrap();
    assert_eq!(after.imap_port, 993);
    assert_eq!(after.imap_security, ImapSecurity::Tls);
    // Untouched fields survive.
    assert_eq!(after.label, before.label);
    assert_eq!(after.username, before.username);
    assert_eq!(after.id, before.id);

    assert!(store.update_account(4242, &edit).is_err());
}

#[test]
fn deleting_an_account_takes_its_mail_and_its_queue_with_it() {
    let (store, account, message, inbox) = store_with_message();
    store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();
    store.exclude_folder(account, "\\All").unwrap();
    store
        .record_correction(message, Some("notification"), "personal")
        .unwrap();
    assert_eq!(store.message_count(account).unwrap(), 1);

    store.delete_account(account).unwrap();

    assert!(store.accounts().unwrap().is_empty());
    assert_eq!(store.message_count(account).unwrap(), 0);
    assert!(store.folders(account).unwrap().is_empty());
    assert!(store.pending_operations(account).unwrap().is_empty());
    assert!(store.folder_exclusions(account).unwrap().is_empty());

    let corrections: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM correction", [], |row| row.get(0))
        .unwrap();
    assert_eq!(corrections, 0, "corrections cascade with their message");

    assert!(store.delete_account(account).is_err());
}

// -- the category a message is shown under ---------------------------------

/// A message in the inbox with one rules verdict recorded against it.
fn classified(store: &Store, account: AccountId, category: &str) -> MessageId {
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let (id, _) = store
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
    store
        .record_verdict(
            id,
            &Verdict {
                category: category.into(),
                confidence: Some(0.9),
                source: ClassifierSource::Rules,
                model: None,
                latency_ms: Some(0),
            },
        )
        .unwrap();
    id
}

fn user_says(store: &Store, id: MessageId, category: &str) {
    store
        .record_verdict(
            id,
            &Verdict {
                category: category.into(),
                confidence: Some(1.0),
                source: ClassifierSource::User,
                model: None,
                latency_ms: None,
            },
        )
        .unwrap();
}

#[test]
fn a_correction_outranks_the_rules_verdict_in_the_list() {
    // Filing a message has to change what the list shows straight away.
    // Waiting for the next sync to agree would make the act of filing look
    // like it had not worked.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    user_says(&store, id, "marketing");

    let window = store
        .message_window(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(window.messages[0].category.as_deref(), Some("marketing"));
    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("marketing")
    );
}

#[test]
fn a_later_rules_verdict_does_not_overturn_the_user() {
    // The next sync reclassifies, and with the correction loaded it should
    // reach the same answer anyway. But if it ever does not — a rule changes,
    // the sender starts sending something else — the user's decision is still
    // the one that stands.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");
    user_says(&store, id, "marketing");

    store
        .record_verdict(
            id,
            &Verdict {
                category: "notification".into(),
                confidence: Some(0.8),
                source: ClassifierSource::Rules,
                model: None,
                latency_ms: Some(0),
            },
        )
        .unwrap();

    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("marketing")
    );
}

#[test]
fn the_most_recent_correction_is_the_one_that_counts() {
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    user_says(&store, id, "marketing");
    user_says(&store, id, "transactional");

    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("transactional")
    );
}

#[test]
fn a_model_verdict_never_changes_what_the_list_shows() {
    // The model is recorded beside the rules so `disagreements` can compare
    // them. A model that quietly moved messages would take that measurement
    // with it, and there would be nothing left to answer "was it worth the
    // latency" with.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    store
        .record_verdict(
            id,
            &Verdict {
                category: "marketing".into(),
                confidence: Some(0.7),
                source: ClassifierSource::Model,
                model: Some("qwen3:8b".into()),
                latency_ms: Some(400),
            },
        )
        .unwrap();

    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("newsletter")
    );
}

#[test]
fn a_reclassified_message_is_counted_under_one_category_only() {
    // Every verdict is kept, so a message classified twice has several rows.
    // Joining all of them counted it under every category it had ever been
    // given, and the sidebar added up to more messages than the mailbox held.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");
    user_says(&store, id, "marketing");

    let counts = store.category_counts(account, None).unwrap();
    let total: usize = counts.iter().map(|(_, n)| n).sum();

    assert_eq!(total, 1, "one message should be counted once: {counts:?}");
    assert_eq!(counts, vec![("marketing".to_string(), 1)]);
}

#[test]
fn a_category_filter_follows_the_correction() {
    // Otherwise filing a message into a category and then opening that
    // category would not show it.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");
    user_says(&store, id, "marketing");

    let filter = ListFilter {
        category: Some("marketing".into()),
        ..Default::default()
    };
    let window = store.message_window(account, 0, 10, &filter).unwrap();
    assert_eq!(window.total, 1);
    assert_eq!(window.messages[0].summary.id, id);

    let gone = ListFilter {
        category: Some("newsletter".into()),
        ..Default::default()
    };
    assert_eq!(
        store.message_window(account, 0, 10, &gone).unwrap().total,
        0,
        "it should have left the category it was filed out of"
    );
}

#[test]
fn an_unclassified_message_has_no_category() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let (id, _) = store
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

    assert_eq!(store.current_category(id).unwrap(), None);
    let window = store
        .message_window(account, 0, 10, &ListFilter::default())
        .unwrap();
    assert_eq!(window.messages[0].category, None);
}

// -- an assistant's filings ------------------------------------------------

fn agent_says(store: &Store, id: MessageId, category: &str) {
    store
        .record_verdict(
            id,
            &Verdict {
                category: category.into(),
                confidence: None,
                source: ClassifierSource::Agent,
                model: None,
                latency_ms: None,
            },
        )
        .unwrap();
}

#[test]
fn an_assistants_filing_outranks_the_rules() {
    // An assistant that files mail and nothing visibly changes is useless.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    agent_says(&store, id, "marketing");

    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("marketing")
    );
}

#[test]
fn the_user_outranks_the_assistant() {
    // Whatever order they arrive in. A person's filing is the one that is
    // definitely right; an assistant filing afterwards does not overturn it.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    user_says(&store, id, "transactional");
    agent_says(&store, id, "marketing");

    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("transactional")
    );
}

#[test]
fn the_users_own_filing_ignores_the_assistant() {
    // "Has the user already said so?" is a different question from "is this
    // where it is shown?" once an assistant can file. Conflating them would
    // make a user confirming a suggestion record nothing.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    agent_says(&store, id, "marketing");

    assert_eq!(store.user_category(id).unwrap(), None);
    assert_eq!(
        store.current_category(id).unwrap().as_deref(),
        Some("marketing")
    );
}

#[test]
fn an_assistants_filing_is_never_learned_from() {
    // §3.4: corrections are the one signal that is definitely right. An
    // assistant's judgement is not that, and must not reach the classifier.
    let (store, account) = store_with_account();
    let id = classified(&store, account, "newsletter");

    agent_says(&store, id, "marketing");

    assert!(store.learned_categories(account).unwrap().is_empty());
}

// -- corrections to post ---------------------------------------------------

#[test]
fn every_address_gets_a_token_key_no_other_store_shares() {
    // Row ids start at 1 in every store, and the token used to be filed under
    // the id: `.devdata` and the real store shared `paper:1`.
    let (first, _) = store_with_account();
    let (second, _) = store_with_account();
    let a = first.paper_mailbox(paper_mailbox(&first)).unwrap().unwrap();
    let b = second
        .paper_mailbox(paper_mailbox(&second))
        .unwrap()
        .unwrap();

    assert_eq!(a.id, b.id, "the collision needs the same id in both stores");
    assert_ne!(a.token_key, b.token_key);
    assert_eq!(a.token_key.len(), 32, "{}", a.token_key);
}

#[test]
fn a_token_key_survives_every_change_to_its_address() {
    // A key that changed would orphan the token, exactly as keying on the URL
    // would have.
    let (store, _) = store_with_account();
    let id = paper_mailbox(&store);
    let before = store.paper_mailbox(id).unwrap().unwrap().token_key;

    // Saved again at the same target: the upsert's update path.
    paper_mailbox(&store);
    store
        .update_paper_mailbox(
            id,
            &core_store::NewPaperMailbox {
                label: "Zuhause".into(),
                base_url: "http://elsewhere.local:8000".into(),
                selector_kind: "tag".into(),
                selector_value: Some("home".into()),
            },
        )
        .unwrap();

    let after = store.paper_mailbox(id).unwrap().unwrap();
    assert_eq!(after.base_url, "http://elsewhere.local:8000");
    assert_eq!(after.token_key, before);
}

#[test]
fn addresses_saved_before_token_keys_get_one_when_the_store_is_opened() {
    let dir = std::env::temp_dir().join(format!("kuverta-store-v9-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");

    let id = paper_mailbox(&Store::open(&path).unwrap());

    // Put the file back the way a v8 store left it.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE urgency;
             DROP INDEX message_by_reply;
             DROP TABLE unsubscription;
             DROP TABLE smart_mailbox;
             DROP TABLE outbox;
             DROP INDEX message_unread_headers;
             ALTER TABLE message DROP COLUMN headers_read;
             ALTER TABLE message DROP COLUMN list_unsubscribe_post;
             ALTER TABLE message DROP COLUMN list_unsubscribe;
             ALTER TABLE message DROP COLUMN recipients;
             DROP TABLE ai_task;
             DROP TABLE ai_provider;
             DROP TABLE paper_read;
             DROP TABLE paper_transcript;
             DROP INDEX paper_mailbox_token_key;
             ALTER TABLE paper_mailbox DROP COLUMN token_key;
             PRAGMA user_version = 8;",
        )
        .unwrap();
    }

    let store = Store::open(&path).unwrap();
    let mailbox = store.paper_mailbox(id).unwrap().unwrap();
    assert_eq!(mailbox.token_key.len(), 32, "{}", mailbox.token_key);
    assert_ne!(mailbox.token_key, id.to_string());

    let _ = std::fs::remove_dir_all(&dir);
}

fn paper_mailbox(store: &Store) -> i64 {
    store
        .upsert_paper_mailbox(&core_store::NewPaperMailbox {
            label: "Home".into(),
            base_url: "http://localhost:8000".into(),
            selector_kind: "everything".into(),
            selector_value: None,
        })
        .unwrap()
}

#[test]
fn the_latest_filing_of_a_document_is_the_one_that_stands() {
    let (store, _account) = store_with_account();
    let home = paper_mailbox(&store);

    store
        .record_paper_correction(
            home,
            41,
            Some("Stadtwerke"),
            Some("personal"),
            "transactional",
        )
        .unwrap();
    store
        .record_paper_correction(
            home,
            41,
            Some("Stadtwerke"),
            Some("transactional"),
            "notification",
        )
        .unwrap();

    assert_eq!(
        store.paper_overrides(home).unwrap(),
        vec![(41, "notification".to_string())]
    );
    // Both events are kept: a reversal is the most interesting row there is.
    let events: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM paper_correction", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(events, 2);
}

#[test]
fn a_correspondent_is_learned_from_its_latest_filing() {
    let (store, _account) = store_with_account();
    let home = paper_mailbox(&store);

    store
        .record_paper_correction(home, 1, Some("Finanzamt"), None, "notification")
        .unwrap();
    store
        .record_paper_correction(home, 2, Some("Finanzamt"), None, "transactional")
        .unwrap();
    // No correspondent: it pins its document and teaches nothing.
    store
        .record_paper_correction(home, 3, None, None, "personal")
        .unwrap();

    assert_eq!(
        store.paper_learned().unwrap(),
        vec![("Finanzamt".to_string(), "transactional".to_string())]
    );
}

#[test]
fn removing_an_address_removes_its_corrections() {
    let (store, _account) = store_with_account();
    let home = paper_mailbox(&store);
    store
        .record_paper_correction(home, 1, Some("Finanzamt"), None, "transactional")
        .unwrap();

    store.delete_paper_mailbox(home).unwrap();

    assert!(store.paper_learned().unwrap().is_empty());
}

#[test]
fn a_window_can_be_read_from_the_oldest_end() {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    for n in 0..3i64 {
        store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("o-{n}@example.com")),
                    subject: Some(format!("message {n}")),
                    date_utc: Some(1_700_000_000 + n),
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
    let subjects = |oldest_first| {
        store
            .message_window(
                account,
                0,
                10,
                &ListFilter {
                    oldest_first,
                    ..Default::default()
                },
            )
            .unwrap()
            .messages
            .into_iter()
            .map(|listed| listed.summary.subject.unwrap_or_default())
            .collect::<Vec<_>>()
    };

    assert_eq!(subjects(false), ["message 2", "message 1", "message 0"]);
    assert_eq!(subjects(true), ["message 0", "message 1", "message 2"]);
}

#[test]
fn categories_are_counted_in_the_folder_they_are_asked_about() {
    // A category means something different in the Inbox than in Sent, and the
    // sidebar counts what the list would show.
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let sent = store.upsert_folder(account, "Sent", None).unwrap();

    for (n, folder) in [(0, inbox), (1, inbox), (2, sent)] {
        let (id, _) = store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("c-{n}@example.com")),
                    subject: Some(format!("message {n}")),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: folder,
                    uid: n as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap();
        store
            .record_verdict(
                id,
                &Verdict {
                    category: "transactional".into(),
                    confidence: Some(0.9),
                    source: ClassifierSource::Rules,
                    model: None,
                    latency_ms: Some(0),
                },
            )
            .unwrap();
    }

    assert_eq!(
        store.category_counts(account, None).unwrap(),
        vec![("transactional".to_string(), 3)]
    );
    assert_eq!(
        store.category_counts(account, Some(inbox)).unwrap(),
        vec![("transactional".to_string(), 2)]
    );
    assert_eq!(
        store.category_counts(account, Some(sent)).unwrap(),
        vec![("transactional".to_string(), 1)]
    );
}

#[test]
fn a_message_moved_but_not_yet_synced_leaves_the_list_it_was_moved_out_of() {
    // What someone sees: press delete, read "moved to Trash", and watch the
    // message sit there until the next sync. A move waits in the undo window
    // before it is sent, and the list showed the store rather than the ask.
    let (store, account, message, inbox) = store_with_message();
    let archive = store.upsert_folder(account, "Archive", None).unwrap();
    let all = ListFilter::default();
    let in_folder = |folder| ListFilter {
        folder: Some(folder),
        ..Default::default()
    };
    let total = |filter: &ListFilter| store.message_window(account, 0, 10, filter).unwrap().total;

    assert_eq!(total(&all), 1);
    assert_eq!(total(&in_folder(inbox)), 1);
    assert_eq!(total(&in_folder(archive)), 0);

    let operation = store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();

    // Gone from the Inbox and from every list, and already in the folder it
    // is on its way to.
    assert_eq!(total(&in_folder(inbox)), 0);
    assert_eq!(total(&all), 0);
    assert_eq!(total(&in_folder(archive)), 1);

    // Undone, it is back where it was.
    assert!(store.cancel_operation(operation).unwrap());
    assert_eq!(total(&in_folder(inbox)), 1);
    assert_eq!(total(&all), 1);
    assert_eq!(total(&in_folder(archive)), 0);
}

#[test]
fn a_queued_flag_change_leaves_the_message_where_it_is() {
    // Only moves take a message out of a list; marking it read does not.
    let (store, account, message, inbox) = store_with_message();
    store
        .enqueue_operation(&NewOperation {
            kind: OperationKind::Flag {
                flag: "\\Seen".into(),
                set: true,
            },
            ..queued_move(account, message, inbox)
        })
        .unwrap();

    assert_eq!(
        store
            .message_window(account, 0, 10, &ListFilter::default())
            .unwrap()
            .total,
        1
    );
}

#[test]
fn a_folder_badge_counts_a_queued_move_where_it_is_going() {
    // Otherwise the sidebar still counted the message in the Inbox it had
    // just been moved out of, and the badge disagreed with the list.
    let (store, account, message, inbox) = store_with_message();
    // Created so there is somewhere for the move to land.
    store.upsert_folder(account, "Archive", None).unwrap();
    let count = |name: &str| {
        let folders = store.folder_summaries(account).unwrap();
        let folder = folders.into_iter().find(|f| f.name == name).unwrap();
        (folder.total, folder.unread)
    };

    assert_eq!(count("INBOX"), (1, 1));
    assert_eq!(count("Archive"), (0, 0));

    let operation = store
        .enqueue_operation(&queued_move(account, message, inbox))
        .unwrap();
    assert_eq!(count("INBOX"), (0, 0));
    assert_eq!(count("Archive"), (1, 1));

    assert!(store.cancel_operation(operation).unwrap());
    assert_eq!(count("INBOX"), (1, 1));
    assert_eq!(count("Archive"), (0, 0));
}
