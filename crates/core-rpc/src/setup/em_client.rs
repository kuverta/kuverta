//! IMAP accounts from eM Client 11's `accounts.dat`.
//!
//! The JSON records separate identities, IMAP and SMTP configurations. Protocol
//! records refer to their identity by BindingConfigurationUID; their own saved
//! credentials can be stale when UseBindingAccount is selected. Read a SQLite
//! transaction (including the live WAL), never deserialize eM Client's .NET types,
//! and never return secrets to the window or write a temporary password export.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::AccountInput;

const MAIL: &str = "MailClient.Accounts.Mail.MailAccountConfiguration";
const IMAP: &str = "MailClient.Protocols.Imap.ImapAccountConfiguration";
const SMTP: &str = "MailClient.Protocols.Smtp.SmtpAccountConfiguration";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read eM Client's accounts: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "this eM Client account format is not supported yet (expected eM Client 11 AccountsJson)"
    )]
    Unsupported,
}

// Source credentials are deliberately never copied into this object.
pub struct EmAccount {
    pub input: AccountInput,
    pub full_name: Option<String>,
    pub notes: Vec<String>,
}

impl EmAccount {
    pub fn into_found(self, existing: &[String]) -> super::FoundAccount {
        let mut entry = super::found("eM Client", self.input, existing);
        entry.full_name = self.full_name;
        entry.password_known = false;
        entry.notes.extend(self.notes);
        entry
    }
}

pub struct Scan {
    pub accounts: Vec<EmAccount>,
    /// Mail identities that could not be represented safely, not silently lost.
    pub skipped: usize,
}

pub fn database() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("eM Client/accounts.dat"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/eM Client/accounts.dat"))
    } else {
        None
    }
}

pub fn scan() -> Result<Option<Scan>, Error> {
    database().map_or(Ok(None), |path| read(&path))
}

pub fn read(path: &Path) -> Result<Option<Scan>, Error> {
    // Opening read-only must not create a new database when eM Client is absent.
    if !path.exists() {
        return Ok(None);
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(2))?;
    let transaction = db.transaction()?;
    let result = accounts(&transaction)?;
    transaction.commit()?;
    Ok(Some(result))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn enabled(value: &Value) -> bool {
    value.get("Enabled").and_then(Value::as_bool) != Some(false)
        && value.get("DisabledByRestriction").and_then(Value::as_bool) != Some(true)
}

fn credentials<'a>(protocol: &'a Value, identity: &'a Value) -> Option<&'a Value> {
    match text(protocol, "AccountCredentialsModel") {
        "UseBindingAccount" => identity.get("Credentials"),
        // Unknown credential modes must not fall back to a possibly stale secret.
        _ => None,
    }
}

fn endpoint(value: &Value) -> Option<(String, u16, String, bool)> {
    let host = text(value, "Host").trim();
    let port = u16::try_from(value.get("Port")?.as_u64()?).ok()?;
    if host.is_empty() || host.contains(['/', '\\', '\r', '\n', ' ']) || port == 0 {
        return None;
    }
    let (security, upgraded) = match text(value, "SSL") {
        "UseTlsOnSpecialPort" => ("tls", false),
        "UseTlsAlways" => ("starttls", false),
        "UseTlsWhenAvailable" | "NoSsl" => ("starttls", true),
        _ => return None,
    };
    Some((host.into(), port, security.into(), upgraded))
}

fn address(value: &str) -> Option<(String, Option<String>)> {
    if value.contains(['\r', '\n']) {
        return None;
    }
    let raw = format!("From: {value}\r\n\r\n");
    let message = mail_parser::MessageParser::default().parse(raw.as_bytes())?;
    let from = message.from()?.first()?;
    let email = from.address()?.trim();
    if !email.contains('@') {
        return None;
    }
    Some((email.into(), from.name().map(str::to_string)))
}

/// Kept separate from discovery so migrations and malformed/unsupported records
/// can be tested with synthetic databases, without reading real credentials.
pub fn accounts(db: &Connection) -> Result<Scan, Error> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='AccountsJson')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Unsupported);
    }
    let mut statement = db.prepare("SELECT key, value FROM AccountsJson ORDER BY key")?;
    let mut rows = statement.query([])?;
    let mut records = Vec::new();
    let mut skipped = 0;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let bytes = match row.get_ref(1)? {
            rusqlite::types::ValueRef::Text(bytes) | rusqlite::types::ValueRef::Blob(bytes) => {
                bytes
            }
            _ => {
                skipped += 1;
                continue;
            }
        };
        // Avoid retaining a huge damaged record. Error messages never contain JSON.
        let parsed = (bytes.len() <= 2 * 1024 * 1024)
            .then(|| serde_json::from_slice::<Value>(bytes).ok())
            .flatten();
        match parsed {
            Some(v) if v["TypeValue"].is_object() => {
                let kind = text(&v, "Type")
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                records.push((id, kind, v["TypeValue"].clone()));
            }
            _ => skipped += 1,
        }
    }
    // Older versions use Accounts instead; do not report a legacy profile as empty.
    if records.is_empty()
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='Accounts')",
            [],
            |r| r.get::<_, bool>(0),
        )?
        && db.query_row("SELECT EXISTS(SELECT 1 FROM Accounts)", [], |r| {
            r.get::<_, bool>(0)
        })?
    {
        return Err(Error::Unsupported);
    }
    let mut result = Vec::new();
    for (id, _, identity) in records.iter().filter(|(_, kind, _)| kind == MAIL) {
        let protocols = |kind: &'static str| {
            records.iter().filter(move |(_, k, v)| {
                k == kind && text(v, "BindingConfigurationUID") == id && enabled(v)
            })
        };
        let imaps: Vec<_> = protocols(IMAP).collect();
        if !enabled(identity) || imaps.len() != 1 {
            skipped += 1;
            continue;
        }
        let imap = &imaps[0].2;
        let Some((email, full_name)) = address(text(identity, "EmailAddress")) else {
            skipped += 1;
            continue;
        };
        let Some((imap_host, imap_port, imap_security, upgraded)) = endpoint(imap) else {
            skipped += 1;
            continue;
        };
        let creds = credentials(imap, identity);
        let username = creds
            .map(|c| text(c, "Username"))
            .filter(|s| !s.is_empty())
            .unwrap_or(&email)
            .to_string();
        // Credentials.Secret is encrypted even when Type is Password and
        // SaveSecret is true. Never mistake its Base64 text for a password.
        let mut notes = vec!["eM Client encrypts saved passwords. Enter the password or app password once; OAuth sign-ins cannot be transferred.".into()];
        if upgraded {
            notes.push(
                "eM Client allows an unencrypted connection here; kuverta requires STARTTLS."
                    .into(),
            );
        }
        let mut input = AccountInput {
            label: text(identity, "AccountName").to_string(),
            email,
            username: Some(username),
            imap_host,
            imap_port,
            imap_security,
            auth_method: "app_password".into(),
            ..AccountInput::default()
        };
        let smtps: Vec<_> = protocols(SMTP).collect();
        if smtps.len() == 1 {
            let smtp = &smtps[0].2;
            // kuverta uses the same credentials for SMTP and IMAP. Never attach a
            // server that would receive the wrong identity's password.
            let same_credentials = credentials(smtp, identity)
                .zip(creds)
                .is_some_and(|(a, b)| a == b);
            if let Some((host, port, security, upgraded)) =
                endpoint(smtp).filter(|_| same_credentials)
            {
                input.smtp_host = Some(host);
                input.smtp_port = Some(port);
                input.smtp_security = Some(security);
                if upgraded {
                    notes.push("kuverta requires STARTTLS for the outgoing server too.".into());
                }
            }
        }
        if input.smtp_host.is_none() {
            notes.push("The outgoing server could not be transferred with the same sign-in. Check SMTP settings before sending.".into());
        }
        if result
            .iter()
            .any(|a: &EmAccount| a.input.email.eq_ignore_ascii_case(&input.email))
        {
            skipped += 1;
            continue;
        }
        result.push(EmAccount {
            input,
            full_name,
            notes,
        });
    }
    Ok(Scan {
        accounts: result,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    fn database_with_accounts() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE AccountsJson (key TEXT PRIMARY KEY, value);")
            .unwrap();
        for (id, kind, value) in [
            (
                "identity-a",
                MAIL,
                json!({"EmailAddress":"\"Erika Example\" <erika@example.org>", "AccountName":"Work", "Enabled":true,
                "Credentials":{"Type":"Password", "Username":"erika-login", "Secret":"fixture-only-secret", "SaveSecret":true}}),
            ),
            (
                "identity-b",
                MAIL,
                json!({"EmailAddress":"other@example.org", "Credentials":{"Type":"Password", "Username":"other@example.org", "Secret":"other-fixture", "SaveSecret":true}}),
            ),
            (
                "imap-a",
                IMAP,
                json!({"BindingConfigurationUID":"identity-a", "Host":"imap.example.org", "Port":143, "SSL":"UseTlsAlways", "AccountCredentialsModel":"UseBindingAccount",
                "Credentials":{"Type":"Password", "Secret":"stale-fixture", "SaveSecret":false}}),
            ),
            (
                "imap-b",
                IMAP,
                json!({"BindingConfigurationUID":"identity-b", "Host":"other.example.org", "Port":993, "SSL":"UseTlsOnSpecialPort", "AccountCredentialsModel":"UseBindingAccount"}),
            ),
            (
                "smtp-a",
                SMTP,
                json!({"BindingConfigurationUID":"identity-a", "Host":"smtp.example.org", "Port":25, "SSL":"UseTlsAlways", "AccountCredentialsModel":"UseBindingAccount"}),
            ),
        ] {
            db.execute(
                "INSERT INTO AccountsJson VALUES (?1, ?2)",
                params![
                    id,
                    json!({"Type":format!("{kind}, Assembly"), "TypeValue":value}).to_string()
                ],
            )
            .unwrap();
        }
        db
    }

    fn change(db: &Connection, id: &str, f: impl FnOnce(&mut Value)) {
        let raw: String = db
            .query_row("SELECT value FROM AccountsJson WHERE key=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        let mut v: Value = serde_json::from_str(&raw).unwrap();
        f(&mut v["TypeValue"]);
        db.execute(
            "UPDATE AccountsJson SET value=?2 WHERE key=?1",
            params![id, v.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn binds_protocols_to_identity_without_reusing_encrypted_credentials() {
        let scan = accounts(&database_with_accounts()).unwrap();
        assert_eq!(scan.accounts.len(), 2);
        assert_eq!(scan.skipped, 0);
        let a = &scan.accounts[0];
        assert_eq!(a.input.email, "erika@example.org");
        assert_eq!(a.full_name.as_deref(), Some("Erika Example"));
        assert_eq!(a.input.username.as_deref(), Some("erika-login"));
        assert_eq!(a.input.imap_security, "starttls");
        assert_eq!(a.input.imap_port, 143);
        assert_eq!(a.input.smtp_port, Some(25));
        assert_eq!(scan.accounts[1].input.imap_security, "tls");
    }

    #[test]
    fn window_gets_only_password_availability_and_existing_account_status() {
        let a = accounts(&database_with_accounts())
            .unwrap()
            .accounts
            .remove(0);
        let found = a.into_found(&["ERIKA@EXAMPLE.ORG".into()]);
        assert!(!found.password_known && found.already_added && found.complete);
        assert!(found.notes.iter().any(|n| n.contains("encrypts")));
        let serialized = serde_json::to_string(&found).unwrap();
        assert!(!serialized.contains("fixture-only-secret"));
        assert!(!serialized.contains("Secret"));
        assert_eq!(found.source, "eM Client");
    }

    #[test]
    fn selecting_em_client_never_falls_back_to_another_clients_password() {
        let a = accounts(&database_with_accounts())
            .unwrap()
            .accounts
            .remove(0);
        assert!(super::super::stored_password_from(Some("eM Client"), &a.input, "").is_none());
    }

    #[test]
    fn oauth_and_unsaved_secrets_are_not_imported_as_passwords() {
        let db = database_with_accounts();
        change(&db, "identity-a", |v| {
            v["Credentials"]["Type"] = json!("OAuth")
        });
        change(&db, "identity-b", |v| {
            v["Credentials"]["SaveSecret"] = json!(false)
        });
        let scan = accounts(&db).unwrap();
        assert_eq!(scan.accounts.len(), 2);
        assert!(scan.accounts.iter().all(|a| !a.notes.is_empty()));
    }

    #[test]
    fn different_smtp_sign_in_is_not_silently_replaced_with_imap_credentials() {
        let db = database_with_accounts();
        change(&db, "smtp-a", |v| {
            v["AccountCredentialsModel"] = json!("SeparateCredentials")
        });
        let scan = accounts(&db).unwrap();
        assert!(scan.accounts[0].input.smtp_host.is_none());
        assert!(scan.accounts[0].notes.iter().any(|n| n.contains("SMTP")));
    }

    #[test]
    fn opportunistic_tls_is_upgraded_and_unknown_transport_is_reported() {
        let db = database_with_accounts();
        change(&db, "imap-a", |v| v["SSL"] = json!("UseTlsWhenAvailable"));
        change(&db, "imap-b", |v| v["SSL"] = json!("FutureUnknownMode"));
        let scan = accounts(&db).unwrap();
        assert_eq!(scan.accounts.len(), 1);
        assert_eq!(scan.skipped, 1);
        assert_eq!(scan.accounts[0].input.imap_security, "starttls");
        assert!(scan.accounts[0]
            .notes
            .iter()
            .any(|n| n.contains("STARTTLS")));
    }

    #[test]
    fn unsupported_mail_protocols_and_disabled_accounts_are_reported() {
        let db = database_with_accounts();
        change(&db, "identity-b", |v| v["Enabled"] = json!(false));
        db.execute("DELETE FROM AccountsJson WHERE key='imap-a'", [])
            .unwrap();
        let scan = accounts(&db).unwrap();
        assert!(scan.accounts.is_empty());
        assert_eq!(scan.skipped, 2);
    }

    #[test]
    fn damaged_record_does_not_hide_other_accounts_or_echo_secrets() {
        let db = database_with_accounts();
        db.execute(
            "INSERT INTO AccountsJson VALUES ('broken', '{\"Secret\":\"dont-echo-me\"')",
            [],
        )
        .unwrap();
        let scan = accounts(&db).unwrap();
        assert_eq!(scan.accounts.len(), 2);
        assert_eq!(scan.skipped, 1);
    }

    #[test]
    fn legacy_format_is_reported_instead_of_appearing_empty() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE Accounts (key TEXT, value);")
            .unwrap();
        assert!(matches!(accounts(&db), Err(Error::Unsupported)));
        db.execute_batch("CREATE TABLE AccountsJson (key TEXT, value); INSERT INTO Accounts VALUES ('old', '<account/>');").unwrap();
        assert!(matches!(accounts(&db), Err(Error::Unsupported)));
    }

    #[test]
    fn reads_committed_wal_data_while_em_client_is_open() {
        let dir = std::env::temp_dir().join(format!("kuverta-em-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.dat");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE AccountsJson (key TEXT, value);")
            .unwrap();
        let fixture = database_with_accounts();
        let mut statement = fixture
            .prepare("SELECT key,value FROM AccountsJson")
            .unwrap();
        for row in statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
        {
            let (id, value) = row.unwrap();
            db.execute(
                "INSERT INTO AccountsJson VALUES (?1, ?2)",
                params![id, value],
            )
            .unwrap();
        }
        assert_eq!(read(&path).unwrap().unwrap().accounts.len(), 2);
        assert_eq!(
            db.query_row("SELECT count(*) FROM AccountsJson", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
