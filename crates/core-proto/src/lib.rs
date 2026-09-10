//! IMAP transport, parsing, and synchronisation into the local store.
//!
//! Scope is deliberately read-only: see `client::ImapClient`.

pub mod client;
pub mod parse;
mod stream;
pub mod sync;

pub use client::{FolderState, ImapClient, ImapConfig, RawMessage, RemoteFolder};
pub use sync::{sync_account, SyncReport};

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("network: {0}")]
    Io(#[from] std::io::Error),

    #[error("imap: {0}")]
    Imap(#[from] async_imap::error::Error),

    #[error("login rejected: {0}")]
    Login(String),

    #[error("server closed the connection before sending a greeting")]
    NoGreeting,

    #[error("tls: {0}")]
    Tls(String),

    #[error("{0:?} is not a valid hostname for certificate validation")]
    InvalidHostname(String),

    #[error("{0}")]
    Unsupported(&'static str),

    #[error(transparent)]
    Auth(#[from] core_accounts::AuthError),

    #[error(transparent)]
    Store(#[from] core_store::StoreError),
}
