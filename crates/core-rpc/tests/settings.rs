//! Account configuration, as the settings window drives it.
//!
//! The keychain is deliberately untouched here: on macOS a keychain read from
//! a test binary raises a GUI prompt, which would make `cargo test` wait on a
//! dialog nobody is watching. So these cover everything except storing a
//! password, which is one call into `core-accounts` and tested there.

use core_rpc::{AccountInput, Core, RpcError};
use core_store::model::*;
use core_store::{Blobs, Store};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("kuverta-settings-{}-{name}", std::process::id()));
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

fn core(name: &str) -> (Core, TempDir) {
    let dir = TempDir::new(name);
    let core = Core::new(
        Store::open(dir.0.join("kuverta.db")).unwrap(),
        Blobs::new(dir.0.join("blobs")),
    );
    (core, dir)
}

/// A filled-in form for a plausible provider.
fn form() -> AccountInput {
    AccountInput {
        id: None,
        label: "Work".into(),
        email: "me@example.de".into(),
        username: None,
        imap_host: "imap.example.de".into(),
        imap_port: 993,
        imap_security: "tls".into(),
        smtp_host: Some("smtp.example.de".into()),
        smtp_port: Some(587),
        smtp_security: Some("starttls".into()),
        auth_method: "app_password".into(),
        ..Default::default()
    }
}

#[test]
fn saving_a_form_creates_an_account_and_reading_it_back_refills_the_form() {
    let (core, _dir) = core("roundtrip");
    let id = core.save_account(&form()).unwrap();

    let settings = core.account_settings(id).unwrap();
    assert_eq!(settings.email, "me@example.de");
    assert_eq!(settings.label, "Work");
    // Blank means "the same as the email", which is right for nearly every
    // provider and one fewer field to fill in.
    assert_eq!(settings.username, "me@example.de");
    assert_eq!(settings.imap_port, 993);
    assert_eq!(settings.smtp_host.as_deref(), Some("smtp.example.de"));
    assert_eq!(settings.smtp_security.as_deref(), Some("starttls"));
    assert!(!settings.has_password);
}

#[test]
fn a_password_is_never_handed_back_out() {
    // The form shows whether one is stored, never what it is. A settings pane
    // that cannot display a password cannot leak one into a screenshot.
    let (core, _dir) = core("nopassword");
    let id = core.save_account(&form()).unwrap();
    let settings = core.account_settings(id).unwrap();

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("has_password"));
    assert!(
        !json.to_lowercase().contains("\"password\""),
        "the settings view must carry no password field: {json}"
    );
}

#[test]
fn editing_changes_only_what_the_form_sent() {
    let (core, _dir) = core("edit");
    let id = core.save_account(&form()).unwrap();

    let mut edit = form();
    edit.id = Some(id);
    edit.label = "Personal".into();
    edit.imap_port = 143;
    edit.imap_security = "starttls".into();
    let same = core.save_account(&edit).unwrap();
    assert_eq!(same, id, "editing must not create a second account");

    let settings = core.account_settings(id).unwrap();
    assert_eq!(settings.label, "Personal");
    assert_eq!(settings.imap_port, 143);
    assert_eq!(settings.email, "me@example.de");
    assert_eq!(core.accounts().unwrap().len(), 1);
}

#[test]
fn clearing_the_smtp_host_turns_sending_off() {
    // The endpoint has to be removable, and a form that simply omitted it
    // would leave the old one in place and the account still able to send.
    let (core, _dir) = core("nosmtp");
    let id = core.save_account(&form()).unwrap();
    assert!(core.accounts().unwrap()[0].can_send);

    let mut edit = form();
    edit.id = Some(id);
    edit.smtp_host = None;
    core.save_account(&edit).unwrap();

    assert!(!core.accounts().unwrap()[0].can_send);
    assert!(core.account_settings(id).unwrap().smtp_host.is_none());
}

#[test]
fn plaintext_is_refused_for_anywhere_but_localhost() {
    // The rule belongs to the setting, not to the front end collecting it:
    // a password sent in the clear is a password disclosed.
    let (core, _dir) = core("plaintext");

    let mut remote = form();
    remote.imap_security = "plaintext".into();
    match core.save_account(&remote) {
        Err(RpcError::Rejected(why)) => assert!(why.contains("localhost"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    let mut remote_smtp = form();
    remote_smtp.smtp_security = Some("plaintext".into());
    assert!(matches!(
        core.save_account(&remote_smtp),
        Err(RpcError::Rejected(_))
    ));

    // Against the dev server it is allowed, because that is what it is for.
    let mut local = form();
    local.imap_host = "127.0.0.1".into();
    local.imap_security = "plaintext".into();
    local.smtp_host = Some("127.0.0.1".into());
    local.smtp_security = Some("plaintext".into());
    assert!(core.save_account(&local).is_ok());
}

#[test]
fn a_half_filled_form_is_refused_with_a_sentence() {
    let (core, _dir) = core("incomplete");

    let missing_email = AccountInput {
        email: String::new(),
        ..form()
    };
    assert!(matches!(
        core.save_account(&missing_email),
        Err(RpcError::Rejected(_))
    ));

    // A host with no port is not half a configuration; it is a mistake.
    let no_port = AccountInput {
        smtp_port: None,
        ..form()
    };
    match core.save_account(&no_port) {
        Err(RpcError::Rejected(why)) => assert!(why.contains("port"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    // OAuth2 without a client id can never log in.
    let no_client_id = AccountInput {
        auth_method: "oauth2".into(),
        ..form()
    };
    match core.save_account(&no_client_id) {
        Err(RpcError::Rejected(why)) => assert!(why.contains("client id"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_same_address_cannot_be_registered_twice() {
    let (core, _dir) = core("duplicate");
    core.save_account(&form()).unwrap();
    match core.save_account(&form()) {
        Err(RpcError::Rejected(why)) => assert!(why.contains("already registered"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn exclusions_are_replaced_by_the_form_rather_than_added_to() {
    // The textarea is the whole set. Removing a line has to remove the
    // exclusion, or folders could only ever be excluded and never restored.
    let (core, _dir) = core("exclusions");
    let mut with = form();
    with.excluded_folders = vec!["\\All".into(), "Junk".into()];
    let id = core.save_account(&with).unwrap();
    // Order is the store's, not the form's — the set is what matters.
    let mut stored = core.account_settings(id).unwrap().excluded_folders;
    stored.sort();
    assert_eq!(stored, vec!["Junk", "\\All"]);

    let mut fewer = with.clone();
    fewer.id = Some(id);
    fewer.excluded_folders = vec!["Junk".into()];
    core.save_account(&fewer).unwrap();
    assert_eq!(
        core.account_settings(id).unwrap().excluded_folders,
        vec!["Junk"]
    );

    let mut none = fewer.clone();
    none.excluded_folders = vec![];
    core.save_account(&none).unwrap();
    assert!(core
        .account_settings(id)
        .unwrap()
        .excluded_folders
        .is_empty());
}

#[test]
fn removing_an_account_takes_its_mail_with_it() {
    let (core, _dir) = core("delete");
    let id = core.save_account(&form()).unwrap();
    let inbox = core.store().upsert_folder(id, "INBOX", None).unwrap();
    core.store()
        .upsert_message(
            id,
            &NewMessage {
                rfc822_message_id: Some("gone@example.com".into()),
                ..Default::default()
            },
            Some(&Location {
                folder_id: inbox,
                uid: 1,
                flags: String::new(),
            }),
        )
        .unwrap();
    assert_eq!(core.store().message_count(id).unwrap(), 1);

    core.delete_account(id).unwrap();
    assert!(core.accounts().unwrap().is_empty());
    assert_eq!(core.store().message_count(id).unwrap(), 0);
    assert!(core.account_settings(id).is_err());
}

#[test]
fn a_data_directory_that_does_not_exist_yet_is_created() {
    // The first run has no data directory by definition, and failing before
    // the window opens leaves nowhere to say so.
    let parent = TempDir::new("firstrun");
    let nested = parent.0.join("does/not/exist");
    assert!(!nested.exists());

    let core = Core::open(&nested).expect("a first run should not need a directory prepared");
    assert!(core.accounts().unwrap().is_empty());
    assert!(nested.join("kuverta.db").exists());
}
