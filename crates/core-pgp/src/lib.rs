//! OpenPGP: a keyring, PGP/MIME for outgoing mail, and reading what arrives.
//!
//! Built on rPGP, which is pure Rust like everything else here — no GnuPG to
//! find on the machine, no agent to talk to, and no C toolchain in the build.
//! rPGP is deliberately low level: it does packets, signatures and ciphers,
//! and leaves the *meaning* of a certificate — which subkey is for what, when
//! a key has expired, whether it was revoked — to the application. That layer
//! lives in [`keyring`], kept small and conservative.
//!
//! ## Where keys live
//!
//! Files under `<data_dir>/pgp/`, one armored certificate per key and, for
//! keys of our own, the passphrase-protected secret key beside it. Not the
//! store: a keyring is something a person copies, backs up and inspects, and
//! keeping it out of SQLite means this never needs a schema migration. The
//! passphrase goes to the OS keychain, one way — like account passwords it is
//! written and never handed back to a window. See [`keyring::Passphrases`].
//!
//! ## Outgoing
//!
//! [`protect`] takes a message `core-smtp` has already built and wraps it. The
//! envelope — who actually receives the message, Bcc included — is decided
//! there and nowhere else; this only changes the bytes.
//!
//! ## Bcc and encryption do not mix
//!
//! An encrypted message names the key of every recipient it is encrypted to,
//! in the clear. Encrypting one message to a blind recipient's key would tell
//! everyone else who was blind-copied, which is the one disclosure the Bcc
//! handling in `core-smtp` exists to prevent. So an encrypted message with Bcc
//! recipients is refused rather than quietly weakened.
//!
//! ## Incoming
//!
//! [`open_message`] recognises PGP/MIME encrypted and signed mail and inline
//! PGP, decrypts with a key whose passphrase is stored, verifies against the
//! certificates on file, and reports what it found as a [`SecurityView`].

pub mod incoming;
pub mod keyring;
pub mod outgoing;

pub use incoming::{
    open_message, open_parsed, OpenedMessage, SecurityView, SignatureState, SignatureView,
};
pub use keyring::{
    ImportReport, ImportedKey, KeyView, KeychainPassphrases, Keyring, MemoryPassphrases,
    Passphrases, RecipientKey, RecipientKeys,
};
pub use outgoing::{protect, Protection};

#[derive(Debug, thiserror::Error)]
pub enum PgpError {
    #[error(
        "no usable OpenPGP key for {}; import their public key, or send without encryption",
        .0.join(", ")
    )]
    NoKeyFor(Vec<String>),

    #[error(
        "an encrypted message cannot have Bcc recipients: every recipient's key is named in the \
         encrypted message, so the others could see who was blind-copied. Send it to them separately."
    )]
    BccWithEncryption,

    #[error("there is no secret key for {0} to sign with; generate or import one first")]
    NoSecretKey(String),

    #[error(
        "there is no key of your own for {0}, so the copy kept in Sent could not be read \
         again; generate or import one first"
    )]
    NoOwnKey(String),

    #[error("no passphrase is stored for key {0}; enter it in settings")]
    NoPassphrase(String),

    #[error("the passphrase does not unlock key {0}")]
    WrongPassphrase(String),

    #[error("no key {0} in the keyring")]
    UnknownKey(String),

    #[error("{0}")]
    Invalid(String),

    #[error("keychain: {0}")]
    Keychain(String),

    #[error("OpenPGP: {0}")]
    Pgp(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<pgp::errors::Error> for PgpError {
    fn from(err: pgp::errors::Error) -> Self {
        PgpError::Pgp(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, PgpError>;

/// Upper-case hex, which is how fingerprints are shown everywhere else.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02X}");
            out
        })
}
