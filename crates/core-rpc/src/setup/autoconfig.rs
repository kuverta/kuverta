//! Server settings from an email address.
//!
//! First the providers most people here use, from a table, so the common
//! case needs no network. Then the same places Thunderbird looks: the
//! provider's own autoconfig file, and Mozilla's directory of providers
//! (ISPDB). The format is the same everywhere, `config-v1.1.xml`.

use std::time::Duration;

use crate::settings::AccountInput;

/// Settings found for an address, and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub input: AccountInput,
    pub source: &'static str,
}

struct Known {
    domains: &'static [&'static str],
    imap: (&'static str, u16, &'static str),
    smtp: (&'static str, u16, &'static str),
    oauth: Option<&'static str>,
}

const KNOWN: &[Known] = &[
    Known {
        domains: &["gmail.com", "googlemail.com"],
        imap: ("imap.gmail.com", 993, "tls"),
        smtp: ("smtp.gmail.com", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["icloud.com", "me.com", "mac.com"],
        imap: ("imap.mail.me.com", 993, "tls"),
        smtp: ("smtp.mail.me.com", 587, "starttls"),
        oauth: None,
    },
    Known {
        domains: &[
            "outlook.com",
            "outlook.de",
            "hotmail.com",
            "hotmail.de",
            "live.com",
            "live.de",
            "msn.com",
        ],
        imap: ("outlook.office365.com", 993, "tls"),
        smtp: ("smtp.office365.com", 587, "starttls"),
        // Microsoft no longer takes passwords over IMAP.
        oauth: Some("microsoft"),
    },
    Known {
        domains: &["yahoo.com", "yahoo.de", "ymail.com"],
        imap: ("imap.mail.yahoo.com", 993, "tls"),
        smtp: ("smtp.mail.yahoo.com", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["aol.com", "aol.de"],
        imap: ("imap.aol.com", 993, "tls"),
        smtp: ("smtp.aol.com", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["gmx.de", "gmx.net", "gmx.at", "gmx.ch"],
        imap: ("imap.gmx.net", 993, "tls"),
        smtp: ("mail.gmx.net", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["gmx.com"],
        imap: ("imap.gmx.com", 993, "tls"),
        smtp: ("mail.gmx.com", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["web.de"],
        imap: ("imap.web.de", 993, "tls"),
        smtp: ("smtp.web.de", 587, "starttls"),
        oauth: None,
    },
    Known {
        domains: &["t-online.de", "magenta.de"],
        imap: ("secureimap.t-online.de", 993, "tls"),
        smtp: ("securesmtp.t-online.de", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["posteo.de", "posteo.net", "posteo.eu", "posteo.org"],
        imap: ("posteo.de", 993, "tls"),
        smtp: ("posteo.de", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["mailbox.org"],
        imap: ("imap.mailbox.org", 993, "tls"),
        smtp: ("smtp.mailbox.org", 465, "tls"),
        oauth: None,
    },
    Known {
        domains: &["freenet.de"],
        imap: ("mx.freenet.de", 993, "tls"),
        smtp: ("mx.freenet.de", 587, "starttls"),
        oauth: None,
    },
    Known {
        domains: &["fastmail.com", "fastmail.fm"],
        imap: ("imap.fastmail.com", 993, "tls"),
        smtp: ("smtp.fastmail.com", 465, "tls"),
        oauth: None,
    },
];

fn domain_of(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim().to_ascii_lowercase();
    (!local.is_empty() && domain.contains('.') && !domain.contains('/')).then_some(domain)
}

fn input(email: &str, imap: (&str, u16, &str), smtp: Option<(&str, u16, &str)>) -> AccountInput {
    AccountInput {
        label: email.trim().to_string(),
        email: email.trim().to_string(),
        imap_host: imap.0.to_string(),
        imap_port: imap.1,
        imap_security: imap.2.to_string(),
        smtp_host: smtp.map(|(host, _, _)| host.to_string()),
        smtp_port: smtp.map(|(_, port, _)| port),
        smtp_security: smtp.map(|(_, _, security)| security.to_string()),
        auth_method: "app_password".to_string(),
        ..AccountInput::default()
    }
}

/// Settings for the providers in the table, with no network.
pub fn known(email: &str) -> Option<Found> {
    let domain = domain_of(email)?;
    let known = KNOWN
        .iter()
        .find(|known| known.domains.contains(&domain.as_str()))?;
    let mut input = input(email, known.imap, Some(known.smtp));
    if let Some(provider) = known.oauth {
        input.auth_method = "oauth2".to_string();
        input.oauth_provider = Some(provider.to_string());
    }
    Some(Found {
        input,
        source: "kuverta's list of providers",
    })
}

/// A `config-v1.1.xml`, read for `email`: the first IMAP server, preferring
/// one with TLS from the start, and the first SMTP server likewise.
pub fn parse_config(xml: &str, email: &str) -> Option<AccountInput> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let local = email.split('@').next().unwrap_or_default();
    let domain = domain_of(email).unwrap_or_default();

    struct Server {
        host: String,
        port: u16,
        security: &'static str,
        username: Option<String>,
        oauth: bool,
    }
    let servers = |tag: &str, kind: &str| -> Vec<Server> {
        document
            .descendants()
            .filter(|node| node.has_tag_name(tag) && node.attribute("type") == Some(kind))
            .filter_map(|node| {
                let child = |name: &str| {
                    node.children()
                        .find(|child| child.has_tag_name(name))
                        .and_then(|child| child.text())
                        .map(|text| text.trim().to_string())
                };
                let security = match child("socketType")?.to_ascii_uppercase().as_str() {
                    "SSL" | "TLS" => "tls",
                    "STARTTLS" => "starttls",
                    // Never offered as a choice: a password in the clear.
                    _ => return None,
                };
                // OAuth2 only when it is the only way in.
                let methods: Vec<&str> = node
                    .children()
                    .filter(|child| child.has_tag_name("authentication"))
                    .filter_map(|child| child.text())
                    .map(str::trim)
                    .collect();
                let oauth = !methods.is_empty()
                    && methods
                        .iter()
                        .all(|method| method.eq_ignore_ascii_case("OAuth2"));
                Some(Server {
                    host: child("hostname")?
                        .replace("%EMAILDOMAIN%", &domain)
                        .to_ascii_lowercase(),
                    port: child("port")?.parse().ok()?,
                    security,
                    username: child("username").map(|name| {
                        name.replace("%EMAILADDRESS%", email.trim())
                            .replace("%EMAILLOCALPART%", local)
                            .replace("%EMAILDOMAIN%", &domain)
                    }),
                    oauth,
                })
            })
            .collect()
    };
    let best = |mut list: Vec<Server>| {
        // Stable: the file's own order among equals.
        list.sort_by_key(|server| server.security != "tls");
        list.into_iter().next()
    };

    let imap = best(servers("incomingServer", "imap"))?;
    let smtp = best(servers("outgoingServer", "smtp"));
    let mut found = input(
        email,
        (&imap.host, imap.port, imap.security),
        smtp.as_ref()
            .map(|smtp| (smtp.host.as_str(), smtp.port, smtp.security)),
    );
    found.username = imap
        .username
        .filter(|name| !name.eq_ignore_ascii_case(email.trim()));
    if imap.oauth {
        let host = imap.host.as_str();
        let provider = if host.ends_with("office365.com") || host.ends_with("outlook.com") {
            Some("microsoft")
        } else if host.ends_with("gmail.com") {
            Some("google")
        } else {
            None
        };
        if let Some(provider) = provider {
            found.auth_method = "oauth2".to_string();
            found.oauth_provider = Some(provider.to_string());
        }
    }
    Some(found)
}

/// Settings for any address: the table, then the provider's own file, then
/// Mozilla's directory. `None` when nobody says.
pub async fn discover(email: &str) -> Option<Found> {
    if let Some(found) = known(email) {
        return Some(found);
    }
    let domain = domain_of(email)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let address = encode(email.trim());
    let places: [(String, &'static str); 3] = [
        (
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={address}"),
            "the provider's own settings",
        ),
        (
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={address}"),
            "the provider's own settings",
        ),
        (
            format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
            "Mozilla's directory of providers",
        ),
    ];
    for (url, source) in places {
        let Ok(response) = http.get(&url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(xml) = response.text().await else {
            continue;
        };
        if let Some(input) = parse_config(&xml, email) {
            return Some(Found { input, source });
        }
    }
    None
}

/// A query-string value.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// What an address needs instead of its ordinary password, where that is
/// known: a text to show, and where to go.
pub fn password_help(imap_host: &str) -> Option<(&'static str, &'static str)> {
    let host = imap_host.to_ascii_lowercase();
    if host.ends_with("gmail.com") {
        Some((
            "Google needs an app password: turn on 2-Step Verification, then create one for kuverta.",
            "https://myaccount.google.com/apppasswords",
        ))
    } else if host.ends_with("mail.me.com") {
        Some((
            "iCloud needs an app-specific password, created under Sign-In and Security.",
            "https://account.apple.com/account/manage",
        ))
    } else if host.ends_with("yahoo.com") {
        Some((
            "Yahoo needs an app password, created under Account Security.",
            "https://login.yahoo.com/account/security",
        ))
    } else if host.ends_with("aol.com") {
        Some((
            "AOL needs an app password, created under Account Security.",
            "https://login.aol.com/account/security",
        ))
    } else if host.ends_with("office365.com") || host.ends_with("outlook.com") {
        Some((
            "Microsoft no longer accepts passwords over IMAP. Sign in with OAuth2: it needs an app registration's client id, then `kuverta login` in a terminal.",
            "https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth",
        ))
    } else if host.ends_with("gmx.net") || host.ends_with("gmx.com") {
        Some((
            "GMX lets mail programs in only once IMAP is switched on in its settings, under POP3/IMAP.",
            "https://www.gmx.net/",
        ))
    } else if host.ends_with("web.de") {
        Some((
            "WEB.DE lets mail programs in only once IMAP is switched on in its settings, under POP3/IMAP.",
            "https://web.de/",
        ))
    } else if host.ends_with("t-online.de") {
        Some((
            "Telekom wants its separate e-mail password here, not the login password; it is set in the Telekom Email Center.",
            "https://email.t-online.de/",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_provider_needs_no_network() {
        let found = known("Erika@GMX.de").unwrap();
        assert_eq!(found.input.imap_host, "imap.gmx.net");
        assert_eq!(found.input.smtp_host.as_deref(), Some("mail.gmx.net"));
        assert_eq!(found.input.email, "Erika@GMX.de");
        assert_eq!(found.input.auth_method, "app_password");

        let outlook = known("someone@hotmail.com").unwrap();
        assert_eq!(outlook.input.auth_method, "oauth2");
        assert_eq!(outlook.input.oauth_provider.as_deref(), Some("microsoft"));

        assert_eq!(known("someone@example.org"), None);
        assert_eq!(known("not an address"), None);
    }

    const CONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.org</hostname><port>143</port><socketType>STARTTLS</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>mail.%EMAILDOMAIN%</hostname><port>993</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>25</port><socketType>plain</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username><authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn a_config_file_gives_the_encrypted_servers() {
        let input = parse_config(CONFIG, "erika@example.org").unwrap();
        assert_eq!(input.imap_host, "mail.example.org");
        assert_eq!(input.imap_port, 993);
        assert_eq!(input.imap_security, "tls");
        assert_eq!(input.username.as_deref(), Some("erika"));
        assert_eq!(input.smtp_host.as_deref(), Some("smtp.example.org"));
        assert_eq!(input.smtp_port, Some(587));
        assert_eq!(input.smtp_security.as_deref(), Some("starttls"));
        assert_eq!(input.auth_method, "app_password");

        assert_eq!(parse_config("<not xml", "erika@example.org"), None);
        assert_eq!(
            parse_config("<clientConfig/>", "erika@example.org"),
            None,
            "no IMAP server"
        );
    }

    #[test]
    fn an_oauth_only_microsoft_server_asks_for_oauth() {
        let xml = r#"<clientConfig><emailProvider>
          <incomingServer type="imap"><hostname>outlook.office365.com</hostname><port>993</port>
            <socketType>SSL</socketType><username>%EMAILADDRESS%</username>
            <authentication>OAuth2</authentication></incomingServer>
        </emailProvider></clientConfig>"#;
        let input = parse_config(xml, "me@contoso.com").unwrap();
        assert_eq!(input.auth_method, "oauth2");
        assert_eq!(input.oauth_provider.as_deref(), Some("microsoft"));
        assert_eq!(input.username, None);
        assert_eq!(input.smtp_host, None);
    }
}
