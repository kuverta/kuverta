use core_store::{ImapSecurity, Location, NewAccount, NewMessage, Store, Urgency};

#[test]
fn opening_an_old_store_repairs_model_verdicts_that_claim_no_action_is_urgent() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kuverta-urgency-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let db = dir.join("test.db");
    let store = Store::open(&db).unwrap();
    let account = store
        .add_account(&NewAccount {
            email: "dev@example.test".into(),
            label: "Test".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 143,
            imap_security: ImapSecurity::Plaintext,
            username: "dev@example.test".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let mut ids = Vec::new();
    for (index, (source, action, deadline)) in [
        ("model", Some("none"), None),
        ("model", Some("none"), Some("2026-10-16")),
        ("model", Some("pay"), Some("2026-10-16")),
        ("model", Some("read"), None),
        ("model", None, None),
        ("rules", Some("read"), None),
    ]
    .into_iter()
    .enumerate()
    {
        let id = store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("urgency-{index}@example.test")),
                    subject: Some(format!("Case {index}")),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: index as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap()
            .0;
        store
            .record_urgency(
                id,
                &Urgency {
                    score: 3,
                    reason: "Keep this explanation.".into(),
                    action: action.map(str::to_string),
                    deadline: deadline.map(str::to_string),
                    source: source.into(),
                    model: Some("test-model".into()),
                },
            )
            .unwrap();
        ids.push(id);
    }
    store
        .connection()
        .execute_batch("PRAGMA user_version = 24")
        .unwrap();
    drop(store);

    let store = Store::open(&db).unwrap();
    for &id in &ids[..2] {
        let verdict = store.urgency_of(id).unwrap().unwrap();
        assert_eq!(verdict.score, 0);
        assert_eq!(verdict.deadline, None);
        assert_eq!(verdict.reason, "Keep this explanation.");
        assert_eq!(verdict.model.as_deref(), Some("test-model"));
    }
    let urgent = store.urgent_messages(account, 2, 10).unwrap();
    assert_eq!(
        urgent.len(),
        4,
        "legitimate action and missing action remain urgent"
    );
    assert!(urgent.iter().all(|m| ids[2..].contains(&m.summary.id)));
    assert_eq!(store.message_count(account).unwrap(), 6);
    assert_eq!(store.location_count(account).unwrap(), 6);
    drop(store);
    // Reopening is safe and does not reintroduce old scores.
    let store = Store::open(&db).unwrap();
    assert_eq!(store.urgency_of(ids[0]).unwrap().unwrap().score, 0);
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}
