//! Saving drafts, without a server.
//!
//! The append itself needs IMAP, which the dev stack provides and these tests do
//! not. What can be checked without it is everything that must happen before a
//! connection is ever made — and the refusal of Bcc is the one that matters.

use core_rpc::{DraftInput, RpcError, Session};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("fuckmail-drafts-{}-{name}", std::process::id()));
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

#[tokio::test]
async fn a_draft_with_bcc_is_refused_before_anything_is_built() {
    // Bcc is never written into the message, so a draft saved to the server
    // would lose its blind recipients — and whoever sent it later would send
    // it without them, from any client, with nothing to say so.
    let dir = TempDir::new("bcc");
    let err = Session::new(dir.0.clone())
        .save_draft(
            "nobody@example.com",
            &DraftInput {
                to: vec!["a@example.com".into()],
                bcc: vec!["hidden@example.com".into()],
                subject: "s".into(),
                body: "b".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

    match err {
        RpcError::Rejected(message) => assert!(message.contains("Bcc"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_blank_bcc_field_is_not_a_blind_recipient() {
    // A compose form sends an empty string for an empty field. That must not
    // be mistaken for a recipient and refused.
    let dir = TempDir::new("blank-bcc");
    let err = Session::new(dir.0.clone())
        .save_draft(
            "nobody@example.com",
            &DraftInput {
                to: vec!["a@example.com".into()],
                bcc: vec!["  ".into()],
                subject: "s".into(),
                body: "b".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RpcError::UnknownAccount(_)), "got {err:?}");
}

#[test]
fn blank_address_fields_are_empty_fields_not_malformed_addresses() {
    // Found against the dev server: a blank Cc was parsed as the address ""
    // and the whole draft refused, while a blank Bcc was already accepted.
    let dir = TempDir::new("blank-fields");
    let store = core_store::Store::open(dir.0.join("fuckmail.db")).unwrap();
    store
        .add_account(&core_store::model::NewAccount {
            label: "me@example.com".into(),
            email: "me@example.com".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 1,
            username: "me@example.com".into(),
            auth_method: "app_password".into(),
            ..Default::default()
        })
        .unwrap();

    let preview = Session::new(dir.0.clone())
        .preview(
            "me@example.com",
            &DraftInput {
                to: vec!["a@example.com".into(), "".into()],
                cc: vec!["  ".into()],
                bcc: vec!["".into()],
                subject: "s".into(),
                body: "b".into(),
                ..Default::default()
            },
        )
        .expect("blank fields should not refuse the draft");

    assert_eq!(preview.recipients, vec!["a@example.com".to_string()]);
    assert!(!preview.rfc822.contains("\r\nCc:"), "{}", preview.rfc822);
}

#[tokio::test]
async fn a_draft_for_an_account_that_does_not_exist_is_refused_before_connecting() {
    let dir = TempDir::new("no-account");
    let err = Session::new(dir.0.clone())
        .save_draft(
            "nobody@example.com",
            &DraftInput {
                to: vec!["a@example.com".into()],
                subject: "s".into(),
                body: "b".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RpcError::UnknownAccount(_)), "got {err:?}");
}
