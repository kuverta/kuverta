//! Read-only IMAP client.
//!
//! Read-only is enforced structurally, not by convention: folders are opened
//! with `EXAMINE` rather than `SELECT`, so the server itself rejects any
//! mutation. A sync bug can therefore lose cached data but never mail. This is
//! the main reason v1 is a triage layer rather than a full client.

use std::collections::HashSet;
use std::sync::Arc;

use async_imap::types::{Flag, NameAttribute};
use async_imap::{Client, Session};
use core_accounts::{AuthProvider, Credential};
use core_store::model::ImapSecurity;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::{pki_types::ServerName, ClientConfig};
use tokio_rustls::TlsConnector;
// Brings `ClientConfig::with_platform_verifier` into scope.
use rustls_platform_verifier::ConfigVerifierExt as _;

use crate::stream::Transport;
use crate::ProtoError;

pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: ImapSecurity,
    pub username: String,
}

/// A folder as the server describes it.
#[derive(Debug, Clone)]
pub struct RemoteFolder {
    pub name: String,
    /// RFC 6154 special-use attribute, e.g. `\Archive`, when the server sets one.
    pub special_use: Option<String>,
    /// `\Noselect` folders are hierarchy nodes with no messages.
    pub selectable: bool,
}

/// State reported when a folder is opened.
#[derive(Debug, Clone)]
pub struct FolderState {
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub highest_modseq: Option<u64>,
    pub exists: u32,
}

/// One message as fetched from the server, still unparsed.
pub struct RawMessage {
    pub uid: u32,
    pub flags: String,
    pub size: Option<i64>,
    pub raw: Vec<u8>,
}

pub struct ImapClient {
    session: Session<Transport>,
    condstore: bool,
}

impl ImapClient {
    /// Connects, authenticates, and enables CONDSTORE where available.
    pub async fn connect(config: &ImapConfig, auth: &dyn AuthProvider) -> Result<Self, ProtoError> {
        let transport = open_transport(config).await?;

        let mut client = Client::new(transport);
        // The server greeting must be consumed before any command is sent.
        client
            .read_response()
            .await
            .map_err(ProtoError::Io)?
            .ok_or(ProtoError::NoGreeting)?;

        let credential = auth.credential().await?;
        let mut session = match credential {
            Credential::Password(password) => client
                .login(&config.username, &password)
                .await
                .map_err(|(err, _client)| ProtoError::Login(err.to_string()))?,
            Credential::OAuthBearer { user, access_token } => {
                let authenticator = XOAuth2 { user, access_token };
                client
                    .authenticate("XOAUTH2", authenticator)
                    .await
                    .map_err(|(err, _client)| ProtoError::Login(err.to_string()))?
            }
        };

        // CONDSTORE makes the server report HIGHESTMODSEQ, which later turns a
        // full folder scan into an incremental one. Servers without it still
        // work; sync just falls back to comparing UIDs.
        let condstore = session
            .run_command_and_check_ok("ENABLE CONDSTORE")
            .await
            .is_ok();
        if !condstore {
            tracing::info!(host = %config.host, "server did not accept ENABLE CONDSTORE");
        }

        Ok(Self { session, condstore })
    }

    pub fn condstore_enabled(&self) -> bool {
        self.condstore
    }

    pub async fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProtoError> {
        let stream = self.session.list(Some(""), Some("*")).await?;
        let names: Vec<_> = stream.try_collect().await?;

        Ok(names
            .iter()
            .map(|name| RemoteFolder {
                name: name.name().to_string(),
                special_use: name.attributes().iter().find_map(special_use),
                selectable: !name
                    .attributes()
                    .iter()
                    .any(|a| matches!(a, NameAttribute::NoSelect)),
            })
            .collect())
    }

    /// Opens a folder read-only.
    pub async fn examine(&mut self, folder: &str) -> Result<FolderState, ProtoError> {
        let mailbox = self.session.examine(folder).await?;
        Ok(FolderState {
            uid_validity: mailbox.uid_validity,
            uid_next: mailbox.uid_next,
            highest_modseq: mailbox.highest_modseq,
            exists: mailbox.exists,
        })
    }

    /// Fetches messages with a UID greater than `after`, or all of them when
    /// `after` is `None`.
    ///
    /// `state` is required to guard an IMAP quirk: in a range like `900:*`, `*`
    /// means the *highest existing UID*, so if 900 is beyond the end the server
    /// helpfully returns the last message instead of nothing. Without the
    /// check, every sync of an unchanged folder would re-fetch its newest mail.
    pub async fn fetch_since(
        &mut self,
        state: &FolderState,
        after: Option<u32>,
    ) -> Result<Vec<RawMessage>, ProtoError> {
        let start = after.map_or(1, |uid| uid.saturating_add(1));

        if let Some(uid_next) = state.uid_next {
            if start >= uid_next {
                return Ok(Vec::new());
            }
        } else if state.exists == 0 {
            return Ok(Vec::new());
        }

        // BODY.PEEK[] rather than BODY[]: the latter sets \Seen. EXAMINE already
        // prevents that, but the two together mean neither alone is load-bearing.
        let query = "(UID FLAGS RFC822.SIZE BODY.PEEK[])";
        let range = format!("{start}:*");

        let mut stream = self.session.uid_fetch(range, query).await?;
        let mut messages = Vec::new();

        while let Some(fetch) = stream.try_next().await? {
            let Some(uid) = fetch.uid else {
                // Cannot be stored without an identity in the folder; skip
                // rather than invent one.
                tracing::warn!(seq = fetch.message, "FETCH response without a UID");
                continue;
            };
            let Some(body) = fetch.body() else {
                tracing::warn!(uid, "FETCH response without a body");
                continue;
            };

            messages.push(RawMessage {
                uid,
                flags: format_flags(fetch.flags()),
                size: fetch.size.map(|s| s as i64),
                raw: body.to_vec(),
            });
        }

        Ok(messages)
    }

    /// Fetches flags for messages whose state changed after `since_modseq`.
    ///
    /// With a modseq this is a `CHANGEDSINCE` fetch, so the server sends only
    /// what actually moved — usually nothing. Without one (a first sync, or a
    /// server with no CONDSTORE) it falls back to fetching every flag, which is
    /// correct but proportional to the mailbox.
    pub async fn fetch_flag_changes(
        &mut self,
        since_modseq: Option<u64>,
    ) -> Result<Vec<(u32, String)>, ProtoError> {
        let query = match since_modseq {
            Some(modseq) => format!("(UID FLAGS) (CHANGEDSINCE {modseq})"),
            None => "(UID FLAGS)".to_string(),
        };

        let mut stream = self.session.uid_fetch("1:*", query).await?;
        let mut changes = Vec::new();

        while let Some(fetch) = stream.try_next().await? {
            if let Some(uid) = fetch.uid {
                changes.push((uid, format_flags(fetch.flags())));
            }
        }

        Ok(changes)
    }

    /// Every UID the server currently holds in the open folder.
    ///
    /// Compared against the local cache this is how expunges are detected.
    /// QRESYNC would deliver `VANISHED` directly and avoid the full listing,
    /// but this works on every server including those without CONDSTORE.
    pub async fn all_uids(&mut self) -> Result<HashSet<u32>, ProtoError> {
        Ok(self.session.uid_search("ALL").await?)
    }

    pub async fn logout(mut self) -> Result<(), ProtoError> {
        self.session.logout().await?;
        Ok(())
    }
}

async fn open_transport(config: &ImapConfig) -> Result<Transport, ProtoError> {
    let tcp = TcpStream::connect((config.host.as_str(), config.port)).await?;
    // Mail is many small commands; Nagle adds latency for no benefit.
    tcp.set_nodelay(true).ok();

    match config.security {
        ImapSecurity::Plaintext => Ok(Transport::Plain(tcp)),
        ImapSecurity::Tls => {
            // Trust the OS trust store rather than a bundled root list, so the
            // user's own enterprise or pinned roots keep working.
            let tls_config = ClientConfig::with_platform_verifier()
                .map_err(|e| ProtoError::Tls(e.to_string()))?;
            let connector = TlsConnector::from(Arc::new(tls_config));
            let server_name = ServerName::try_from(config.host.clone())
                .map_err(|_| ProtoError::InvalidHostname(config.host.clone()))?;
            let tls = connector.connect(server_name, tcp).await?;
            Ok(Transport::Tls(Box::new(tls)))
        }
        ImapSecurity::StartTls => Err(ProtoError::Unsupported(
            "STARTTLS is not implemented; use implicit TLS on port 993",
        )),
    }
}

/// Renders flags back into their IMAP wire form for storage.
fn format_flags<'a>(flags: impl Iterator<Item = Flag<'a>>) -> String {
    flags
        .map(|flag| match flag {
            Flag::Seen => "\\Seen".to_string(),
            Flag::Answered => "\\Answered".to_string(),
            Flag::Flagged => "\\Flagged".to_string(),
            Flag::Deleted => "\\Deleted".to_string(),
            Flag::Draft => "\\Draft".to_string(),
            Flag::Recent => "\\Recent".to_string(),
            Flag::MayCreate => "\\*".to_string(),
            Flag::Custom(name) => name.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn special_use(attr: &NameAttribute<'_>) -> Option<String> {
    match attr {
        NameAttribute::All => Some("\\All".into()),
        NameAttribute::Archive => Some("\\Archive".into()),
        NameAttribute::Drafts => Some("\\Drafts".into()),
        NameAttribute::Flagged => Some("\\Flagged".into()),
        NameAttribute::Junk => Some("\\Junk".into()),
        NameAttribute::Sent => Some("\\Sent".into()),
        NameAttribute::Trash => Some("\\Trash".into()),
        _ => None,
    }
}

/// SASL XOAUTH2, as used by Microsoft 365 and Gmail.
///
/// Unused until the OAuth provider lands in month 3, but implemented now so the
/// credential enum has a real consumer rather than a notional one.
struct XOAuth2 {
    user: String,
    access_token: String,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}
