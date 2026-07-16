//! Smart mailboxes, the outbox, and what the Cleanup view asks for.

use core_store::model::*;
use core_store::{SmartField, SmartOp, SmartQuery, SmartRule, Store, Urgency};

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

struct Mail<'a> {
    id: &'a str,
    from: &'a str,
    subject: &'a str,
    days_ago: i64,
    seen: bool,
    list: Option<&'a str>,
    unsubscribe: Option<&'a str>,
    category: &'a str,
    folder: FolderId,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn put(store: &Store, account: AccountId, uid: u32, mail: &Mail<'_>) -> MessageId {
    let (id, _) = store
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some(mail.id.into()),
                subject: Some(mail.subject.into()),
                from_addr: Some(mail.from.into()),
                date_utc: Some(now() - mail.days_ago * 86_400),
                list_id: mail.list.map(Into::into),
                list_unsubscribe: mail.unsubscribe.map(Into::into),
                recipients: Some("dev@kuverta.test".into()),
                search_text: Some(format!("{} body text", mail.subject)),
                ..Default::default()
            },
            Some(&Location {
                folder_id: mail.folder,
                uid,
                flags: if mail.seen {
                    "\\Seen".into()
                } else {
                    String::new()
                },
            }),
        )
        .unwrap();
    store
        .record_verdict(
            id,
            &Verdict {
                category: mail.category.into(),
                confidence: Some(0.9),
                source: ClassifierSource::Rules,
                model: None,
                latency_ms: None,
            },
        )
        .unwrap();
    id
}

fn mailbox() -> (Store, AccountId, FolderId, FolderId) {
    let (store, account) = store_with_account();
    let inbox = store.upsert_folder(account, "INBOX", None).unwrap();
    let archive = store
        .upsert_folder(account, "Archive", Some("\\Archive"))
        .unwrap();
    let mails = [
        Mail {
            id: "1@x",
            from: "news@shop.example",
            subject: "Summer sale 20% off",
            days_ago: 40,
            seen: false,
            list: None,
            unsubscribe: Some("<https://shop.example/u/1>"),
            category: "marketing",
            folder: inbox,
        },
        Mail {
            id: "2@x",
            from: "news@shop.example",
            subject: "Winter sale 30% off",
            days_ago: 2,
            seen: true,
            list: None,
            unsubscribe: Some("<https://shop.example/u/2>"),
            category: "marketing",
            folder: inbox,
        },
        Mail {
            id: "3@x",
            from: "digest@list.example",
            subject: "Weekly digest",
            days_ago: 5,
            seen: false,
            list: Some("weekly.list.example"),
            unsubscribe: Some("<mailto:leave@list.example>"),
            category: "newsletter",
            folder: archive,
        },
        Mail {
            id: "4@x",
            from: "erika@example.de",
            subject: "Rechnung für Mai",
            days_ago: 1,
            seen: false,
            list: None,
            unsubscribe: None,
            category: "personal",
            folder: inbox,
        },
    ];
    for (uid, mail) in mails.iter().enumerate() {
        put(&store, account, uid as u32 + 1, mail);
    }
    (store, account, inbox, archive)
}

fn smart(match_all: bool, rules: &[(SmartField, SmartOp, &str)]) -> ListFilter {
    ListFilter {
        smart: Some(SmartQuery {
            match_all,
            rules: rules
                .iter()
                .map(|(field, op, value)| SmartRule {
                    field: *field,
                    op: *op,
                    value: value.to_string(),
                })
                .collect(),
        }),
        ..Default::default()
    }
}

fn subjects(store: &Store, account: AccountId, filter: &ListFilter) -> Vec<String> {
    let mut found: Vec<String> = store
        .message_window(account, 0, 50, filter)
        .unwrap()
        .messages
        .into_iter()
        .map(|m| m.summary.subject.unwrap())
        .collect();
    found.sort();
    found
}

#[test]
fn a_smart_mailbox_selects_by_every_rule_or_any() {
    let (store, account, _, _) = mailbox();

    let sales = smart(
        true,
        &[
            (SmartField::From, SmartOp::EndsWith, "@SHOP.example"),
            (SmartField::Subject, SmartOp::Contains, "sale"),
            (SmartField::Unread, SmartOp::Is, "yes"),
        ],
    );
    assert_eq!(subjects(&store, account, &sales), ["Summer sale 20% off"]);

    let either = smart(
        false,
        &[
            (SmartField::Category, SmartOp::Is, "newsletter"),
            (SmartField::Subject, SmartOp::BeginsWith, "rechnung"),
        ],
    );
    assert_eq!(
        subjects(&store, account, &either),
        ["Rechnung für Mai", "Weekly digest"]
    );
}

#[test]
fn smart_rules_cover_age_folders_body_and_unsubscribe() {
    let (store, account, _, _) = mailbox();

    let old = smart(true, &[(SmartField::OlderThanDays, SmartOp::Is, "30")]);
    assert_eq!(subjects(&store, account, &old), ["Summer sale 20% off"]);

    let archived = smart(true, &[(SmartField::Folder, SmartOp::Is, "archive")]);
    assert_eq!(subjects(&store, account, &archived), ["Weekly digest"]);

    let not_archived = smart(true, &[(SmartField::Folder, SmartOp::IsNot, "archive")]);
    assert_eq!(subjects(&store, account, &not_archived).len(), 3);

    let body = smart(true, &[(SmartField::Body, SmartOp::Contains, "winter")]);
    assert_eq!(subjects(&store, account, &body), ["Winter sale 30% off"]);

    let bulk = smart(true, &[(SmartField::CanUnsubscribe, SmartOp::Is, "no")]);
    assert_eq!(subjects(&store, account, &bulk), ["Rechnung für Mai"]);
}

#[test]
fn umlauts_match_whatever_their_case() {
    let (store, account, inbox, _) = mailbox();
    put(
        &store,
        account,
        40,
        &Mail {
            id: "ae@x",
            from: "shop@shop.example",
            subject: "Änderung Ihrer Bestellung",
            days_ago: 1,
            seen: true,
            list: None,
            unsubscribe: None,
            category: "transactional",
            folder: inbox,
        },
    );
    let lower = smart(
        true,
        &[(SmartField::Subject, SmartOp::Contains, "änderung")],
    );
    assert_eq!(
        subjects(&store, account, &lower),
        ["Änderung Ihrer Bestellung"]
    );
    let upper = smart(
        true,
        &[(SmartField::Subject, SmartOp::BeginsWith, "ÄNDERUNG")],
    );
    assert_eq!(
        subjects(&store, account, &upper),
        ["Änderung Ihrer Bestellung"]
    );
}

#[test]
fn a_mailbox_whose_rules_cannot_be_read_holds_nothing() {
    let (store, account, _, _) = mailbox();
    // A number of days that is not a number: the rule is refused on saving,
    // and one that got in anyway (imported, or from a newer build) selects
    // nothing rather than widening the mailbox to all mail.
    let broken = smart(true, &[(SmartField::OlderThanDays, SmartOp::Is, "soon")]);
    assert!(subjects(&store, account, &broken).is_empty());
    let half = smart(
        true,
        &[
            (SmartField::Category, SmartOp::Is, "marketing"),
            (SmartField::NewerThanDays, SmartOp::Is, "lately"),
        ],
    );
    assert!(subjects(&store, account, &half).is_empty());
    let none = smart(true, &[]);
    assert!(subjects(&store, account, &none).is_empty());
}

#[test]
fn a_value_that_looks_like_sql_is_only_ever_a_value() {
    let (store, account, _, _) = mailbox();
    let hostile = smart(
        true,
        &[(SmartField::Subject, SmartOp::Contains, "') OR 1=1 --")],
    );
    assert!(subjects(&store, account, &hostile).is_empty());
}

#[test]
fn smart_mailboxes_are_saved_counted_and_removed() {
    let (store, account, _, _) = mailbox();
    let query = smart(true, &[(SmartField::Category, SmartOp::Is, "marketing")])
        .smart
        .unwrap();
    let id = store
        .save_smart_mailbox(None, account, "Shops", &query, None)
        .unwrap();

    let saved = store.smart_mailboxes(account).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].query, query);
    assert!(store.smart_mailbox_named(account, "Shops").unwrap());

    let filter = ListFilter {
        smart: Some(saved[0].query.clone()),
        ..Default::default()
    };
    assert_eq!(store.count_matching(account, &filter).unwrap(), (2, 1));

    store.delete_smart_mailbox(id).unwrap();
    assert!(store.smart_mailboxes(account).unwrap().is_empty());
}

#[test]
fn senders_to_unsubscribe_from_are_grouped_newest_link_first() {
    let (store, account, _, _) = mailbox();
    let senders = store.unsubscribe_senders(account).unwrap();
    assert_eq!(senders.len(), 2);

    let shop = &senders[0];
    assert_eq!(shop.key, "news@shop.example");
    assert_eq!(shop.messages, 2);
    assert_eq!(shop.unread, 1);
    // The newest message's link: an older one may have expired.
    assert_eq!(shop.list_unsubscribe, "<https://shop.example/u/2>");

    // A list is keyed by its List-Id, not by whoever sent this issue.
    assert_eq!(senders[1].key, "weekly.list.example");
    assert_eq!(
        store
            .messages_from_sender(account, "news@shop.example")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn one_message_says_what_there_is_to_unsubscribe_from_and_how_much_of_it() {
    // What the offer made when a newsletter is deleted is built from: this
    // message's own link, and how much of that sender's mail is here — so the
    // offer can say what else goes with it.
    let (store, account, _, _) = mailbox();
    let senders = store.unsubscribe_senders(account).unwrap();
    let newest = senders[0].latest_message;

    let found = store
        .unsubscribe_for_message(account, newest)
        .unwrap()
        .expect("a newsletter offers something");
    assert_eq!(found.key, "news@shop.example");
    assert_eq!(found.list_unsubscribe, "<https://shop.example/u/2>");
    assert_eq!(found.messages, 2, "the sender's whole run, not this one");
    assert_eq!(found.unread, 1);

    // The message a person wrote offers nothing, which is what keeps the
    // question from being asked over an ordinary delete.
    let personal = store
        .unsubscribe_senders(account)
        .unwrap()
        .iter()
        .flat_map(|sender| store.messages_from_sender(account, &sender.key).unwrap())
        .count();
    assert_eq!(personal, 3, "the three that carry the header");
    let none = store.unsubscribe_for_message(account, 999_999).unwrap();
    assert!(none.is_none(), "a message that is not there offers nothing");
}

#[test]
fn the_cleanup_count_is_bulk_mail_still_in_the_inbox() {
    let (store, account, _, _) = mailbox();
    // Two marketing mails in the Inbox; the newsletter is archived already,
    // and the personal mail is not bulk.
    assert_eq!(store.cleanup_count(account).unwrap(), 2);
}

#[test]
fn the_outbox_sends_each_message_once() {
    let (store, account, _, _) = mailbox();
    let due = store
        .schedule_send(account, "{}", "Hello", "erika@example.de", now() - 5)
        .unwrap();
    let later = store
        .schedule_send(account, "{}", "Later", "erika@example.de", now() + 3600)
        .unwrap();

    let ready = store.due_outbox(now()).unwrap();
    assert_eq!(ready.iter().map(|e| e.id).collect::<Vec<_>>(), [due]);

    assert!(!store.claim_outbox(later, now()).unwrap(), "not due yet");
    assert!(store.claim_outbox(due, now()).unwrap());
    assert!(!store.claim_outbox(due, now()).unwrap(), "claimed twice");
    assert!(
        !store.cancel_outbox(due).unwrap(),
        "a send in flight is not cancelled"
    );
    store.outbox_sent(due).unwrap();

    assert!(store.cancel_outbox(later).unwrap());
    assert!(store.outbox().unwrap().is_empty());
}

#[test]
fn a_send_interrupted_by_closing_is_never_retried_on_a_guess() {
    let (store, account, _, _) = mailbox();
    let id = store
        .schedule_send(account, "{}", "Hello", "erika@example.de", now() - 5)
        .unwrap();
    assert!(store.claim_outbox(id, now()).unwrap());

    assert_eq!(store.recover_interrupted_sends().unwrap(), 1);
    let entry = store.outbox_entry(id).unwrap().unwrap();
    assert_eq!(entry.state, "failed");
    assert!(store.due_outbox(now()).unwrap().is_empty());

    // Rescheduling is how a person says "yes, send it again".
    assert!(store.reschedule_outbox(id, now() - 1).unwrap());
    assert_eq!(store.due_outbox(now()).unwrap().len(), 1);
}

#[test]
fn headers_are_read_back_once_for_mail_synced_before_they_were_kept() {
    let (store, account, _, _) = mailbox();
    // Everything stored through upsert_message has its headers already.
    assert!(store
        .messages_missing_headers(account, 10)
        .unwrap()
        .is_empty());

    store
        .connection()
        .execute(
            "UPDATE message SET headers_read = 0, list_unsubscribe = NULL",
            [],
        )
        .unwrap();
    let missing = store.messages_missing_headers(account, 10).unwrap();
    assert_eq!(missing.len(), 4);

    let updates: Vec<_> = missing
        .iter()
        .map(|(id, _)| {
            (
                *id,
                Some(core_store::BackfilledHeaders {
                    list_unsubscribe: Some("<https://x.example/u>".into()),
                    ..Default::default()
                }),
            )
        })
        .collect();
    store.set_backfilled_headers(&updates).unwrap();
    assert!(store
        .messages_missing_headers(account, 10)
        .unwrap()
        .is_empty());
    assert_eq!(store.unsubscribe_senders(account).unwrap().len(), 3);
}

// -- conversations and urgency ------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn sent_by_me(
    store: &Store,
    account: AccountId,
    sent: FolderId,
    uid: u32,
    to: &str,
    subject: &str,
    days_ago: i64,
    reply_to: Option<&str>,
) -> MessageId {
    let (id, _) = store
        .upsert_message(
            account,
            &NewMessage {
                rfc822_message_id: Some(format!("me-{uid}@x")),
                subject: Some(subject.into()),
                from_addr: Some("Dev@Kuverta.test".into()),
                recipients: Some(format!("{to}, cc@example.org")),
                date_utc: Some(now() - days_ago * 86_400),
                in_reply_to: reply_to.map(Into::into),
                snippet: Some("my words".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: sent,
                uid,
                flags: "\\Seen".into(),
            }),
        )
        .unwrap();
    id
}

#[test]
fn conversations_pair_what_came_with_what_went() {
    let (store, account, _, _) = mailbox();
    let sent = store
        .upsert_folder(account, "Sent", Some("\\Sent"))
        .unwrap();
    sent_by_me(
        &store,
        account,
        sent,
        1,
        "erika@example.de",
        "Re: Rechnung für Mai",
        0,
        Some("4@x"),
    );

    let page = store
        .conversations(account, "dev@kuverta.test", &[], false, None, 0, 50)
        .unwrap();
    // Erika (one from her, one to her); the shop and the list are bulk.
    assert_eq!(page.total, 1);
    let erika = &page.conversations[0];
    assert_eq!(erika.key, "erika@example.de");
    assert_eq!(erika.messages, 2);
    assert_eq!(erika.unread, 1);
    assert!(erika.last_from_me);

    let with_bulk = store
        .conversations(account, "dev@kuverta.test", &[], true, None, 0, 50)
        .unwrap();
    assert_eq!(with_bulk.total, 3);
    let found = store
        .conversations(account, "dev@kuverta.test", &[], true, Some("SHOP"), 0, 50)
        .unwrap();
    assert_eq!(found.conversations[0].key, "news@shop.example");

    let messages = store
        .conversation(account, "dev@kuverta.test", "erika@example.de", &[], 50)
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(!messages[0].from_me, "oldest first");
    assert!(messages[1].from_me);
}

#[test]
fn a_correspondent_is_summed_up_from_both_directions() {
    let (store, account, inbox, _) = mailbox();
    let sent = store
        .upsert_folder(account, "Sent", Some("\\Sent"))
        .unwrap();
    sent_by_me(
        &store,
        account,
        sent,
        1,
        "erika@example.de",
        "Re: Rechnung für Mai",
        0,
        Some("4@x"),
    );
    // A second letter from her, older and filed away, so the folder counts
    // have more than one answer.
    let archive = store.upsert_folder(account, "Archive", None).unwrap();
    put(
        &store,
        account,
        9,
        &Mail {
            id: "9@x",
            from: "erika@example.de",
            subject: "Angebot",
            days_ago: 30,
            seen: true,
            list: None,
            unsubscribe: None,
            category: "personal",
            folder: archive,
        },
    );

    let erika = store
        .correspondent(account, "dev@kuverta.test", "Erika@example.de", &[])
        .unwrap();
    assert_eq!(erika.address, "erika@example.de");
    assert_eq!((erika.received, erika.sent, erika.unread), (2, 1, 1));
    assert_eq!(erika.usual_category, Some(("personal".into(), 2)));
    assert_eq!(
        erika.folders,
        vec![
            ("Archive".to_string(), 1),
            ("INBOX".to_string(), 1),
            ("Sent".to_string(), 1)
        ]
    );
    // She wrote yesterday; the reply went today.
    assert!(erika.last_to_them_utc >= erika.last_from_them_utc);
    assert!(erika.first_utc.unwrap() < erika.last_from_them_utc.unwrap());
    assert!(erika.waiting.is_none());

    // The shop is bulk, and one-sided: it has never been written to.
    let shop = store
        .correspondent(account, "dev@kuverta.test", "news@shop.example", &[])
        .unwrap();
    assert_eq!((shop.received, shop.sent), (2, 0));
    assert_eq!(shop.usual_category, Some(("marketing".into(), 2)));

    // Somebody with no mail at all answers as an empty card, not an error.
    let stranger = store
        .correspondent(account, "dev@kuverta.test", "nobody@example.com", &[])
        .unwrap();
    assert_eq!((stranger.received, stranger.sent), (0, 0));
    assert!(stranger.name.is_none() && stranger.folders.is_empty());

    // A verdict on her Inbox letter surfaces on her card.
    let waiting = store
        .message_window(account, 0, 50, &ListFilter::default())
        .unwrap()
        .messages
        .into_iter()
        .find(|m| m.summary.subject.as_deref() == Some("Rechnung für Mai"))
        .unwrap()
        .summary
        .id;
    store
        .record_urgency(
            waiting,
            &Urgency {
                score: 3,
                reason: "an invoice is due".into(),
                action: Some("pay".into()),
                deadline: None,
                source: "rules".into(),
                model: None,
            },
        )
        .unwrap();
    let erika = store
        .correspondent(account, "dev@kuverta.test", "erika@example.de", &[])
        .unwrap();
    assert_eq!(erika.waiting.map(|u| u.score), Some(3));

    // The card asked for by message finds the same person from either side:
    // from her letter by its sender, from the reply by its recipient.
    let from_her = store
        .counterpart_of(account, "dev@kuverta.test", waiting)
        .unwrap();
    assert_eq!(from_her.as_deref(), Some("erika@example.de"));
    let _ = inbox;
}

#[test]
fn urgency_is_asked_once_and_ranks_the_view() {
    let (store, account, _, _) = mailbox();
    let sent = store
        .upsert_folder(account, "Sent", Some("\\Sent"))
        .unwrap();
    // Only Erika's mail is not bulk, and it is recent.
    let candidates = store
        .urgency_candidates(account, "dev@kuverta.test", now() - 30 * 86_400, false, 50)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    let erika = candidates[0].id;
    assert!(!store
        .has_replied(account, "dev@kuverta.test", "4@x")
        .unwrap());

    store
        .record_urgency(
            erika,
            &Urgency {
                score: 2,
                reason: "a person is waiting".into(),
                action: Some("reply".into()),
                deadline: None,
                source: "rules".into(),
                model: None,
            },
        )
        .unwrap();
    assert!(store
        .urgency_candidates(account, "dev@kuverta.test", 0, false, 50)
        .unwrap()
        .is_empty());
    // A model is still wanted where only the rules have spoken.
    assert_eq!(
        store
            .urgency_candidates(account, "dev@kuverta.test", 0, true, 50)
            .unwrap()
            .len(),
        1
    );

    let urgent = store.urgent_messages(account, 2, 50).unwrap();
    assert_eq!(urgent.len(), 1);
    assert_eq!(urgent[0].urgency.action.as_deref(), Some("reply"));

    sent_by_me(
        &store,
        account,
        sent,
        2,
        "erika@example.de",
        "Re: Rechnung für Mai",
        0,
        Some("4@x"),
    );
    assert!(store
        .has_replied(account, "dev@kuverta.test", "4@x")
        .unwrap());
    assert_eq!(
        store
            .times_written_to(account, "dev@kuverta.test", "erika@example.de")
            .unwrap(),
        1
    );
}

// -- profiles -------------------------------------------------------------------

#[test]
fn profiles_hold_accounts_and_let_go_of_them_when_deleted() {
    let (store, account) = store_with_account();
    let private = store.add_profile("Private").unwrap();
    let work = store.add_profile("Work").unwrap();
    assert!(
        store.add_profile("private").is_err(),
        "names are unique, whatever the case"
    );
    assert_eq!(store.profile_named("WORK").unwrap(), Some(work));

    store.set_account_profile(account, Some(work)).unwrap();
    assert_eq!(store.account_profiles().unwrap(), [(account, Some(work))]);

    store.reorder_profiles(&[work, private]).unwrap();
    let names: Vec<String> = store
        .profiles()
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Work", "Private"]);

    store.delete_profile(work).unwrap();
    assert_eq!(store.account_profiles().unwrap(), [(account, None)]);
    assert_eq!(store.profiles().unwrap().len(), 1);
}

// -- tasks ----------------------------------------------------------------------

#[test]
fn a_task_deals_with_each_message_once_and_proposals_settle_once() {
    let (store, account, _, _) = mailbox();
    let query = SmartQuery {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::Category,
            op: SmartOp::Is,
            value: "marketing".into(),
        }],
    };
    let id = store
        .save_task(
            None,
            account,
            "Shops",
            &query,
            r#"{"kind":"archive"}"#,
            false,
            true,
        )
        .unwrap();
    let task = store.task(id).unwrap().unwrap();
    assert_eq!(task.query, query);
    assert!(task.enabled && !task.review);

    store.mark_task_seen(id, &[1, 2]).unwrap();
    store.mark_task_seen(id, &[2]).unwrap();
    assert_eq!(store.task_seen(id).unwrap().len(), 2);

    let proposal = store
        .add_proposal(&core_store::NewProposal {
            task_id: Some(id),
            account_id: account,
            message_id: Some(1),
            kind: "archive".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(store.pending_proposals(account).unwrap().len(), 1);
    assert_eq!(store.pending_by_task(account).unwrap(), [(Some(id), 1)]);
    assert!(store.settle_proposal(proposal, "done", None).unwrap());
    assert!(
        !store.settle_proposal(proposal, "done", None).unwrap(),
        "only once"
    );
    assert!(store.pending_proposals(account).unwrap().is_empty());

    store.delete_task(id).unwrap();
    assert!(store.tasks(account).unwrap().is_empty());
}
