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
            })
            .unwrap()
    };

    let store = Store::open(&path).unwrap();
    assert_eq!(store.accounts().unwrap().len(), 1);
    assert_eq!(store.message_count(account).unwrap(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}
