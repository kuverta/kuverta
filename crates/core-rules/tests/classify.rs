//! Classifier behaviour on messages shaped like the seeded fixtures and the
//! conflict cases the scoring model exists to handle.

use core_rules::{Category, Classifier, Learned, MessageFacts};

fn classify(facts: &MessageFacts<'_>) -> (Category, f64) {
    let c = Classifier::without_history().classify(facts);
    (c.category, c.confidence)
}

#[test]
fn a_personal_reply_is_personal() {
    let facts = MessageFacts {
        from_addr: Some("anna.weber@example.de"),
        from_name: Some("Anna Weber"),
        subject: Some("Re: Termin nächste Woche"),
        in_reply_to: Some("<prev@example.de>"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Personal);
}

#[test]
fn a_mailing_list_is_a_newsletter() {
    let facts = MessageFacts {
        from_addr: Some("hello@news.rustweekly.example"),
        from_name: Some("Rust Weekly"),
        subject: Some("Rust Weekly #612 — async traits everywhere"),
        list_id: Some("news.rustweekly.example"),
        precedence: Some("bulk"),
        list_unsubscribe: Some("<https://news.rustweekly.example/u/8837221>"),
        recipient_count: 1,
        ..Default::default()
    };
    // A newsletter from a two-word "name" to a single recipient must not be
    // mistaken for personal mail — the bulk headers suppress that.
    assert_eq!(classify(&facts).0, Category::Newsletter);
}

#[test]
fn a_promotional_blast_is_marketing() {
    let facts = MessageFacts {
        from_addr: Some("deals@shop.example.com"),
        from_name: Some("Shop Deals"),
        subject: Some("20% off everything this week only"),
        list_id: Some("deals.shop.example.com"),
        list_unsubscribe: Some("<https://shop.example.com/u>"),
        ..Default::default()
    };
    // It arrives as a list, but the marketing subject outweighs that.
    assert_eq!(classify(&facts).0, Category::Marketing);
}

#[test]
fn a_german_invoice_is_transactional() {
    let facts = MessageFacts {
        from_addr: Some("rechnung@hosting.example.de"),
        from_name: Some("Hosting AG"),
        subject: Some("Ihre Rechnung 4471182 für September 2026"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Transactional);
}

#[test]
fn an_invoice_with_a_pdf_is_transactional() {
    let facts = MessageFacts {
        from_addr: Some("buchhaltung@lieferant.example.de"),
        from_name: Some("Buchhaltung"),
        subject: Some("Rechnung RE-2026-0912"),
        has_attachments: true,
        recipient_count: 1,
        ..Default::default()
    };
    let result = Classifier::without_history().classify(&facts);
    assert_eq!(result.category, Category::Transactional);
    assert!(
        result
            .reasons
            .iter()
            .any(|r| r.rule == "attachment.with_transactional_subject"),
        "the attachment should be part of the reasoning"
    );
}

#[test]
fn a_cron_notice_from_a_noreply_is_a_notification() {
    let facts = MessageFacts {
        from_addr: Some("noreply@legacy-system.example.org"),
        from_name: Some("Legacy Cron"),
        subject: Some("nightly backup completed"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Notification);
}

#[test]
fn nothing_to_go_on_yields_unknown_not_a_confident_guess() {
    let facts = MessageFacts {
        from_addr: Some("x@example.com"),
        subject: Some("hi"),
        recipient_count: 3,
        ..Default::default()
    };
    let result = Classifier::without_history().classify(&facts);
    assert_eq!(result.category, Category::Unknown);
    assert_eq!(result.confidence, 0.0);
}

#[test]
fn confidence_is_lower_when_the_evidence_is_split() {
    // An invoice sent through a mailing list: transactional subject vs. list
    // headers. It should still decide, but not claim certainty.
    let conflicted = MessageFacts {
        from_addr: Some("billing@service.example.com"),
        subject: Some("Ihre Rechnung für März"),
        list_id: Some("service.example.com"),
        list_unsubscribe: Some("<https://service.example.com/u>"),
        ..Default::default()
    };
    let clean = MessageFacts {
        from_addr: Some("rechnung@hosting.example.de"),
        subject: Some("Ihre Rechnung 4471182"),
        recipient_count: 1,
        ..Default::default()
    };

    let split = Classifier::without_history()
        .classify(&conflicted)
        .confidence;
    let unanimous = Classifier::without_history().classify(&clean).confidence;
    assert!(
        split < unanimous,
        "split evidence ({split}) should be less confident than unanimous ({unanimous})"
    );
}

#[test]
fn a_learned_sender_overrides_the_heuristics() {
    // The user has decided this shop's mail is transactional (order updates),
    // not marketing. Their decision wins outright.
    let learned = Learned::new().with_sender("deals@shop.example.com", Category::Transactional);
    let facts = MessageFacts {
        from_addr: Some("deals@shop.example.com"),
        subject: Some("20% off everything this week only"),
        list_id: Some("deals.shop.example.com"),
        ..Default::default()
    };

    let result = Classifier::new(learned).classify(&facts);
    assert_eq!(result.category, Category::Transactional);
    assert_eq!(result.confidence, 1.0);
    assert_eq!(result.reasons[0].rule, "learned.sender");
}

#[test]
fn a_learned_list_is_matched_case_insensitively() {
    let learned = Learned::new().with_list("News.RustWeekly.Example", Category::Marketing);
    let facts = MessageFacts {
        from_addr: Some("hello@news.rustweekly.example"),
        list_id: Some("news.rustweekly.example"),
        ..Default::default()
    };
    assert_eq!(
        Classifier::new(learned).classify(&facts).category,
        Category::Marketing
    );
}

#[test]
fn every_verdict_can_explain_itself() {
    // The explanation is the point of the rules layer; a verdict with no
    // reasons would be as opaque as the model it is meant to keep honest.
    let facts = MessageFacts {
        from_addr: Some("rechnung@hosting.example.de"),
        subject: Some("Ihre Rechnung 4471182"),
        ..Default::default()
    };
    let result = Classifier::without_history().classify(&facts);
    assert!(!result.reasons.is_empty());
    assert!(result.reasons.iter().all(|r| r.category == result.category));
}

// -- people or organisations (rules version 2) ------------------------------

#[test]
fn an_organisation_named_after_its_domain_is_not_a_person() {
    let facts = MessageFacts {
        from_addr: Some("k.meier@sparkasse-musterstadt.example"),
        from_name: Some("Sparkasse Musterstadt"),
        subject: Some("Neue Nachricht im Postfach"),
        recipient_count: 1,
        ..Default::default()
    };
    // The bank's name is its domain's: the bank speaking, not Frau Meier.
    assert_ne!(classify(&facts).0, Category::Personal);
}

#[test]
fn a_role_address_is_not_a_person() {
    let facts = MessageFacts {
        from_addr: Some("kundenbetreuung@telefon.example"),
        from_name: Some("Erika Mustermann"),
        subject: Some("Ihr Anliegen"),
        recipient_count: 1,
        ..Default::default()
    };
    // Signed with a name, sent from a department.
    assert_ne!(classify(&facts).0, Category::Personal);
}

#[test]
fn an_address_that_is_the_company_is_not_a_person() {
    let facts = MessageFacts {
        from_addr: Some("uber@uber.example"),
        from_name: Some("Uber Receipts"),
        subject: Some("Your Tuesday trip"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_ne!(classify(&facts).0, Category::Personal);
}

#[test]
fn a_bulk_sending_subdomain_is_marketing() {
    let facts = MessageFacts {
        from_addr: Some("email@news.broker.example"),
        from_name: Some("Broker Team"),
        subject: Some("Was diese Woche wichtig war"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Marketing);
}

#[test]
fn a_person_relayed_by_a_platform_is_personal() {
    let facts = MessageFacts {
        from_addr: Some("x7-4f2a@mail.kleinanzeigen.example"),
        from_name: Some("Anna über Kleinanzeigen"),
        subject: Some("Ist das Fahrrad noch zu haben?"),
        recipient_count: 1,
        ..Default::default()
    };
    // Named after the platform's domain, but relayed: a person wrote it.
    assert_eq!(classify(&facts).0, Category::Personal);
}

#[test]
fn a_person_at_their_own_domain_is_personal() {
    let facts = MessageFacts {
        from_addr: Some("anna@weber.example"),
        from_name: Some("Anna Weber"),
        subject: Some("Fotos vom Wochenende"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Personal);
}

#[test]
fn an_unattended_address_inside_a_longer_one_is_a_notification() {
    let facts = MessageFacts {
        from_addr: Some("nicht.antworten@kundenservice.telefon.example"),
        from_name: Some("Telefon Kundenservice"),
        subject: Some("Ihr Tarif"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Notification);
}

#[test]
fn a_bill_sent_through_a_mailing_system_is_still_the_bill() {
    let facts = MessageFacts {
        from_addr: Some("rechnung@strom.example"),
        from_name: Some("Strom GmbH"),
        subject: Some("Ihre Rechnung für August"),
        list_id: Some("kunden.strom.example"),
        list_unsubscribe: Some("<https://strom.example/u>"),
        recipient_count: 1,
        ..Default::default()
    };
    assert_eq!(classify(&facts).0, Category::Transactional);
}
