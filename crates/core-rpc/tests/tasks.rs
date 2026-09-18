//! Tasks and the assistant's tools, against a real store and without a model:
//! what a task does to mail, that it does it once, that asking first leaves
//! proposals, and that the tools the assistant is given act as the window does.

use core_ai::ToolCall;
use core_rpc::{AssistantEvent, Core, TaskAction, TaskInput};
use core_store::model::*;
use core_store::{Blobs, SmartField, SmartOp, SmartQuery, SmartRule, Store};
use serde_json::json;

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An account with INBOX, Archive and Rechnungen; three invoices and two
/// messages from a person in the Inbox.
fn mailbox(name: &str) -> (Core, AccountId, TempDir) {
    let path = std::env::temp_dir().join(format!("kuverta-tasks-{}-{name}", std::process::id()));
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
    store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();
    store.upsert_folder(account, "Rechnungen", None).unwrap();
    let mail = [
        ("billing@acme.example", "Invoice 1001", "transactional"),
        ("billing@acme.example", "Invoice 1002", "transactional"),
        (
            "rechnung@stadtwerke.example",
            "Ihre Rechnung Juni",
            "transactional",
        ),
        ("erika@example.de", "Termin am Dienstag", "personal"),
        ("erika@example.de", "Fotos", "personal"),
    ];
    for (n, (from, subject, category)) in mail.iter().enumerate() {
        let (id, _) = store
            .upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("m{n}@x")),
                    subject: Some((*subject).into()),
                    from_addr: Some((*from).into()),
                    date_utc: Some(1_780_000_000 + n as i64),
                    search_text: Some(format!("{subject} body")),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: inbox,
                    uid: n as u32 + 1,
                    flags: String::new(),
                }),
            )
            .unwrap();
        store
            .record_verdict(
                id,
                &Verdict {
                    category: (*category).into(),
                    confidence: Some(0.9),
                    source: ClassifierSource::Rules,
                    model: None,
                    latency_ms: None,
                },
            )
            .unwrap();
    }
    (
        Core::new(store, Blobs::new(dir.0.join("blobs"))),
        account,
        dir,
    )
}

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

fn task(account: AccountId, action: TaskAction, review: bool, include_existing: bool) -> TaskInput {
    TaskInput {
        id: None,
        account_id: account,
        name: "Invoices".into(),
        query: invoices(),
        action,
        review,
        enabled: true,
        include_existing,
    }
}

#[test]
fn a_task_moves_what_its_rules_select_once() {
    let (core, account, _dir) = mailbox("moves");
    let id = core
        .save_task(&task(
            account,
            TaskAction::Move {
                folder: "Rechnungen".into(),
            },
            false,
            true,
        ))
        .unwrap();

    let runs = core.run_rule_tasks(account).unwrap();
    assert_eq!(runs[0].done, 3);
    assert_eq!(core.queue(account).unwrap().len(), 3, "three queued moves");
    assert!(core
        .queue(account)
        .unwrap()
        .iter()
        .all(|c| c.what == "move to Rechnungen"));

    // Once: a second run finds nothing new.
    assert_eq!(core.run_rule_tasks(account).unwrap()[0].done, 0);
    let view = &core.tasks(account).unwrap()[0];
    assert_eq!(view.id, id);
    assert_eq!(view.last_summary.as_deref(), Some("3 done"));
}

#[test]
fn a_task_from_now_on_leaves_what_is_already_there() {
    let (core, account, _dir) = mailbox("from-now");
    core.save_task(&task(account, TaskAction::Archive, false, false))
        .unwrap();
    assert_eq!(core.run_rule_tasks(account).unwrap()[0].done, 0);
}

#[test]
fn a_task_that_asks_first_leaves_proposals_to_approve() {
    let (core, account, _dir) = mailbox("review");
    core.save_task(&task(account, TaskAction::Archive, true, true))
        .unwrap();
    let runs = core.run_rule_tasks(account).unwrap();
    assert_eq!((runs[0].done, runs[0].proposed), (0, 3));
    assert!(core.queue(account).unwrap().is_empty(), "nothing done yet");

    let proposals = core.proposals(account).unwrap();
    assert_eq!(proposals.len(), 3);
    assert_eq!(proposals[0].task_name.as_deref(), Some("Invoices"));
    core.approve_proposal(account, proposals[0].id).unwrap();
    assert_eq!(core.queue(account).unwrap().len(), 1);
    assert!(
        core.approve_proposal(account, proposals[0].id).is_err(),
        "not twice"
    );
    core.settle_proposal(proposals[1].id, "rejected", None)
        .unwrap();
    assert_eq!(core.proposals(account).unwrap().len(), 1);
}

#[test]
fn tasks_are_refused_when_they_could_not_work() {
    let (core, account, _dir) = mailbox("refused");
    let err = core
        .save_task(&task(
            account,
            TaskAction::Move {
                folder: "Nowhere".into(),
            },
            false,
            false,
        ))
        .unwrap_err();
    assert!(err.to_string().contains("no folder Nowhere"));
    // A reply always asks first, whatever it was saved as.
    let id = core
        .save_task(&task(
            account,
            TaskAction::Reply {
                instruction: "answer politely".into(),
            },
            false,
            false,
        ))
        .unwrap();
    assert!(
        core.tasks(account)
            .unwrap()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .review
    );
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "1".into(),
        name: name.into(),
        arguments,
    }
}

#[test]
fn the_assistant_s_tools_look_and_act_as_the_window_does() {
    let (core, account, _dir) = mailbox("tools");

    let found = core.assistant_tool(
        account,
        &call(
            "find_mail",
            json!({"rules": [{"field": "subject", "op": "contains", "value": "invoice"}]}),
        ),
    );
    let found: serde_json::Value = serde_json::from_str(&found.content).unwrap();
    assert_eq!(found["total"], 2);
    let ids: Vec<i64> = found["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect();

    let moved = core.assistant_tool(
        account,
        &call("move_messages", json!({"ids": ids, "folder": "rechnungen"})),
    );
    match moved.event {
        Some(AssistantEvent::Changed { what, changes }) => {
            assert_eq!(what, "2 messages moved to rechnungen");
            assert_eq!(changes.len(), 2);
            // The window's Undo cancels exactly these.
            assert_eq!(core.cancel_changes(&changes).unwrap(), 2);
            assert!(core.queue(account).unwrap().is_empty());
        }
        other => panic!("expected a change, got {other:?}"),
    }

    let draft = core.assistant_tool(
        account,
        &call("draft_reply", json!({"id": ids[0], "body": "Danke!"})),
    );
    assert!(
        matches!(draft.event, Some(AssistantEvent::Draft { ref subject, .. }) if subject == "Re: Invoice 1002" || subject == "Re: Invoice 1001")
    );

    let created = core.assistant_tool(
        account,
        &call(
            "create_task",
            json!({
                "name": "Shop invoices",
                "rules": [{"field": "from", "op": "ends_with", "value": "@acme.example"}],
                "action": "move", "folder": "Rechnungen", "include_existing": true
            }),
        ),
    );
    assert!(
        matches!(created.event, Some(AssistantEvent::TaskCreated { .. })),
        "{}",
        created.content
    );

    let wrong = core.assistant_tool(
        account,
        &call("move_messages", json!({"ids": [], "folder": "Rechnungen"})),
    );
    assert!(matches!(wrong.event, Some(AssistantEvent::Failed { .. })));
    assert!(wrong.content.contains("error"));
}
