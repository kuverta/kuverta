//! IMAP client: read-only, plus `APPEND`.
//!
//! Read-only is enforced structurally, not by convention: folders are opened
//! with `EXAMINE` rather than `SELECT`, so the server itself rejects any
//! mutation. A sync bug can therefore lose cached data but never mail.
//!
//! [`ImapClient::append`] is the one write, and it does not weaken that.
//! `APPEND` creates a message; it cannot modify or remove one, and it names its
//! target mailbox rather than acting on a selected one — so no folder is ever
//! opened writable. It exists so a sent message can be filed in Sent, which is
//! stage 1 of the write capability in docs/implementation-plan.md section 1a.
//! Stage 2 is where `SELECT`, `STORE` and `EXPUNGE` arrive, and where this
//! property is deliberately given up.

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
// Brings `with_platform_verifier` into scope on the config builder.
use rustls_platform_verifier::BuilderVerifierExt as _;

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
    /// Answers to `CAPABILITY`, cached per connection. The set does not change
    /// mid-session once authenticated, and the move path would otherwise ask
    /// twice per message.
    capabilities: std::collections::HashMap<String, bool>,
}

/// How a move actually happened, since not every server can do it atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// RFC 6851 `UID MOVE`.
    Moved,
    /// Copied, flagged `\\Deleted`, and expunged via RFC 4315 `UID EXPUNGE`.
    CopiedAndExpunged,
    /// Copied and flagged `\\Deleted`, but the source copy is still there:
    /// the server offers neither extension. See [`ImapClient::uid_move`].
    CopiedAndFlagged,
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

        Ok(Self {
            session,
            condstore,
            capabilities: std::collections::HashMap::new(),
        })
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

    /// Opens a folder for writing.
    ///
    /// The counterpart to [`Self::examine`], and the point at which this client
    /// stops being structurally incapable of changing a mailbox. Everything
    /// that calls it goes through the executor in [`crate::mutate`], which
    /// verifies what it is about to touch first.
    pub async fn select(&mut self, folder: &str) -> Result<FolderState, ProtoError> {
        let mailbox = self.session.select(folder).await?;
        Ok(FolderState {
            uid_validity: mailbox.uid_validity,
            uid_next: mailbox.uid_next,
            highest_modseq: mailbox.highest_modseq,
            exists: mailbox.exists,
        })
    }

    /// Reads back the Message-ID stored at `uid` in the open folder.
    ///
    /// This is the conflict check. UIDs are stable within a UIDVALIDITY, but
    /// "stable" only means the server will not reissue one — it says nothing
    /// about the message still being there, and a stale UID acted on blindly is
    /// how a client archives the wrong mail. `None` means there is no message
    /// at that UID any more.
    ///
    /// `BODY.PEEK` rather than `BODY`, so the check itself does not mark
    /// anything read.
    pub async fn uid_message_id(&mut self, uid: u32) -> Result<Option<String>, ProtoError> {
        let mut stream = self
            .session
            .uid_fetch(uid.to_string(), "BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]")
            .await?;

        let mut found = None;
        while let Some(fetch) = stream.try_next().await? {
            let Some(header) = fetch.header() else {
                continue;
            };
            let text = String::from_utf8_lossy(header);
            found = text
                .split_once(':')
                .map(|(_, value)| value.trim().trim_start_matches('<').trim_end_matches('>'))
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        Ok(found)
    }

    /// Moves `uid` out of the open folder and into `target`.
    ///
    /// Prefers RFC 6851 `UID MOVE`, which is atomic. Without it the move is a
    /// copy, a `\\Deleted` flag, and — only if the server offers RFC 4315
    /// `UIDPLUS` — a `UID EXPUNGE` of that one message.
    ///
    /// A bare `EXPUNGE` is deliberately never sent: it removes every
    /// `\\Deleted` message in the folder, including ones another client marked,
    /// so on a server with neither extension the copy is left behind flagged
    /// rather than risk destroying mail this client was never asked to touch.
    /// Both extensions are near-universal; Dovecot, Gmail and M365 all have
    /// them.
    pub async fn uid_move(&mut self, uid: u32, target: &str) -> Result<MoveOutcome, ProtoError> {
        if self.capability("MOVE").await? {
            self.session.uid_mv(uid.to_string(), target).await?;
            return Ok(MoveOutcome::Moved);
        }

        self.session.uid_copy(uid.to_string(), target).await?;
        self.uid_store_flag(uid, "\\Deleted", true).await?;

        if self.capability("UIDPLUS").await? {
            // Boxed because the expunge stream is not Unpin, and the
            // response has to be drained before the next command goes out.
            let mut stream = Box::pin(self.session.uid_expunge(uid.to_string()).await?);
            while stream.try_next().await?.is_some() {}
            Ok(MoveOutcome::CopiedAndExpunged)
        } else {
            tracing::warn!(
                uid,
                target,
                "server has neither MOVE nor UIDPLUS; the copy in the source folder is \
                 flagged \\Deleted but left in place"
            );
            Ok(MoveOutcome::CopiedAndFlagged)
        }
    }

    /// Adds or removes one flag on `uid` in the open folder.
    pub async fn uid_store_flag(
        &mut self,
        uid: u32,
        flag: &str,
        set: bool,
    ) -> Result<(), ProtoError> {
        let op = if set {
            "+FLAGS.SILENT"
        } else {
            "-FLAGS.SILENT"
        };
        let mut stream = self
            .session
            .uid_store(uid.to_string(), format!("{op} ({flag})"))
            .await?;
        // .SILENT asks the server not to report the new flags, but the response
        // still has to be drained before the next command can be sent.
        while stream.try_next().await?.is_some() {}
        Ok(())
    }

    /// Whether the server advertises `name`, asking it at most once.
    async fn capability(&mut self, name: &str) -> Result<bool, ProtoError> {
        if let Some(known) = self.capabilities.get(name) {
            return Ok(*known);
        }
        let has = self.session.capabilities().await?.has_str(name);
        self.capabilities.insert(name.to_string(), has);
        Ok(has)
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

    /// Appends a message to `folder`, as [`RFC 3501 section
    /// 6.3.11`](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.11).
    ///
    /// Additive by construction: it can only create a message, and it takes the
    /// mailbox as an argument rather than acting on a selected one, so nothing
    /// is ever opened writable. See the module docs.
    ///
    /// `flags` are IMAP flag names such as `\\Seen`. A message filed in Sent
    /// should carry `\\Seen`: the sender has, by definition, read it, and
    /// without it every client shows the Sent folder as full of unread mail.
    pub async fn append(
        &mut self,
        folder: &str,
        flags: &[&str],
        raw: &[u8],
    ) -> Result<(), ProtoError> {
        let flags = (!flags.is_empty()).then(|| format!("({})", flags.join(" ")));
        self.session
            .append(folder, flags.as_deref(), None, raw)
            .await?;
        Ok(())
    }

    pub async fn logout(mut self) -> Result<(), ProtoError> {
        self.session.logout().await?;
        Ok(())
    }
}

/// Picks the folder that sent mail belongs in.
///
/// `\\Sent` (RFC 6154) when the server declares it, which is the only reliable
/// answer. The name fallbacks exist because plenty of servers — including
/// Dovecot in its default configuration — advertise no special-use attributes
/// at all, and guessing from a known list beats creating a second Sent folder
/// alongside the one the user's other clients already use.
pub fn find_sent(folders: &[RemoteFolder]) -> Option<&RemoteFolder> {
    if let Some(declared) = folders
        .iter()
        .find(|folder| folder.special_use.as_deref() == Some("\\Sent"))
    {
        return Some(declared);
    }

    const KNOWN_NAMES: &[&str] = &[
        "Sent",
        "Sent Items",
        "Sent Messages",
        "INBOX.Sent",
        // German, for the same reason the classifier is bilingual.
        "Gesendet",
        "Gesendete Objekte",
        "Gesendete Elemente",
    ];

    folders.iter().find(|folder| {
        folder.selectable
            && KNOWN_NAMES
                .iter()
                .any(|name| folder.name.eq_ignore_ascii_case(name))
    })
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
            //
            // The crypto provider is named rather than left to rustls to infer.
            // Since `core-smtp` arrived the workspace links two of them —
            // reqwest's rustls pulls aws-lc-rs, mail-send pulls ring — and with
            // both present rustls cannot choose a process-wide default, so
            // `ClientConfig::with_platform_verifier()` panics at the first
            // handshake. `core_smtp::submit::tls_connector` is the same code
            // for the same reason; keep the two in step.
            let connector = tls_connector()?;
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

fn tls_connector() -> Result<TlsConnector, ProtoError> {
    let tls = |e: tokio_rustls::rustls::Error| ProtoError::Tls(e.to_string());

    let config = ClientConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(tls)?
    .with_platform_verifier()
    .map_err(tls)?
    .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, special_use: Option<&str>) -> RemoteFolder {
        RemoteFolder {
            name: name.into(),
            special_use: special_use.map(Into::into),
            selectable: true,
        }
    }

    #[test]
    fn the_sent_folder_is_found_by_attribute_before_name() {
        // A server that declares \\Sent on a folder named something else is
        // right and the name list is wrong; trust the attribute.
        let folders = vec![
            folder("INBOX", None),
            folder("Sent", None),
            folder("Verschickt", Some("\\Sent")),
        ];
        assert_eq!(find_sent(&folders).unwrap().name, "Verschickt");
    }

    #[test]
    fn the_sent_folder_falls_back_to_known_names() {
        let folders = vec![
            folder("INBOX", None),
            folder("Gesendete Objekte", None),
            folder("Trash", Some("\\Trash")),
        ];
        assert_eq!(find_sent(&folders).unwrap().name, "Gesendete Objekte");

        // Rather than inventing one when there is nothing to go on: the caller
        // has to decide whether to create a folder, not this function.
        let bare = vec![folder("INBOX", None), folder("Archive", Some("\\Archive"))];
        assert!(find_sent(&bare).is_none());
    }

    #[test]
    fn a_tls_connector_can_be_built() {
        // Regression test with no server in it. Two crypto providers are linked
        // into this workspace, so any rustls config that leaves the choice to
        // the process-wide default panics here rather than returning an error —
        // and it would do so at the first real TLS sync, which is exactly the
        // path no test reaches (the dev server is plaintext).
        assert!(tls_connector().is_ok());
    }
}
