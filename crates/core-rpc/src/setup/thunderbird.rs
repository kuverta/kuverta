//! Accounts from Thunderbird's profiles.
//!
//! Thunderbird keeps its account settings in each profile's `prefs.js`, as
//! `user_pref("key", value);` lines, and lists its profiles in
//! `profiles.ini`. Both are plain text and readable without Thunderbird
//! running. Passwords are not in either — they are in `logins.json`,
//! encrypted — so an import brings the servers and the person enters the
//! password once more.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::settings::AccountInput;

/// An account as Thunderbird has it.
#[derive(Debug, Clone, PartialEq)]
pub struct TbAccount {
    pub input: AccountInput,
    pub full_name: Option<String>,
    /// Whether Thunderbird connected without encryption, which kuverta will
    /// not do for anything but this computer; the import asks for STARTTLS.
    pub was_cleartext: bool,
}

/// Where Thunderbird keeps its profiles on this system, in the order they are
/// looked in.
pub fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Library/Thunderbird"));
        roots.push(home.join(".thunderbird"));
        roots.push(home.join("snap/thunderbird/common/.thunderbird"));
        roots.push(home.join(".var/app/org.mozilla.Thunderbird/.thunderbird"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        roots.push(appdata.join("Thunderbird"));
    }
    roots
}

/// The profile directories `profiles.ini` in `root` names, default first.
pub fn profiles(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let ini = std::fs::read_to_string(root.join("profiles.ini"))?;
    let mut found = Vec::new();
    let mut section: HashMap<String, String> = HashMap::new();
    let mut flush = |section: &mut HashMap<String, String>| {
        if let Some(path) = section.get("Path") {
            let relative = section.get("IsRelative").is_none_or(|value| value == "1");
            let dir = if relative {
                root.join(path)
            } else {
                PathBuf::from(path)
            };
            let default = section.get("Default").is_some_and(|value| value == "1");
            found.push((default, dir));
        }
        section.clear();
    };
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            flush(&mut section);
        } else if let Some((key, value)) = line.split_once('=') {
            section.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    flush(&mut section);
    // The default profile first, so its accounts win when two profiles have
    // the same address.
    found.sort_by_key(|(default, _)| !default);
    let mut dirs: Vec<PathBuf> = Vec::new();
    for (_, dir) in found {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

/// A preference's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Pref {
    Text(String),
    Number(i64),
    Flag(bool),
}

impl Pref {
    fn text(&self) -> Option<&str> {
        match self {
            Pref::Text(text) => Some(text),
            _ => None,
        }
    }

    fn number(&self) -> Option<i64> {
        match self {
            Pref::Number(number) => Some(*number),
            Pref::Text(text) => text.trim().parse().ok(),
            Pref::Flag(_) => None,
        }
    }
}

/// Every `user_pref("key", value);` in a `prefs.js`. Lines that are not one
/// are skipped: the file is Thunderbird's to write, and a line this cannot
/// read should cost that line, not the import.
pub fn parse_prefs(text: &str) -> HashMap<String, Pref> {
    let mut prefs = HashMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("user_pref(") else {
            continue;
        };
        let Some((key, rest)) = string_literal(rest.trim_start()) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(',') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = if rest.starts_with('"') {
            match string_literal(rest) {
                Some((text, _)) => Pref::Text(text),
                None => continue,
            }
        } else {
            let raw = rest.trim_end().trim_end_matches(';').trim_end();
            let raw = raw.strip_suffix(')').unwrap_or(raw).trim();
            match raw {
                "true" => Pref::Flag(true),
                "false" => Pref::Flag(false),
                number => match number.parse() {
                    Ok(number) => Pref::Number(number),
                    Err(_) => continue,
                },
            }
        };
        prefs.insert(key, value);
    }
    prefs
}

/// A JavaScript string in double quotes at the start of `text`, unescaped,
/// and what follows it.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return Some((out, &text[at + 2..])),
            '\\' => {
                let (_, escaped) = chars.next()?;
                match escaped {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = (0..4)
                            .filter_map(|_| chars.next().map(|(_, c)| c))
                            .collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

/// Thunderbird's `socketType` and `try_ssl`: 3 is TLS from the start, 2 is
/// STARTTLS, anything else is none.
fn security(value: Option<i64>) -> (&'static str, bool) {
    match value {
        Some(3) => ("tls", false),
        Some(2) => ("starttls", false),
        _ => ("starttls", true),
    }
}

/// `authMethod` 10 is OAuth2.
const OAUTH2: i64 = 10;

/// Which provider an OAuth2 server belongs to, for the servers kuverta can
/// sign in to that way.
fn oauth_provider(host: &str) -> Option<&'static str> {
    let host = host.to_ascii_lowercase();
    if host.ends_with("gmail.com") || host.ends_with("googlemail.com") {
        Some("google")
    } else if host.ends_with("office365.com") || host.ends_with("outlook.com") {
        Some("microsoft")
    } else {
        None
    }
}

/// The IMAP accounts in one profile's preferences, in Thunderbird's order.
pub fn accounts(prefs: &HashMap<String, Pref>) -> Vec<TbAccount> {
    let get = |key: String| prefs.get(&key);
    let list = |key: &str| -> Vec<String> {
        prefs
            .get(key)
            .and_then(Pref::text)
            .map(|text| {
                text.split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut found = Vec::new();
    for account in list("mail.accountmanager.accounts") {
        let Some(server) = get(format!("mail.account.{account}.server")).and_then(Pref::text)
        else {
            continue;
        };
        let server_pref = |name: &str| get(format!("mail.server.{server}.{name}"));
        if server_pref("type").and_then(Pref::text) != Some("imap") {
            continue;
        }
        let Some(host) = server_pref("hostname").and_then(Pref::text) else {
            continue;
        };

        let identities: Vec<String> = prefs
            .get(&format!("mail.account.{account}.identities"))
            .and_then(Pref::text)
            .map(|text| text.split(',').map(|id| id.trim().to_string()).collect())
            .unwrap_or_default();
        let identity = identities.first();
        let identity_pref =
            |name: &str| identity.and_then(|id| get(format!("mail.identity.{id}.{name}")));

        let username = server_pref("userName")
            .and_then(Pref::text)
            .unwrap_or_default()
            .to_string();
        let email = identity_pref("useremail")
            .and_then(Pref::text)
            .map(str::to_string)
            .or_else(|| username.contains('@').then(|| username.clone()));
        let Some(email) = email else {
            continue;
        };

        let (imap_security, imap_cleartext) =
            security(server_pref("socketType").and_then(Pref::number));
        let imap_port = server_pref("port")
            .and_then(Pref::number)
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(if imap_security == "tls" { 993 } else { 143 });

        // The identity's outgoing server, or the default one.
        let smtp = identity_pref("smtpServer")
            .and_then(Pref::text)
            .map(str::to_string)
            .or_else(|| {
                prefs
                    .get("mail.smtp.defaultserver")
                    .and_then(Pref::text)
                    .map(str::to_string)
            });
        let smtp_pref = |name: &str| {
            smtp.as_ref()
                .and_then(|id| get(format!("mail.smtpserver.{id}.{name}")))
        };
        let smtp_host = smtp_pref("hostname")
            .and_then(Pref::text)
            .map(str::to_string);
        let (smtp_security, smtp_cleartext) = security(smtp_pref("try_ssl").and_then(Pref::number));
        let smtp_port = smtp_host.as_ref().map(|_| {
            smtp_pref("port")
                .and_then(Pref::number)
                .and_then(|port| u16::try_from(port).ok())
                .filter(|&port| port != 0)
                .unwrap_or(if smtp_security == "tls" { 465 } else { 587 })
        });

        let oauth = server_pref("authMethod").and_then(Pref::number) == Some(OAUTH2);
        let provider = oauth.then(|| oauth_provider(host)).flatten();
        let label = server_pref("name")
            .and_then(Pref::text)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&email)
            .to_string();

        found.push(TbAccount {
            input: AccountInput {
                id: None,
                label,
                username: (!username.is_empty() && username != email).then_some(username),
                email,
                imap_host: host.to_string(),
                imap_port,
                imap_security: imap_security.to_string(),
                smtp_security: smtp_host.as_ref().map(|_| smtp_security.to_string()),
                smtp_host,
                smtp_port,
                // An OAuth2 account kuverta cannot sign in to that way comes
                // in as a password account, for the person to decide.
                auth_method: if provider.is_some() {
                    "oauth2".to_string()
                } else {
                    "app_password".to_string()
                },
                oauth_provider: provider.map(str::to_string),
                oauth_client_id: None,
                oauth_tenant: None,
                excluded_folders: Vec::new(),
            },
            full_name: identity_pref("fullName")
                .and_then(Pref::text)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_string),
            was_cleartext: imap_cleartext || (smtp.is_some() && smtp_cleartext),
        });
    }
    found
}

/// Every IMAP account in every Thunderbird profile on this system, each
/// address once. `Ok(None)` when Thunderbird has never run here.
pub fn scan() -> std::io::Result<Option<Vec<TbAccount>>> {
    let mut any_root = false;
    let mut found: Vec<TbAccount> = Vec::new();
    for root in roots() {
        let dirs = match profiles(&root) {
            Ok(dirs) => dirs,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        any_root = true;
        for dir in dirs {
            let Ok(text) = std::fs::read_to_string(dir.join("prefs.js")) else {
                continue;
            };
            for account in accounts(&parse_prefs(&text)) {
                let seen = found
                    .iter()
                    .any(|known| known.input.email.eq_ignore_ascii_case(&account.input.email));
                if !seen {
                    found.push(account);
                }
            }
        }
    }
    Ok(any_root.then_some(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFS: &str = r#"// Mozilla User Preferences
user_pref("mail.accountmanager.accounts", "account1,account2,account3,account4");
user_pref("mail.account.account1.identities", "id1");
user_pref("mail.account.account1.server", "server1");
user_pref("mail.account.account2.server", "server2");
user_pref("mail.account.account3.identities", "id3");
user_pref("mail.account.account3.server", "server3");
user_pref("mail.account.account4.identities", "id4");
user_pref("mail.account.account4.server", "server4");
user_pref("mail.identity.id1.fullName", "Erika Mustermann");
user_pref("mail.identity.id1.smtpServer", "smtp1");
user_pref("mail.identity.id1.useremail", "erika@example.de");
user_pref("mail.identity.id3.useremail", "erika.m@gmail.com");
user_pref("mail.identity.id4.useremail", "old@example.org");
user_pref("mail.server.server1.hostname", "imap.example.de");
user_pref("mail.server.server1.name", "Arbeit \"Büro\"");
user_pref("mail.server.server1.port", 993);
user_pref("mail.server.server1.socketType", 3);
user_pref("mail.server.server1.type", "imap");
user_pref("mail.server.server1.userName", "erika");
user_pref("mail.server.server2.hostname", "Local Folders");
user_pref("mail.server.server2.type", "none");
user_pref("mail.server.server3.authMethod", 10);
user_pref("mail.server.server3.hostname", "imap.gmail.com");
user_pref("mail.server.server3.port", 993);
user_pref("mail.server.server3.socketType", 3);
user_pref("mail.server.server3.type", "imap");
user_pref("mail.server.server3.userName", "erika.m@gmail.com");
user_pref("mail.server.server4.hostname", "mail.example.org");
user_pref("mail.server.server4.type", "imap");
user_pref("mail.server.server4.userName", "old@example.org");
user_pref("mail.smtp.defaultserver", "smtp2");
user_pref("mail.smtpserver.smtp1.hostname", "smtp.example.de");
user_pref("mail.smtpserver.smtp1.port", 587);
user_pref("mail.smtpserver.smtp1.try_ssl", 2);
user_pref("mail.smtpserver.smtp2.hostname", "smtp.gmail.com");
user_pref("mail.smtpserver.smtp2.try_ssl", 3);
user_pref("mail.smtpserver.smtp2.port", 0);
user_pref("some.flag", true);
user_pref("some.escaped", "tab\there é");
"#;

    #[test]
    fn preferences_are_read_with_their_types_and_escapes() {
        let prefs = parse_prefs(PREFS);
        assert_eq!(
            prefs["mail.server.server1.name"],
            Pref::Text("Arbeit \"Büro\"".into())
        );
        assert_eq!(prefs["mail.server.server1.port"], Pref::Number(993));
        assert_eq!(prefs["some.flag"], Pref::Flag(true));
        assert_eq!(prefs["some.escaped"], Pref::Text("tab\there é".into()));
    }

    #[test]
    fn imap_accounts_come_with_their_outgoing_server() {
        let found = accounts(&parse_prefs(PREFS));
        assert_eq!(
            found.len(),
            3,
            "local folders are not an account: {found:#?}"
        );

        let work = &found[0];
        assert_eq!(work.input.email, "erika@example.de");
        assert_eq!(work.input.label, "Arbeit \"Büro\"");
        assert_eq!(work.input.username.as_deref(), Some("erika"));
        assert_eq!(
            (
                work.input.imap_host.as_str(),
                work.input.imap_port,
                work.input.imap_security.as_str()
            ),
            ("imap.example.de", 993, "tls")
        );
        assert_eq!(work.input.smtp_host.as_deref(), Some("smtp.example.de"));
        assert_eq!(work.input.smtp_port, Some(587));
        assert_eq!(work.input.smtp_security.as_deref(), Some("starttls"));
        assert_eq!(work.input.auth_method, "app_password");
        assert_eq!(work.full_name.as_deref(), Some("Erika Mustermann"));
        assert!(!work.was_cleartext);

        let gmail = &found[1];
        assert_eq!(gmail.input.auth_method, "oauth2");
        assert_eq!(gmail.input.oauth_provider.as_deref(), Some("google"));
        assert_eq!(gmail.input.username, None, "the same as the address");
        // No identity server: the default one, on its TLS port since the
        // stored port is 0.
        assert_eq!(gmail.input.smtp_host.as_deref(), Some("smtp.gmail.com"));
        assert_eq!(gmail.input.smtp_port, Some(465));

        let old = &found[2];
        assert_eq!(old.input.imap_security, "starttls");
        assert_eq!(old.input.imap_port, 143);
        assert!(
            old.was_cleartext,
            "no socketType is no encryption in Thunderbird"
        );
    }

    #[test]
    fn profiles_ini_lists_the_default_profile_first() {
        let root = std::env::temp_dir().join(format!("kuverta-tb-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile1]\nName=old\nIsRelative=1\nPath=Profiles/a.old\n\n\
             [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/b.default-release\nDefault=1\n\n\
             [Profile2]\nName=elsewhere\nIsRelative=0\nPath=/mnt/tb\n\n[General]\nVersion=2\n",
        )
        .unwrap();
        let dirs = profiles(&root).unwrap();
        assert_eq!(
            dirs,
            vec![
                root.join("Profiles/b.default-release"),
                root.join("Profiles/a.old"),
                PathBuf::from("/mnt/tb"),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
