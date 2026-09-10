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

/// Most messages to pull in one `FETCH`, regardless of how small they are.
///
/// A cap on the command length and on the per-response bookkeeping, not on
/// memory — [`MAX_BATCH_BYTES`] does that.
pub const MAX_BATCH_MESSAGES: usize = 200;

/// Roughly how many bytes of message body to hold in memory at once.
///
/// The first sync of a real mailbox is the case this exists for: fetching
/// `1:*` in one command means the entire mailbox is resident before a single
/// row is written, which on a mailbox of any size is an out-of-memory kill
/// rather than a slow sync.
pub const MAX_BATCH_BYTES: u64 = 16 * 1024 * 1024;

/// Groups `(uid, size)` pairs into batches that respect both caps.
///
/// A single message larger than [`MAX_BATCH_BYTES`] gets a batch to itself:
/// refusing to fetch it would be worse than the memory spike, and a 30 MB
/// attachment is unusual but not pathological.
pub fn plan_batches(sizes: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut batches = Vec::new();
    let mut batch: Vec<u32> = Vec::new();
    let mut bytes: u64 = 0;

    for (uid, size) in sizes {
        let size = *size as u64;
        if !batch.is_empty()
            && (batch.len() >= MAX_BATCH_MESSAGES || bytes + size > MAX_BATCH_BYTES)
        {
            batches.push(std::mem::take(&mut batch));
            bytes = 0;
        }
        batch.push(*uid);
        bytes += size;
    }

    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

/// The IMAP extensions this client knows how to take advantage of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extensions {
    /// Turns a full folder scan into an incremental one.
    pub condstore: bool,
    /// Would replace the `UID SEARCH ALL` expunge reconciliation, which is
    /// currently proportional to folder size. Not used yet.
    pub qresync: bool,
    /// Makes archiving and deleting atomic. Without it, see
    /// [`ImapClient::uid_move`].
    pub r#move: bool,
    /// Lets a single message be expunged, which is what makes the fallback
    /// move safe to complete.
    pub uidplus: bool,
    /// Push, rather than polling. Not used yet.
    pub idle: bool,
}

/// What a [`ImapClient::uid_probe`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UidProbe {
    /// Nothing at that UID any more. UIDs are never reused within a
    /// UIDVALIDITY, so this means the message left, not that it changed.
    Vacant,
    /// A message is there. `message_id` is `None` when it carries no
    /// `Message-ID` header, which is legal.
    Present { message_id: Option<String> },
}

/// Pulls the value out of a one-header `HEADER.FIELDS` response.
fn header_message_id(header: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(header);
    text.split_once(':')
        .map(|(_, value)| value.trim().trim_start_matches('<').trim_end_matches('>'))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
        let opened = open_transport(config).await?;

        let mut client = Client::new(opened.transport);
        // The server greeting must be consumed before any command is sent —
        // unless STARTTLS already did, since no second greeting follows the
        // upgrade.
        if !opened.greeting_consumed {
            client
                .read_response()
                .await
                .map_err(ProtoError::Io)?
                .ok_or(ProtoError::NoGreeting)?;
        }

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

    /// Looks at what is actually at `uid` in the open folder.
    ///
    /// This is the conflict check. UIDs are stable within a UIDVALIDITY, but
    /// "stable" only means the server will not reissue one — it says nothing
    /// about the message still being there, and a stale UID acted on blindly is
    /// how a client archives the wrong mail.
    ///
    /// The distinction [`UidProbe`] draws is not pedantry: a message with no
    /// `Message-ID` header is unusual but legal, and one is in the dev
    /// fixtures. Reading "no Message-ID came back" as "no message here" would
    /// quietly make such mail impossible to file.
    ///
    /// `BODY.PEEK` rather than `BODY`, so the check itself does not mark
    /// anything read.
    pub async fn uid_probe(&mut self, uid: u32) -> Result<UidProbe, ProtoError> {
        let mut stream = self
            .session
            .uid_fetch(uid.to_string(), "BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]")
            .await?;

        // A UID with nothing at it draws no FETCH response at all, which is
        // what separates the two cases.
        let mut probe = UidProbe::Vacant;
        while let Some(fetch) = stream.try_next().await? {
            probe = UidProbe::Present {
                message_id: fetch.header().and_then(header_message_id),
            };
        }
        Ok(probe)
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

    /// Creates a folder, marking it with an RFC 6154 special-use attribute
    /// where the server allows it.
    ///
    /// Explicit rather than implicit: `archive` does not conjure a folder into
    /// existence as a side effect of filing a message. Creating a mailbox on
    /// someone's mail server is the kind of thing that should happen because
    /// they asked, not because a command needed somewhere to put something.
    ///
    /// Returns whether the special-use attribute was applied. Dovecot — and so
    /// most of the small providers — advertises `SPECIAL-USE`, meaning it
    /// reports attributes in `LIST`, but not `CREATE-SPECIAL-USE`, meaning it
    /// will not let you set one. There the folder is created plainly and found
    /// by name instead, which is why [`find_archive`] keeps a name fallback.
    pub async fn create_folder(
        &mut self,
        name: &str,
        special_use: Option<&str>,
    ) -> Result<bool, ProtoError> {
        // The USE form is built by hand, so the name has to be checked here.
        // `Session::create` validates for itself; this path would otherwise let
        // a folder name close the quoted string and append a command.
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_control() || c == '"' || c == '\\')
        {
            return Err(ProtoError::Unsupported(
                "a folder name cannot be empty or contain quotes, backslashes or control characters",
            ));
        }

        if let Some(attribute) = special_use {
            if self.capability("CREATE-SPECIAL-USE").await? {
                self.session
                    .run_command_and_check_ok(&format!("CREATE \"{name}\" (USE ({attribute}))"))
                    .await?;
                return Ok(true);
            }
        }

        self.session.create(name).await?;
        Ok(false)
    }

    /// Which of the extensions this client can exploit the server actually has.
    ///
    /// Reported rather than assumed, because what a provider supports decides
    /// how sync and the mutation queue behave: no CONDSTORE means every folder
    /// is rescanned, and no MOVE means archiving is a copy-and-delete that
    /// needs UIDPLUS to finish cleanly.
    pub async fn extensions(&mut self) -> Result<Extensions, ProtoError> {
        Ok(Extensions {
            condstore: self.capability("CONDSTORE").await?,
            qresync: self.capability("QRESYNC").await?,
            r#move: self.capability("MOVE").await?,
            uidplus: self.capability("UIDPLUS").await?,
            idle: self.capability("IDLE").await?,
        })
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

    /// UIDs at or above `lo`, with each message's size, and nothing else.
    ///
    /// The cheap half of a fetch: a few bytes per message rather than the
    /// message. [`plan_batches`] uses it to decide how much to pull at a time,
    /// which is what stops a first sync of a real mailbox from loading every
    /// body into memory at once.
    ///
    /// `lo:*` has an IMAP quirk — `*` is the *highest existing UID*, so a `lo`
    /// past the end returns the last message rather than nothing. Filtering on
    /// `uid >= lo` here handles that in one place, for every caller.
    pub async fn uid_sizes(&mut self, lo: u32) -> Result<Vec<(u32, u32)>, ProtoError> {
        let mut stream = self
            .session
            .uid_fetch(format!("{lo}:*"), "(UID RFC822.SIZE)")
            .await?;

        let mut sizes = Vec::new();
        while let Some(fetch) = stream.try_next().await? {
            if let Some(uid) = fetch.uid {
                if uid >= lo {
                    sizes.push((uid, fetch.size.unwrap_or(0)));
                }
            }
        }
        sizes.sort_unstable();
        Ok(sizes)
    }

    /// Fetches the given messages in full, in one command.
    ///
    /// Callers are expected to have sized the batch with [`plan_batches`]; this
    /// will happily pull whatever it is given into memory.
    pub async fn fetch_uids(&mut self, uids: &[u32]) -> Result<Vec<RawMessage>, ProtoError> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        // BODY.PEEK[] rather than BODY[]: the latter sets \\Seen. EXAMINE already
        // prevents that, but the two together mean neither alone is load-bearing.
        let query = "(UID FLAGS RFC822.SIZE BODY.PEEK[])";
        let set = uids
            .iter()
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut stream = self.session.uid_fetch(set, query).await?;
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

/// Names a client might have given the folder behind each RFC 6154 attribute.
///
/// The fallbacks exist because plenty of servers — including Dovecot in its
/// default configuration — advertise no special-use attributes at all, and
/// guessing from a known list beats creating a second Sent folder alongside the
/// one the user's other clients already use. German names are here for the same
/// reason the classifier is bilingual.
pub const SENT_NAMES: &[&str] = &[
    "Sent",
    "Sent Items",
    "Sent Messages",
    "INBOX.Sent",
    "Gesendet",
    "Gesendete Objekte",
    "Gesendete Elemente",
];

pub const ARCHIVE_NAMES: &[&str] = &["Archive", "Archiv", "Archived"];

pub const TRASH_NAMES: &[&str] = &[
    "Trash",
    "Deleted Items",
    "Deleted Messages",
    "INBOX.Trash",
    "Papierkorb",
    "Gelöschte Objekte",
    "Gelöschte Elemente",
];

/// Picks the folder behind a special-use attribute.
///
/// Attributes are tried in the order given and win outright over names: a
/// server that puts `\\Sent` on a folder called something else is right and the
/// name list is wrong. Names are only the fallback, for servers — Dovecot in
/// its default configuration among them — that declare no special use at all.
///
/// A name matches either in full or as the last path segment, because real
/// servers nest these: Gmail has `[Gmail]/All Mail`, and a Dovecot configured
/// with a `.` delimiter has `INBOX.Archive`. The delimiter is not asked for;
/// the candidates are the three IMAP hierarchy delimiters in use, and since an
/// attribute match has already been tried this is a fallback on a fallback.
///
/// Takes `(name, special_use)` pairs so it works on both a `LIST` response and
/// the folders already recorded in the store.
pub fn find_special<'a>(
    folders: impl IntoIterator<Item = (&'a str, Option<&'a str>)> + Clone,
    attributes: &[&str],
    known_names: &[&str],
) -> Option<&'a str> {
    for attribute in attributes {
        if let Some((name, _)) = folders
            .clone()
            .into_iter()
            .find(|(_, special)| *special == Some(*attribute))
        {
            return Some(name);
        }
    }

    folders
        .into_iter()
        .find(|(name, _)| known_names.iter().any(|known| name_matches(name, known)))
        .map(|(name, _)| name)
}

/// Whether `name` is `known`, either outright or as its last path segment.
fn name_matches(name: &str, known: &str) -> bool {
    if name.eq_ignore_ascii_case(known) {
        return true;
    }
    name.rsplit(['/', '.', '\\'])
        .next()
        .is_some_and(|leaf| leaf.eq_ignore_ascii_case(known))
}

/// Picks the folder that archived mail belongs in.
pub fn find_archive<'a>(
    folders: impl IntoIterator<Item = (&'a str, Option<&'a str>)> + Clone,
) -> Option<&'a str> {
    find_special(folders.clone(), &["\\Archive"], ARCHIVE_NAMES).or_else(|| {
        // Gmail has no Archive folder at all. Archiving there means removing
        // the INBOX label, which over IMAP is a move into All Mail — the
        // folder it marks `\\All`. Tried last so a server with a real Archive
        // folder is never sent here instead.
        find_special(folders, &["\\All"], &[])
    })
}

/// Picks the folder that deleted mail belongs in.
pub fn find_trash<'a>(
    folders: impl IntoIterator<Item = (&'a str, Option<&'a str>)> + Clone,
) -> Option<&'a str> {
    find_special(folders, &["\\Trash"], TRASH_NAMES)
}

/// Picks the folder that sent mail belongs in.
pub fn find_sent(folders: &[RemoteFolder]) -> Option<&RemoteFolder> {
    let selectable: Vec<_> = folders.iter().filter(|f| f.selectable).collect();
    let pairs: Vec<(&str, Option<&str>)> = selectable
        .iter()
        .map(|f| (f.name.as_str(), f.special_use.as_deref()))
        .collect();

    let name = find_special(pairs, &["\\Sent"], SENT_NAMES)?.to_string();
    folders.iter().find(|f| f.selectable && f.name == name)
}

/// A connection, and whether opening it already consumed the server greeting.
///
/// STARTTLS has to read the greeting itself, before the IMAP client exists —
/// and the server does not send a second one after the upgrade, so the caller
/// must know not to wait for one.
struct Opened {
    transport: Transport,
    greeting_consumed: bool,
}

async fn open_transport(config: &ImapConfig) -> Result<Opened, ProtoError> {
    let tcp = TcpStream::connect((config.host.as_str(), config.port)).await?;
    // Mail is many small commands; Nagle adds latency for no benefit.
    tcp.set_nodelay(true).ok();

    match config.security {
        ImapSecurity::StartTls => Ok(Opened {
            transport: negotiate_starttls(tcp, &config.host).await?,
            greeting_consumed: true,
        }),
        ImapSecurity::Plaintext => Ok(Opened {
            transport: Transport::Plain(tcp),
            greeting_consumed: false,
        }),
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
            Ok(Opened {
                transport: Transport::Tls(Box::new(tls)),
                greeting_consumed: false,
            })
        }
    }
}

/// Upgrades a cleartext connection with RFC 2595 `STARTTLS`.
///
/// Hand-rolled because `async-imap` neither performs the upgrade nor hands back
/// its stream so we could. It is three lines of protocol, and doing it here
/// keeps the security properties in view:
///
/// * **No silent downgrade.** A server that refuses or cannot do `STARTTLS`
///   ends the connection with an error. Nothing continues in cleartext, so no
///   credential can be sent over one.
/// * **Nothing learned before the upgrade is kept.** Pre-TLS capabilities are
///   read and discarded rather than cached, so a stripped `CAPABILITY` cannot
///   influence anything after the handshake.
/// * **The certificate is verified** by the same platform verifier the implicit
///   TLS path uses.
async fn negotiate_starttls(mut tcp: TcpStream, host: &str) -> Result<Transport, ProtoError> {
    use tokio::io::AsyncWriteExt;

    let greeting = read_line(&mut tcp).await?;
    if !greeting.starts_with("* OK") {
        return Err(ProtoError::Login(format!(
            "server did not greet us: {}",
            greeting.trim()
        )));
    }

    const TAG: &str = "fmtls";
    tcp.write_all(format!("{TAG} STARTTLS\r\n").as_bytes())
        .await?;

    loop {
        let line = read_line(&mut tcp).await?;
        if line.is_empty() {
            return Err(ProtoError::NoGreeting);
        }
        if let Some(rest) = line.strip_prefix(TAG) {
            let rest = rest.trim();
            if rest.starts_with("OK") {
                break;
            }
            return Err(ProtoError::Tls(format!(
                "server refused STARTTLS ({rest}); refusing to continue in cleartext"
            )));
        }
        // An untagged line — a CAPABILITY listing, usually. Discarded on
        // purpose: see the note about stripping above.
    }

    let connector = tls_connector()?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| ProtoError::InvalidHostname(host.to_string()))?;
    let tls = connector.connect(server_name, tcp).await?;
    Ok(Transport::Tls(Box::new(tls)))
}

/// Reads one CRLF-terminated line, one byte at a time.
///
/// Deliberately unbuffered. A `BufReader` could read ahead past the `STARTTLS`
/// response, and anything it swallowed would be lost when the socket is handed
/// to the TLS connector. This is about thirty bytes of traffic, once per
/// connection.
async fn read_line(tcp: &mut TcpStream) -> Result<String, ProtoError> {
    use tokio::io::AsyncReadExt;

    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while tcp.read(&mut byte).await? > 0 {
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            break;
        }
        // A server that never sends CRLF must not be able to grow this without
        // bound.
        if line.len() > 8192 {
            return Err(ProtoError::Login("server sent an oversized line".into()));
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
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

    /// Folder layouts as the real providers actually present them over IMAP.
    /// The point of writing them out is that none of them can be reached from
    /// the dev server, and all three are what this code exists to handle.
    fn gmail() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("INBOX", None),
            ("[Gmail]/All Mail", Some("\\All")),
            ("[Gmail]/Sent Mail", Some("\\Sent")),
            ("[Gmail]/Trash", Some("\\Trash")),
            ("[Gmail]/Drafts", Some("\\Drafts")),
            ("[Gmail]/Spam", Some("\\Junk")),
        ]
    }

    fn microsoft365() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("INBOX", None),
            ("Archive", Some("\\Archive")),
            ("Sent Items", Some("\\Sent")),
            ("Deleted Items", Some("\\Trash")),
        ]
    }

    /// A Dovecot that declares nothing and nests under INBOX with a `.`
    /// delimiter — the case the name fallback exists for.
    fn bare_dovecot() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("INBOX", None),
            ("INBOX.Archive", None),
            ("INBOX.Sent", None),
            ("INBOX.Trash", None),
        ]
    }

    #[test]
    fn gmail_archives_into_all_mail() {
        // Gmail has no Archive folder: archiving is removing the INBOX label,
        // which over IMAP is a move into All Mail. Before this, `archive`
        // failed outright on the provider it matters most for.
        assert_eq!(find_archive(gmail()), Some("[Gmail]/All Mail"));
        assert_eq!(find_trash(gmail()), Some("[Gmail]/Trash"));
    }

    #[test]
    fn a_real_archive_folder_is_never_passed_over_for_all_mail() {
        // \All is the last resort, not a peer of \Archive.
        let mixed = vec![
            ("INBOX", None),
            ("Everything", Some("\\All")),
            ("Archive", Some("\\Archive")),
        ];
        assert_eq!(find_archive(mixed), Some("Archive"));
    }

    #[test]
    fn microsoft_folders_resolve_by_attribute() {
        assert_eq!(find_archive(microsoft365()), Some("Archive"));
        assert_eq!(find_trash(microsoft365()), Some("Deleted Items"));
    }

    #[test]
    fn nested_folders_match_on_their_last_segment() {
        // Declares nothing, so everything here is the name fallback.
        assert_eq!(find_archive(bare_dovecot()), Some("INBOX.Archive"));
        assert_eq!(find_trash(bare_dovecot()), Some("INBOX.Trash"));
    }

    #[test]
    fn a_server_with_nowhere_to_archive_says_so() {
        // The caller has to tell the user rather than invent a folder.
        let sparse = vec![("INBOX", None), ("Work", None)];
        assert_eq!(find_archive(sparse.clone()), None);
        assert_eq!(find_trash(sparse), None);
    }

    #[test]
    fn batches_respect_both_the_count_and_the_byte_cap() {
        // Small messages fill a batch by count.
        let many: Vec<(u32, u32)> = (1..=450).map(|uid| (uid, 1_000)).collect();
        let batches = plan_batches(&many);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), MAX_BATCH_MESSAGES);
        assert_eq!(batches[2].len(), 50);
        // Every UID appears once, in order.
        let flat: Vec<u32> = batches.concat();
        assert_eq!(flat, (1..=450).collect::<Vec<_>>());

        // Large ones fill it by size long before the count cap.
        let heavy: Vec<(u32, u32)> = (1..=10).map(|uid| (uid, 5 * 1024 * 1024)).collect();
        let batches = plan_batches(&heavy);
        assert!(batches.len() >= 4, "{batches:?}");
        for batch in &batches {
            assert!(batch.len() <= 3, "16 MB holds at most three 5 MB messages");
        }
    }

    #[test]
    fn a_message_larger_than_the_cap_gets_a_batch_to_itself() {
        // Refusing to fetch it would be worse than the memory spike, but it
        // must not drag its neighbours into the same batch.
        let sizes = vec![(1, 1_000), (2, (MAX_BATCH_BYTES + 1) as u32), (3, 1_000)];
        let batches = plan_batches(&sizes);
        assert_eq!(batches, vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn nothing_to_fetch_plans_nothing() {
        assert!(plan_batches(&[]).is_empty());
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
