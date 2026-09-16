//! Accounts from Apple Mail.
//!
//! Since macOS 10.13 Mail keeps no account list of its own: its accounts are
//! the system's Internet Accounts, in `~/Library/Accounts/Accounts4.sqlite`.
//! macOS guards that directory, so reading it takes Full Disk Access for
//! whichever program asks — kuverta, or the terminal it was started from.
//!
//! The database is Core Data's: `ZACCOUNT` rows with a type from
//! `ZACCOUNTTYPE`, and settings in `ZACCOUNTPROPERTY` as binary property
//! lists. Only the address is relied on. A server that is stored is used; a
//! provider's account (iCloud, Google, Yahoo) stores none, and gets its
//! servers the way a typed-in address does.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// What reading Apple Mail's accounts came to.
#[derive(Debug)]
pub enum Outcome {
    /// Not a Mac, or never set up.
    Absent,
    /// macOS would not let this program read the accounts.
    NotPermitted,
    Found(Vec<AppleAccount>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppleAccount {
    pub email: String,
    pub description: Option<String>,
    /// Stored for an IMAP account typed in by hand; none for a provider's.
    pub imap: Option<(String, Option<u16>, Option<bool>)>,
    pub username: Option<String>,
    /// The account type's identifier, e.g. `com.apple.account.IMAP`.
    pub kind: String,
}

pub fn database() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Accounts/Accounts4.sqlite"))
}

/// Where System Settings shows Full Disk Access.
pub const PERMISSION_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

pub fn scan() -> Outcome {
    if !cfg!(target_os = "macos") {
        return Outcome::Absent;
    }
    let Some(path) = database() else {
        return Outcome::Absent;
    };
    match read(&path) {
        Ok(Some(accounts)) => Outcome::Found(accounts),
        Ok(None) => Outcome::Absent,
        Err(Problem::NotPermitted) => Outcome::NotPermitted,
        Err(Problem::Other(err)) => {
            tracing::warn!(%err, "could not read Apple Mail's accounts");
            Outcome::Found(Vec::new())
        }
    }
}

enum Problem {
    NotPermitted,
    Other(String),
}

/// The mail accounts in an accounts database. `Ok(None)` when there is no
/// database.
///
/// Read from a copy: the live file is in use, in WAL mode, and a reader that
/// is not allowed to write beside it cannot open it reliably.
fn read(path: &Path) -> Result<Option<Vec<AppleAccount>>, Problem> {
    let copy_dir = std::env::temp_dir().join(format!("kuverta-accounts-{}", std::process::id()));
    let result = (|| {
        let dir = path
            .parent()
            .ok_or_else(|| Problem::Other("no directory".into()))?;
        // Listing the directory is what macOS refuses without the permission;
        // a missing directory is simply no accounts.
        match std::fs::read_dir(dir) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(Problem::NotPermitted)
            }
            Err(err) => return Err(Problem::Other(err.to_string())),
        }
        if !path.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(&copy_dir).map_err(|err| Problem::Other(err.to_string()))?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for suffix in ["", "-wal", "-shm"] {
            let from = dir.join(format!("{name}{suffix}"));
            match std::fs::copy(&from, copy_dir.join(format!("{name}{suffix}"))) {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound && !suffix.is_empty() => {}
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                    return Err(Problem::NotPermitted)
                }
                Err(err) => return Err(Problem::Other(err.to_string())),
            }
        }
        let connection = Connection::open_with_flags(
            copy_dir.join(&name),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| Problem::Other(err.to_string()))?;
        accounts(&connection)
            .map(Some)
            .map_err(|err| Problem::Other(err.to_string()))
    })();
    let _ = std::fs::remove_dir_all(&copy_dir);
    result
}

/// Account types whose address is a mailbox kuverta can reach over IMAP.
fn is_mail(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    [
        "imap", "google", "yahoo", "aol", "hotmail", "outlook", "icloud", "mobileme",
    ]
    .iter()
    .any(|mail| kind.contains(mail))
}

pub fn accounts(connection: &Connection) -> rusqlite::Result<Vec<AppleAccount>> {
    let mut statement = connection.prepare(
        "SELECT a.Z_PK, a.ZUSERNAME, a.ZACCOUNTDESCRIPTION, t.ZIDENTIFIER
         FROM ZACCOUNT a JOIN ZACCOUNTTYPE t ON a.ZACCOUNTTYPE = t.Z_PK
         ORDER BY a.Z_PK",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut properties =
        connection.prepare("SELECT ZKEY, ZVALUE FROM ZACCOUNTPROPERTY WHERE ZOWNER = ?1")?;
    let mut found: Vec<AppleAccount> = Vec::new();
    for (id, username, description, kind) in rows {
        let kind = kind.unwrap_or_default();
        if !is_mail(&kind) {
            continue;
        }
        let props: Vec<(String, plist::Value)> = properties
            .query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
            })?
            .filter_map(|row| row.ok())
            .filter_map(|(key, value)| Some((key, plist::from_bytes(&value?).ok()?)))
            .collect();
        let prop = |names: &[&str]| {
            props
                .iter()
                .find(|(key, _)| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
                .map(|(_, value)| value)
        };
        let text = |value: &plist::Value| match value {
            plist::Value::String(text) => Some(text.clone()),
            plist::Value::Array(items) => items.iter().find_map(|item| match item {
                plist::Value::String(text) => Some(text.clone()),
                plist::Value::Dictionary(dict) => dict
                    .get("EmailAddress")
                    .and_then(|value| value.as_string())
                    .map(str::to_string),
                _ => None,
            }),
            _ => None,
        };

        let address = prop(&["IdentityEmailAddress", "EmailAddress", "EmailAddresses"])
            .and_then(text)
            .or_else(|| username.clone())
            .filter(|address| address.contains('@'));
        let Some(email) = address else {
            continue;
        };
        if found
            .iter()
            .any(|known| known.email.eq_ignore_ascii_case(&email))
        {
            continue;
        }
        let host = prop(&["Hostname", "IMAPHostname"]).and_then(text);
        let port = prop(&["PortNumber", "Port"]).and_then(|value| match value {
            plist::Value::Integer(number) => {
                number.as_unsigned().and_then(|n| u16::try_from(n).ok())
            }
            plist::Value::String(text) => text.parse().ok(),
            _ => None,
        });
        let ssl = prop(&["SSLEnabled", "UseSSL"]).and_then(|value| match value {
            plist::Value::Boolean(flag) => Some(*flag),
            plist::Value::Integer(number) => number.as_signed().map(|n| n != 0),
            plist::Value::String(text) => Some(text == "YES" || text == "1" || text == "true"),
            _ => None,
        });
        found.push(AppleAccount {
            username: username.filter(|name| !name.eq_ignore_ascii_case(&email)),
            email,
            description: description.filter(|text| !text.trim().is_empty()),
            imap: host.map(|host| (host, port, ssl)),
            kind,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(value: plist::Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.to_writer_binary(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn mail_accounts_come_from_the_system_accounts() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER VARCHAR);
             CREATE TABLE ZACCOUNT (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPE INTEGER,
                 ZUSERNAME VARCHAR, ZACCOUNTDESCRIPTION VARCHAR);
             CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER,
                 ZKEY VARCHAR, ZVALUE BLOB);
             INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.IMAP'),
                 (2, 'com.apple.account.Google'), (3, 'com.apple.account.CalDAV'),
                 (4, 'com.apple.account.SMTP');
             INSERT INTO ZACCOUNT VALUES (1, 1, 'erika', 'Arbeit'),
                 (2, 2, 'erika.m@gmail.com', 'Google'),
                 (3, 3, 'erika@example.de', 'Kalender'),
                 (4, 4, 'erika@example.de', NULL),
                 (5, 2, 'not-an-address', NULL);",
        )
        .unwrap();
        let mut insert = db
            .prepare("INSERT INTO ZACCOUNTPROPERTY (ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3)")
            .unwrap();
        insert
            .execute(rusqlite::params![
                1,
                "Hostname",
                value("imap.example.de".into())
            ])
            .unwrap();
        insert
            .execute(rusqlite::params![1, "PortNumber", value(993.into())])
            .unwrap();
        insert
            .execute(rusqlite::params![1, "SSLEnabled", value(true.into())])
            .unwrap();
        insert
            .execute(rusqlite::params![
                1,
                "EmailAddresses",
                value(plist::Value::Array(vec!["erika@example.de".into()]))
            ])
            .unwrap();
        insert
            .execute(rusqlite::params![2, "Broken", vec![1u8, 2, 3]])
            .unwrap();

        let found = accounts(&db).unwrap();
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_eq!(found[0].email, "erika@example.de");
        assert_eq!(found[0].username.as_deref(), Some("erika"));
        assert_eq!(found[0].description.as_deref(), Some("Arbeit"));
        assert_eq!(
            found[0].imap,
            Some(("imap.example.de".to_string(), Some(993), Some(true)))
        );
        assert_eq!(found[1].email, "erika.m@gmail.com");
        assert_eq!(found[1].imap, None, "a provider's account stores no server");
        assert_eq!(found[1].username, None);
    }
}
