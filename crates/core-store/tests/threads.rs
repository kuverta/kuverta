use core_store::{ListFilter, Location, NewAccount, NewMessage, Store};

fn account(store: &Store, email: &str) -> i64 {
    store
        .add_account(&NewAccount {
            email: email.into(),
            label: email.into(),
            username: email.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap()
}

fn add(
    store: &Store,
    account: i64,
    folder: i64,
    n: u32,
    subject: &str,
    reply: Option<&str>,
    read: bool,
) -> i64 {
    store
        .upsert_message(
            account,
            &NewMessage {
                subject: Some(subject.into()),
                rfc822_message_id: Some(format!("m{n}@test")),
                in_reply_to: reply.map(str::to_string),
                date_utc: Some(i64::from(n)),
                ..Default::default()
            },
            Some(&Location {
                folder_id: folder,
                uid: n,
                flags: if read { "\\Seen" } else { "" }.into(),
            }),
        )
        .unwrap()
        .0
}

#[test]
fn subject_and_reply_groups_are_paged_before_returning_rows() {
    let store = Store::open_in_memory().unwrap();
    let a = account(&store, "one@test");
    let inbox = store.upsert_folder(a, "INBOX", None).unwrap();
    let sent = store.upsert_folder(a, "Sent", Some("\\Sent")).unwrap();
    let first = add(&store, a, inbox, 1, "  Angebot   Müller ", None, false);
    add(
        &store,
        a,
        sent,
        2,
        "AW: Re: angebot müller",
        Some("<m1@test>"),
        true,
    );
    add(&store, a, inbox, 3, "Neuer Betreff", Some("m2@test"), true);
    let other = add(&store, a, inbox, 4, "Anderes Thema", None, true);
    let filter = ListFilter {
        threaded: true,
        ..Default::default()
    };
    let page = store.message_window(a, 0, 1, &filter).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.messages[0].summary.id, other);
    let page = store.message_window(a, 1, 1, &filter).unwrap();
    assert_eq!(page.messages[0].thread_id, Some(first));
    assert_eq!(page.messages[0].thread_count, 3);
    assert!(
        page.messages[0].unread,
        "an older unread member keeps the group unread"
    );
    let only_unread = store
        .message_window(
            a,
            0,
            1,
            &ListFilter {
                unread_only: true,
                ..filter.clone()
            },
        )
        .unwrap();
    assert_eq!(only_unread.total, 1);
    assert_eq!(only_unread.messages[0].summary.id, first);
    let members = store
        .message_window(
            a,
            0,
            10,
            &ListFilter {
                thread_id: Some(first),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(members.total, 3, "includes the reply in Sent");
    assert!(members.messages.iter().all(|m| m.thread_id.is_none()));
    assert_eq!(
        store
            .message_window(a, 0, 10, &ListFilter::default())
            .unwrap()
            .total,
        4
    );
    let inbox_groups = store
        .message_window(
            a,
            0,
            10,
            &ListFilter {
                folder: Some(inbox),
                ..filter
            },
        )
        .unwrap();
    assert_eq!(inbox_groups.total, 2);
    assert_eq!(inbox_groups.messages[1].thread_count, 2);
}

#[test]
fn accounts_and_empty_subjects_stay_separate_and_new_mail_invalidates_the_cache() {
    let store = Store::open_in_memory().unwrap();
    let a = account(&store, "one@test");
    let b = account(&store, "two@test");
    let inbox = store.upsert_folder(a, "INBOX", None).unwrap();
    let other = store.upsert_folder(b, "INBOX", None).unwrap();
    add(&store, a, inbox, 1, "Thema", None, false);
    add(&store, b, other, 1, "Thema", None, false);
    add(&store, a, inbox, 2, "", None, false);
    add(&store, a, inbox, 3, "", None, false);
    let filter = ListFilter {
        threaded: true,
        ..Default::default()
    };
    let page = store.message_window_in(&[a, b], 0, 10, &filter).unwrap();
    assert_eq!(page.total, 4);
    add(&store, a, inbox, 4, "Re: Thema", None, false);
    let page = store.message_window_in(&[a, b], 0, 10, &filter).unwrap();
    assert_eq!(page.total, 4);
    assert_eq!(page.messages[0].thread_count, 2);
    assert_eq!(page.messages[0].account_id, a);
    assert_eq!(
        store.message_window_in(&[], 0, 10, &filter).unwrap().total,
        0
    );
}

#[test]
fn missing_parents_and_reply_cycles_do_not_loop_or_hide_messages() {
    let store = Store::open_in_memory().unwrap();
    let a = account(&store, "one@test");
    let inbox = store.upsert_folder(a, "INBOX", None).unwrap();
    add(&store, a, inbox, 1, "First", Some("missing@test"), false);
    add(&store, a, inbox, 2, "Second", Some("missing@test"), false);
    add(&store, a, inbox, 3, "Third", Some("m4@test"), false);
    add(&store, a, inbox, 4, "Fourth", Some("m3@test"), false);
    let groups = store
        .message_window(
            a,
            0,
            10,
            &ListFilter {
                threaded: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(groups.total, 2);
    assert!(groups.messages.iter().all(|m| m.thread_count == 2));
}

#[test]
fn another_connections_sync_invalidates_cached_thread_headers() {
    let path = std::env::temp_dir().join(format!("kuverta-threads-{}.db", std::process::id()));
    {
        let first = Store::open(&path).unwrap();
        let a = account(&first, "cache@test");
        let inbox = first.upsert_folder(a, "INBOX", None).unwrap();
        add(&first, a, inbox, 1, "First", None, false);
        let filter = ListFilter {
            threaded: true,
            ..Default::default()
        };
        assert_eq!(
            first.message_window(a, 0, 10, &filter).unwrap().messages[0].thread_count,
            1
        );
        let sync = Store::open(&path).unwrap();
        add(&sync, a, inbox, 2, "Re: First", Some("m1@test"), false);
        let groups = first.message_window(a, 0, 10, &filter).unwrap();
        assert_eq!(groups.total, 1);
        assert_eq!(groups.messages[0].thread_count, 2);
    }
    std::fs::remove_file(path).unwrap();
}
