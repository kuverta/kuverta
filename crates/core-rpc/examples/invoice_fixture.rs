//! Offline native-UI fixture; refuses to touch any existing directory.
//! cargo run -p core-rpc --release --example invoice_fixture -- NEW_DIRECTORY
use base64::Engine;
use core_rpc::Core;
use core_store::model::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path =
        std::path::PathBuf::from(std::env::args().nth(1).ok_or("Pass a NEW test directory")?);
    if path.exists() {
        return Err("The fixture directory must not already exist".into());
    }
    std::fs::create_dir_all(path.join("output"))?;
    let core = Core::open(&path)?;
    let profile = core.save_profile(None, "Rechnungstest Firma A")?;
    let other = core.save_profile(None, "Rechnungstest Firma B")?;
    let a="TESTBELEG - KEINE ECHTE RECHNUNG\nRechnungsnummer: TEST-100\nRechnungsdatum: 2026-09-15\nBetrag: 10,00 EUR\n";
    let b="TESTBELEG - KEINE ECHTE GUTSCHRIFT\nGutschriftnummer: TEST-101\nBelegdatum: 2026-09-20\nBetrag: -2,00 EUR\n";
    for (n, p, docs) in [
        (1, profile, vec![a, b]),
        (2, profile, vec![a]),
        (3, other, vec![a]),
    ] {
        let email = format!("test{n}@example.invalid");
        let account = core.store().add_account(&NewAccount {
            label: format!("Test {n}"),
            email: email.clone(),
            username: email,
            imap_host: "127.0.0.1".into(),
            imap_port: 19999,
            auth_method: "app_password".into(),
            ..Default::default()
        })?;
        core.set_account_profile(account, Some(p))?;
        let folder = core.store().upsert_folder(account, "INBOX", None)?;
        for (index, body) in docs.iter().enumerate() {
            let raw=format!("From: Testvendor <vendor@example.invalid>\r\nSubject: Rechnungstest {n}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain\r\n\r\nNur ein synthetischer Testbeleg.\r\n--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<h2>Kuverta HTML-Test</h2><p>Diese Nachricht ist ein <strong>synthetischer Testbeleg</strong>.</p>\r\n--alt--\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"beleg.txt\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--b--\r\n",base64::engine::general_purpose::STANDARD.encode(body.as_bytes()));
            let stored = core
                .blobs()
                .put(account, &format!("test-{index}"), raw.as_bytes())?;
            core.store().upsert_message(
                account,
                &NewMessage {
                    rfc822_message_id: Some(format!("test-{account}-{index}@example.invalid")),
                    subject: Some(format!("Rechnungstest {n}")),
                    from_addr: Some("vendor@example.invalid".into()),
                    date_utc: Some(1789948800),
                    has_attachments: true,
                    body_path: Some(stored),
                    ..Default::default()
                },
                Some(&Location {
                    folder_id: folder,
                    uid: index as u32 + 1,
                    flags: String::new(),
                }),
            )?;
        }
    }
    let root = std::fs::canonicalize(path.join("output"))?;
    core.store()
        .set_preference(&format!("invoice.root.{profile}"), &root.to_string_lossy())?;
    println!(
        "Offline fixture ready: 2 profiles, 3 accounts, 4 test messages. Output: {}",
        root.display()
    );
    Ok(())
}
