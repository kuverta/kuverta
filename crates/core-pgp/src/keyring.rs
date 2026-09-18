//! The keyring: certificates and secret keys as files, passphrases in the
//! keychain, and the little bit of OpenPGP meaning rPGP leaves to us.
//!
//! ## Files
//!
//! `<dir>/<FINGERPRINT>.asc` is a certificate (a public key), and
//! `<dir>/<FINGERPRINT>.secret.asc` the matching secret key where we have
//! one. Armored text, so a person can look at them and a backup tool can diff
//! them. Nothing is cached: a keyring holds a handful of keys, parsing them is
//! well under a millisecond each, and reading the files every time means the
//! window and a send running on another thread can never disagree about what
//! is on disk.
//!
//! A secret key on disk is always passphrase-protected. One generated without
//! a passphrase, or imported without one, is given a random passphrase that
//! only the keychain knows — so the file is useless to whoever copies it off a
//! backup, and the person never has to type anything.
//!
//! ## Meaning
//!
//! rPGP verifies packets and signatures; which subkey may encrypt, when a key
//! expires and whether it was revoked is application policy. The rules here
//! are deliberately plain: the newest self-signature wins, a revocation from
//! the key itself revokes, and a key or subkey with no key flags at all is
//! taken to be able to do whatever its algorithm can — which is how keys made
//! before key flags existed have to be read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pgp::composed::{
    ArmorOptions, Deserializable, EncryptionCaps, KeyType, PublicOrSecret, SecretKeyParamsBuilder,
    SignedPublicKey, SignedPublicSubKey, SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{KeyFlags, Signature, SignatureType};
use pgp::types::{KeyDetails, Password};
use serde::{Deserialize, Serialize};

use crate::{hex, PgpError, Result};

// -- passphrases ---------------------------------------------------------------

/// Where secret-key passphrases are kept.
///
/// A trait so tests can use [`MemoryPassphrases`]: on macOS every keychain
/// read from a test binary raises a dialog, which would make `cargo test`
/// interactive. The app uses [`KeychainPassphrases`].
///
/// There is deliberately no way from here to the window: `get` is for this
/// crate's own signing and decryption, and a settings view learns only
/// whether a passphrase is stored ([`KeyView::has_passphrase`]).
pub trait Passphrases: Send + Sync {
    fn get(&self, fingerprint: &str) -> Result<Option<String>>;
    fn set(&self, fingerprint: &str, passphrase: &str) -> Result<()>;
    fn delete(&self, fingerprint: &str) -> Result<()>;
}

/// The OS keychain, under the instance's own service name — so a dev build
/// and the installed app never share or overwrite each other's entries.
pub struct KeychainPassphrases;

impl KeychainPassphrases {
    fn entry(fingerprint: &str) -> core_accounts::KeychainPassword {
        core_accounts::KeychainPassword::new(format!("pgp:{fingerprint}"))
    }
}

impl Passphrases for KeychainPassphrases {
    fn get(&self, fingerprint: &str) -> Result<Option<String>> {
        Self::entry(fingerprint)
            .peek()
            .map_err(|e| PgpError::Keychain(e.to_string()))
    }

    fn set(&self, fingerprint: &str, passphrase: &str) -> Result<()> {
        Self::entry(fingerprint)
            .store(passphrase)
            .map_err(|e| PgpError::Keychain(e.to_string()))
    }

    fn delete(&self, fingerprint: &str) -> Result<()> {
        // Absent is fine: deleting a key whose passphrase was never entered
        // must not fail on the part that was never there.
        let entry = Self::entry(fingerprint);
        if entry
            .peek()
            .map_err(|e| PgpError::Keychain(e.to_string()))?
            .is_some()
        {
            entry
                .delete()
                .map_err(|e| PgpError::Keychain(e.to_string()))?;
        }
        Ok(())
    }
}

/// Passphrases held in memory, for tests and for callers that must not touch
/// the keychain.
#[derive(Default)]
pub struct MemoryPassphrases(Mutex<HashMap<String, String>>);

impl Passphrases for MemoryPassphrases {
    fn get(&self, fingerprint: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| PgpError::Keychain("passphrase store poisoned".into()))?
            .get(fingerprint)
            .cloned())
    }

    fn set(&self, fingerprint: &str, passphrase: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| PgpError::Keychain("passphrase store poisoned".into()))?
            .insert(fingerprint.to_string(), passphrase.to_string());
        Ok(())
    }

    fn delete(&self, fingerprint: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| PgpError::Keychain("passphrase store poisoned".into()))?
            .remove(fingerprint);
        Ok(())
    }
}

// -- what crosses the boundary -------------------------------------------------

/// A key as a settings view lists it. No secret material, ever.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyView {
    /// Upper-case hex, no spaces.
    pub fingerprint: String,
    /// The last 16 hex digits, which is what most people have seen quoted.
    pub key_id: String,
    pub user_ids: Vec<String>,
    /// The addresses in the user IDs, lower-cased.
    pub emails: Vec<String>,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds; `None` when the key does not expire.
    pub expires: Option<i64>,
    pub expired: bool,
    pub revoked: bool,
    /// The primary key's algorithm, e.g. `"Ed25519"` or `"RSA"`.
    pub algorithm: String,
    /// Mail can be encrypted to this key: a valid subkey (or primary) for it.
    pub can_encrypt: bool,
    /// We hold a secret key that can sign.
    pub can_sign: bool,
    pub has_secret: bool,
    /// Whether the secret key can be used without asking — a passphrase is in
    /// the keychain. Whether, not what.
    pub has_passphrase: bool,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub imported: Vec<ImportedKey>,
    /// Blocks that could not be read or failed their self-signatures, one
    /// sentence each.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedKey {
    pub fingerprint: String,
    pub user_ids: Vec<String>,
    /// A secret key came with it.
    pub secret: bool,
    /// The key was already in the keyring and has been replaced.
    pub updated: bool,
    /// For a secret key: whether it can be used as it is. False for a
    /// protected key until its passphrase has been entered.
    pub has_passphrase: bool,
}

/// What compose needs to know to offer Sign and Encrypt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipientKeys {
    pub sender: String,
    /// The sender's secret key that would sign, when there is one.
    pub sender_key: Option<String>,
    /// A secret key for the sender that can sign, with its passphrase stored.
    pub can_sign: bool,
    /// Why signing is not possible, in a sentence, when it is not.
    pub sign_problem: Option<String>,
    /// Every recipient, and the sender, has a key mail can be encrypted to.
    pub can_encrypt: bool,
    pub recipients: Vec<RecipientKey>,
    /// Addresses without a usable key — the sender's own included, because
    /// the copy filed in Sent is encrypted to it.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipientKey {
    pub address: String,
    /// The certificate mail to this address would be encrypted to.
    pub fingerprint: Option<String>,
}

// -- the keyring ---------------------------------------------------------------

/// A certificate on file, with its secret key when we have one.
pub(crate) struct Cert {
    pub fingerprint: String,
    pub public: SignedPublicKey,
    pub secret: Option<SignedSecretKey>,
}

/// A key that can take a session key: the primary or one of its subkeys.
pub(crate) enum EncryptionTarget<'a> {
    Primary(&'a SignedPublicKey),
    Subkey(&'a SignedPublicSubKey),
}

impl EncryptionTarget<'_> {
    pub fn fingerprint(&self) -> String {
        match self {
            Self::Primary(key) => hex(key.fingerprint().as_bytes()),
            Self::Subkey(key) => hex(key.fingerprint().as_bytes()),
        }
    }
}

#[derive(Clone)]
pub struct Keyring {
    dir: PathBuf,
    passphrases: Arc<dyn Passphrases>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyring").field("dir", &self.dir).finish()
    }
}

impl Keyring {
    pub fn new(dir: impl Into<PathBuf>, passphrases: Arc<dyn Passphrases>) -> Self {
        Self {
            dir: dir.into(),
            passphrases,
        }
    }

    /// The keyring of a data directory, with passphrases in the keychain.
    pub fn in_data_dir(data_dir: impl AsRef<Path>) -> Self {
        Self::new(data_dir.as_ref().join("pgp"), Arc::new(KeychainPassphrases))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn public_path(&self, fingerprint: &str) -> PathBuf {
        self.dir.join(format!("{fingerprint}.asc"))
    }

    fn secret_path(&self, fingerprint: &str) -> PathBuf {
        self.dir.join(format!("{fingerprint}.secret.asc"))
    }

    /// Every certificate on file.
    ///
    /// A file that will not parse is logged and skipped rather than failing
    /// the lot: one damaged file must not make every other key unusable.
    pub(crate) fn certs(&self) -> Result<Vec<Cert>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };

        let mut certs = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(fingerprint) = name.strip_suffix(".asc") else {
                continue;
            };
            if fingerprint.ends_with(".secret") || !is_fingerprint(fingerprint) {
                continue;
            }

            let public = match read_public(&path) {
                Ok(public) => public,
                Err(err) => {
                    tracing::warn!(file = %path.display(), %err, "skipping an unreadable certificate");
                    continue;
                }
            };
            let secret_path = self.secret_path(fingerprint);
            let secret = if secret_path.exists() {
                match read_secret(&secret_path) {
                    Ok(secret) => Some(secret),
                    Err(err) => {
                        tracing::warn!(file = %secret_path.display(), %err, "skipping an unreadable secret key");
                        None
                    }
                }
            } else {
                None
            };
            certs.push(Cert {
                fingerprint: fingerprint.to_string(),
                public,
                secret,
            });
        }
        // Stable order: newest first, which is also the order a list shows.
        certs.sort_by_key(|cert| std::cmp::Reverse(cert.public.created_at().as_secs()));
        Ok(certs)
    }

    fn cert(&self, fingerprint: &str) -> Result<Cert> {
        let fingerprint = normalise_fingerprint(fingerprint)?;
        self.certs()?
            .into_iter()
            .find(|cert| cert.fingerprint == fingerprint)
            .ok_or(PgpError::UnknownKey(fingerprint))
    }

    /// Every key, newest first.
    pub fn keys(&self) -> Result<Vec<KeyView>> {
        let now = now();
        self.certs()?
            .iter()
            .map(|cert| self.view(cert, now))
            .collect()
    }

    fn view(&self, cert: &Cert, now: i64) -> Result<KeyView> {
        let public = &cert.public;
        let user_ids = user_ids(public);
        let emails = user_ids.iter().filter_map(|uid| email_of(uid)).collect();
        let created = public.created_at().as_secs() as i64;
        let expires = primary_expiry(public);
        let has_passphrase = match &cert.secret {
            Some(secret) => self.password_for(cert, secret).is_ok(),
            None => false,
        };
        Ok(KeyView {
            fingerprint: cert.fingerprint.clone(),
            key_id: cert.fingerprint[cert.fingerprint.len().saturating_sub(16)..].to_string(),
            user_ids,
            emails,
            created,
            expires,
            expired: expires.is_some_and(|at| at <= now),
            revoked: is_revoked(public),
            algorithm: algorithm_name(public.algorithm()),
            can_encrypt: !encryption_targets(public, now).is_empty(),
            can_sign: cert
                .secret
                .as_ref()
                .is_some_and(|secret| signing_key_usable(secret, now)),
            has_secret: cert.secret.is_some(),
            has_passphrase,
        })
    }

    /// The passphrase that unlocks `secret`: empty for an unprotected key,
    /// else the one in the keychain.
    pub(crate) fn password_for(&self, cert: &Cert, secret: &SignedSecretKey) -> Result<Password> {
        let protected = secret.primary_key.secret_params().is_encrypted()
            || secret
                .secret_subkeys
                .iter()
                .any(|sub| sub.key.secret_params().is_encrypted());
        if !protected {
            return Ok(Password::empty());
        }
        match self.passphrases.get(&cert.fingerprint)? {
            Some(passphrase) => Ok(Password::from(passphrase)),
            None => Err(PgpError::NoPassphrase(cert.fingerprint.clone())),
        }
    }

    /// Makes a new key pair for an address, and stores it.
    ///
    /// Ed25519 to sign and certify, Curve25519 to encrypt: GnuPG's own default
    /// since 2.3, small, fast, and read by every OpenPGP implementation still
    /// in use. v4 rather than v6 keys, because much of the software people
    /// actually have — GnuPG 2.2 included — cannot read v6 yet.
    ///
    /// `passphrase` protects the secret key on disk and goes in the keychain.
    /// Without one a random passphrase is made up: the key is still
    /// protected, and nobody has to remember anything.
    pub fn generate(&self, name: &str, email: &str, passphrase: Option<&str>) -> Result<KeyView> {
        let email = email.trim();
        let name = name.trim();
        if !email.contains('@') || email.chars().any(|c| c.is_whitespace() || "<>".contains(c)) {
            return Err(PgpError::Invalid(format!(
                "{email:?} is not an address a key can be made for"
            )));
        }
        if name.chars().any(|c| c.is_control() || "<>".contains(c)) {
            return Err(PgpError::Invalid(
                "a name on a key cannot contain angle brackets or line breaks".into(),
            ));
        }
        let user_id = if name.is_empty() {
            format!("<{email}>")
        } else {
            format!("{name} <{email}>")
        };
        let passphrase = match passphrase.map(str::to_string).filter(|p| !p.is_empty()) {
            Some(passphrase) => passphrase,
            None => random_passphrase(),
        };

        let mut rng = rand::thread_rng();
        let encryption = SubkeyParamsBuilder::default()
            .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
            .can_encrypt(EncryptionCaps::All)
            .passphrase(Some(passphrase.clone()))
            .build()
            .map_err(|e| PgpError::Pgp(e.to_string()))?;
        let params = SecretKeyParamsBuilder::default()
            .key_type(KeyType::Ed25519Legacy)
            .can_certify(true)
            .can_sign(true)
            .primary_user_id(user_id)
            .passphrase(Some(passphrase.clone()))
            .preferred_symmetric_algorithms(
                [SymmetricKeyAlgorithm::AES256, SymmetricKeyAlgorithm::AES128]
                    .into_iter()
                    .collect(),
            )
            .preferred_hash_algorithms(
                [HashAlgorithm::Sha512, HashAlgorithm::Sha256]
                    .into_iter()
                    .collect(),
            )
            .subkeys(vec![encryption])
            .build()
            .map_err(|e| PgpError::Pgp(e.to_string()))?;
        let secret = params.generate(&mut rng)?;

        let fingerprint = hex(secret.fingerprint().as_bytes());
        // The passphrase first: a key on disk nobody can unlock is worse than
        // no key, and the keychain is the step that can fail on a locked
        // login keychain.
        self.passphrases.set(&fingerprint, &passphrase)?;
        self.write(&fingerprint, &secret.to_public_key(), Some(&secret))?;

        let cert = self.cert(&fingerprint)?;
        self.view(&cert, now())
    }

    fn write(
        &self,
        fingerprint: &str,
        public: &SignedPublicKey,
        secret: Option<&SignedSecretKey>,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if let Some(secret) = secret {
            write_private(
                &self.secret_path(fingerprint),
                &secret.to_armored_string(ArmorOptions::default())?,
            )?;
        }
        std::fs::write(
            self.public_path(fingerprint),
            public.to_armored_string(ArmorOptions::default())?,
        )?;
        Ok(())
    }

    /// Imports every key in a piece of armored text.
    ///
    /// Several blocks are fine — people paste a whole exported keyring, or a
    /// public and a secret key together. Each key has to pass its own
    /// self-signature check before it is written: an unchecked certificate
    /// is a forgery waiting for someone to encrypt to it.
    pub fn import(&self, armored: &str) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let blocks = armored_blocks(armored);
        if blocks.is_empty() {
            report.errors.push(
                "no OpenPGP key block (-----BEGIN PGP PUBLIC KEY BLOCK-----) was found".into(),
            );
            return Ok(report);
        }

        let existing: Vec<String> = self.certs()?.into_iter().map(|c| c.fingerprint).collect();
        for block in blocks {
            if !(block.contains("PUBLIC KEY BLOCK") || block.contains("PRIVATE KEY BLOCK")) {
                report.errors.push(
                    "a block that is not a key (a message or a signature) was skipped".into(),
                );
                continue;
            }
            let keys = match PublicOrSecret::from_armor_many(block.as_bytes()) {
                Ok((keys, _)) => keys,
                Err(err) => {
                    report
                        .errors
                        .push(format!("a key block could not be read: {err}"));
                    continue;
                }
            };
            for key in keys {
                match key {
                    Ok(key) => match self.import_one(key, &existing) {
                        Ok(imported) => report.imported.push(imported),
                        Err(err) => report.errors.push(err.to_string()),
                    },
                    Err(err) => report
                        .errors
                        .push(format!("a key could not be read: {err}")),
                }
            }
        }
        Ok(report)
    }

    fn import_one(&self, key: PublicOrSecret, existing: &[String]) -> Result<ImportedKey> {
        key.verify_bindings().map_err(|err| {
            PgpError::Invalid(format!("a key failed its self-signature check: {err}"))
        })?;

        match key {
            PublicOrSecret::Public(public) => {
                let fingerprint = hex(public.fingerprint().as_bytes());
                // A certificate for a key we hold the secret of replaces the
                // public half only; the secret key stays as it was.
                self.write(&fingerprint, &public, None)?;
                Ok(ImportedKey {
                    user_ids: user_ids(&public),
                    updated: existing.contains(&fingerprint),
                    has_passphrase: false,
                    secret: false,
                    fingerprint,
                })
            }
            PublicOrSecret::Secret(mut secret) => {
                let fingerprint = hex(secret.fingerprint().as_bytes());
                let protected = secret.primary_key.secret_params().is_encrypted();
                if !protected {
                    // Never written in the clear: see the module note.
                    let passphrase = random_passphrase();
                    let password = Password::from(passphrase.as_str());
                    let mut rng = rand::thread_rng();
                    secret.primary_key.set_password(&mut rng, &password)?;
                    for sub in &mut secret.secret_subkeys {
                        if !sub.key.secret_params().is_encrypted() {
                            sub.key.set_password(&mut rng, &password)?;
                        }
                    }
                    self.passphrases.set(&fingerprint, &passphrase)?;
                }
                self.write(&fingerprint, &secret.to_public_key(), Some(&secret))?;
                Ok(ImportedKey {
                    user_ids: user_ids(&secret.to_public_key()),
                    updated: existing.contains(&fingerprint),
                    has_passphrase: !protected || self.passphrases.get(&fingerprint)?.is_some(),
                    secret: true,
                    fingerprint,
                })
            }
        }
    }

    /// A certificate, armored, to hand to someone.
    pub fn export_public(&self, fingerprint: &str) -> Result<String> {
        Ok(self
            .cert(fingerprint)?
            .public
            .to_armored_string(ArmorOptions::default())?)
    }

    /// Removes a key: certificate, secret key and stored passphrase.
    pub fn delete(&self, fingerprint: &str) -> Result<()> {
        let fingerprint = normalise_fingerprint(fingerprint)?;
        let public = self.public_path(&fingerprint);
        let secret = self.secret_path(&fingerprint);
        if !public.exists() && !secret.exists() {
            return Err(PgpError::UnknownKey(fingerprint));
        }
        // The secret key first: were the rest to fail, a certificate left
        // behind is harmless, a secret key left behind is not what was asked.
        if secret.exists() {
            std::fs::remove_file(&secret)?;
        }
        if public.exists() {
            std::fs::remove_file(&public)?;
        }
        self.passphrases.delete(&fingerprint)
    }

    /// Stores the passphrase for a secret key, after checking it opens it.
    ///
    /// Checked rather than trusted: a mistyped passphrase stored silently
    /// would surface weeks later as a message that "cannot be decrypted".
    pub fn set_passphrase(&self, fingerprint: &str, passphrase: &str) -> Result<()> {
        let cert = self.cert(fingerprint)?;
        let secret = cert.secret.as_ref().ok_or_else(|| {
            PgpError::Invalid(format!(
                "key {} has no secret key here, so it has no passphrase",
                cert.fingerprint
            ))
        })?;
        let password = Password::from(passphrase);
        secret
            .primary_key
            .unlock(&password, |_, _| Ok(()))
            .map_err(|_| PgpError::WrongPassphrase(cert.fingerprint.clone()))?
            .map_err(|_| PgpError::WrongPassphrase(cert.fingerprint.clone()))?;
        self.passphrases.set(&cert.fingerprint, passphrase)
    }

    /// The key mail to `email` would be encrypted to, if there is one.
    ///
    /// Of several, a key we hold the secret of wins (it is ours, and we know
    /// it is current), then the newest.
    pub fn key_for_email(&self, email: &str) -> Result<Option<KeyView>> {
        let now = now();
        let certs = self.certs()?;
        match pick_for_email(&certs, email, now) {
            Some(cert) => Ok(Some(self.view(cert, now)?)),
            None => Ok(None),
        }
    }

    /// Which recipients have keys, and whether the sender can sign.
    pub fn recipients(&self, sender: &str, recipients: &[String]) -> Result<RecipientKeys> {
        let now = now();
        let certs = self.certs()?;

        let mut seen = std::collections::HashSet::new();
        let mut listed = Vec::new();
        let mut missing = Vec::new();
        for address in recipients
            .iter()
            .map(|a| bare_address(a))
            .filter(|a| !a.is_empty())
        {
            if !seen.insert(address.to_lowercase()) {
                continue;
            }
            let fingerprint =
                pick_for_email(&certs, &address, now).map(|cert| cert.fingerprint.clone());
            if fingerprint.is_none() {
                missing.push(address.clone());
            }
            listed.push(RecipientKey {
                address,
                fingerprint,
            });
        }
        let sender = bare_address(sender);
        if pick_for_email(&certs, &sender, now).is_none() {
            missing.push(sender.clone());
        }

        let signer = pick_signer(&certs, &sender, now);
        let (sender_key, can_sign, sign_problem) = match signer {
            None => (
                None,
                false,
                Some(PgpError::NoSecretKey(sender.clone()).to_string()),
            ),
            Some((cert, secret)) => match self.password_for(cert, secret) {
                Ok(_) => (Some(cert.fingerprint.clone()), true, None),
                Err(err) => (Some(cert.fingerprint.clone()), false, Some(err.to_string())),
            },
        };

        Ok(RecipientKeys {
            can_encrypt: missing.is_empty() && !listed.is_empty(),
            sender,
            sender_key,
            can_sign,
            sign_problem,
            recipients: listed,
            missing,
        })
    }
}

// -- choosing keys ---------------------------------------------------------------

/// The certificate to encrypt to for an address.
pub(crate) fn pick_for_email<'a>(certs: &'a [Cert], email: &str, now: i64) -> Option<&'a Cert> {
    let email = bare_address(email).to_lowercase();
    certs
        .iter()
        .filter(|cert| has_email(&cert.public, &email))
        .filter(|cert| !encryption_targets(&cert.public, now).is_empty())
        // `certs` is newest first, so the first of each kind is the newest.
        .min_by_key(|cert| cert.secret.is_none())
}

/// The secret key to sign as an address with.
pub(crate) fn pick_signer<'a>(
    certs: &'a [Cert],
    email: &str,
    now: i64,
) -> Option<(&'a Cert, &'a SignedSecretKey)> {
    let email = bare_address(email).to_lowercase();
    certs.iter().find_map(|cert| {
        let secret = cert.secret.as_ref()?;
        (has_email(&cert.public, &email) && signing_key_usable(secret, now))
            .then_some((cert, secret))
    })
}

fn has_email(public: &SignedPublicKey, email: &str) -> bool {
    user_ids(public)
        .iter()
        .filter_map(|uid| email_of(uid))
        .any(|candidate| candidate == email)
}

/// The keys mail may be encrypted to, newest first.
///
/// One per certificate is what gets used — the newest — which is what GnuPG
/// does, and what a recipient who rotated their encryption subkey expects.
pub(crate) fn encryption_targets(public: &SignedPublicKey, now: i64) -> Vec<EncryptionTarget<'_>> {
    if is_revoked(public) || primary_expiry(public).is_some_and(|at| at <= now) {
        return Vec::new();
    }
    let mut subkeys: Vec<&SignedPublicSubKey> = public
        .public_subkeys
        .iter()
        .filter(|sub| {
            let binding = latest_binding(&sub.signatures);
            let flags = binding.map(Signature::key_flags);
            let allowed = match flags {
                Some(flags) if has_any_flag(&flags) => {
                    flags.encrypt_comms() || flags.encrypt_storage()
                }
                _ => sub.algorithm().can_encrypt(),
            };
            let expired = binding
                .and_then(|sig| expiry(sub.created_at().as_secs(), sig))
                .is_some_and(|at| at <= now);
            allowed && !expired && !subkey_revoked(&sub.signatures) && sub.algorithm().can_encrypt()
        })
        .collect();
    subkeys.sort_by_key(|sub| std::cmp::Reverse(sub.created_at().as_secs()));

    let mut targets: Vec<EncryptionTarget<'_>> =
        subkeys.into_iter().map(EncryptionTarget::Subkey).collect();
    if targets.is_empty() {
        let flags = primary_self_signature(public).map(Signature::key_flags);
        let allowed = match flags {
            Some(flags) if has_any_flag(&flags) => flags.encrypt_comms() || flags.encrypt_storage(),
            _ => true,
        };
        if allowed && public.algorithm().can_encrypt() {
            targets.push(EncryptionTarget::Primary(public));
        }
    }
    targets
}

/// Whether a secret key has a component that may sign.
pub(crate) fn signing_key_usable(secret: &SignedSecretKey, now: i64) -> bool {
    signing_component(secret, now).is_some()
}

/// The component that signs: a signing subkey where there is one, else the
/// primary if it is allowed to.
pub(crate) enum SigningComponent<'a> {
    Primary(&'a pgp::packet::SecretKey),
    Subkey(&'a pgp::packet::SecretSubkey),
}

pub(crate) fn signing_component(
    secret: &SignedSecretKey,
    now: i64,
) -> Option<SigningComponent<'_>> {
    let public = secret.to_public_key();
    if is_revoked(&public) || primary_expiry(&public).is_some_and(|at| at <= now) {
        return None;
    }
    let subkey = secret
        .secret_subkeys
        .iter()
        .filter(|sub| {
            let binding = latest_binding(&sub.signatures);
            let allowed = binding.is_some_and(|sig| sig.key_flags().sign());
            let expired = binding
                .and_then(|sig| expiry(sub.key.created_at().as_secs(), sig))
                .is_some_and(|at| at <= now);
            allowed
                && !expired
                && !subkey_revoked(&sub.signatures)
                && sub.key.algorithm().can_sign()
        })
        .max_by_key(|sub| sub.key.created_at().as_secs());
    if let Some(sub) = subkey {
        return Some(SigningComponent::Subkey(&sub.key));
    }

    let flags = primary_self_signature(&public).map(Signature::key_flags);
    let allowed = match flags {
        Some(flags) if has_any_flag(&flags) => flags.sign(),
        _ => true,
    };
    (allowed && secret.primary_key.algorithm().can_sign())
        .then_some(SigningComponent::Primary(&secret.primary_key))
}

// -- reading a certificate ---------------------------------------------------------

pub(crate) fn user_ids(public: &SignedPublicKey) -> Vec<String> {
    public
        .details
        .users
        .iter()
        .map(|user| String::from_utf8_lossy(user.id.id()).into_owned())
        .collect()
}

/// The primary user ID as a person would read it.
pub(crate) fn primary_user_id(public: &SignedPublicKey) -> Option<String> {
    let users = &public.details.users;
    users
        .iter()
        .find(|user| user.is_primary())
        .or_else(|| users.first())
        .map(|user| String::from_utf8_lossy(user.id.id()).into_owned())
}

/// The address in `Name <address>`, or a bare address, lower-cased.
pub(crate) fn email_of(user_id: &str) -> Option<String> {
    let candidate = match (user_id.rfind('<'), user_id.rfind('>')) {
        (Some(open), Some(close)) if open < close => &user_id[open + 1..close],
        _ => user_id,
    };
    let candidate = candidate.trim();
    (candidate.contains('@') && !candidate.contains(char::is_whitespace))
        .then(|| candidate.to_lowercase())
}

/// An address as typed into a To: field, down to `local@domain`.
pub(crate) fn bare_address(input: &str) -> String {
    email_of(input).unwrap_or_else(|| input.trim().to_lowercase())
}

fn is_self_issued(public: &SignedPublicKey, sig: &Signature) -> bool {
    let fingerprint = public.fingerprint();
    let key_id = public.legacy_key_id();
    let fingerprints = sig.issuer_fingerprint();
    let ids = sig.issuer_key_id();
    // A self-signature with no issuer at all is malformed but seen in old
    // keys; `verify_bindings` has already checked it against this key.
    (fingerprints.is_empty() && ids.is_empty())
        || fingerprints.iter().any(|fp| **fp == fingerprint)
        || ids.iter().any(|id| **id == key_id)
}

/// The newest self-signature on the primary user ID, or on the key itself.
fn primary_self_signature(public: &SignedPublicKey) -> Option<&Signature> {
    let users = &public.details.users;
    let primary = users
        .iter()
        .find(|user| user.is_primary())
        .or_else(|| users.first());
    primary
        .into_iter()
        .flat_map(|user| user.signatures.iter())
        .chain(public.details.direct_signatures.iter())
        .filter(|sig| is_self_issued(public, sig))
        .max_by_key(|sig| sig.created().map(|t| t.as_secs()).unwrap_or(0))
}

fn primary_expiry(public: &SignedPublicKey) -> Option<i64> {
    primary_self_signature(public).and_then(|sig| expiry(public.created_at().as_secs(), sig))
}

fn expiry(created: u32, sig: &Signature) -> Option<i64> {
    sig.key_expiration_time()
        .map(|d| d.as_secs())
        .filter(|secs| *secs > 0)
        .map(|secs| created as i64 + secs as i64)
}

fn is_revoked(public: &SignedPublicKey) -> bool {
    public
        .details
        .revocation_signatures
        .iter()
        .any(|sig| sig.typ() == Some(SignatureType::KeyRevocation))
}

fn latest_binding(signatures: &[Signature]) -> Option<&Signature> {
    signatures
        .iter()
        .filter(|sig| sig.typ() == Some(SignatureType::SubkeyBinding))
        .max_by_key(|sig| sig.created().map(|t| t.as_secs()).unwrap_or(0))
}

fn subkey_revoked(signatures: &[Signature]) -> bool {
    signatures
        .iter()
        .any(|sig| sig.typ() == Some(SignatureType::SubkeyRevocation))
}

fn has_any_flag(flags: &KeyFlags) -> bool {
    flags.certify()
        || flags.sign()
        || flags.encrypt_comms()
        || flags.encrypt_storage()
        || flags.authentication()
}

fn algorithm_name(algorithm: PublicKeyAlgorithm) -> String {
    match algorithm {
        PublicKeyAlgorithm::EdDSALegacy | PublicKeyAlgorithm::Ed25519 => "Ed25519".into(),
        PublicKeyAlgorithm::Ed448 => "Ed448".into(),
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSAEncrypt | PublicKeyAlgorithm::RSASign => {
            "RSA".into()
        }
        PublicKeyAlgorithm::ECDSA => "ECDSA".into(),
        PublicKeyAlgorithm::DSA => "DSA".into(),
        other => format!("{other:?}"),
    }
}

// -- files -------------------------------------------------------------------------

fn read_public(path: &Path) -> Result<SignedPublicKey> {
    let text = std::fs::read_to_string(path)?;
    let (key, _) = SignedPublicKey::from_string(&text)?;
    Ok(key)
}

fn read_secret(path: &Path) -> Result<SignedSecretKey> {
    let text = std::fs::read_to_string(path)?;
    let (key, _) = SignedSecretKey::from_string(&text)?;
    Ok(key)
}

/// Writes a file only its owner can read, where the system has such a thing.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)?;
        Ok(())
    }
}

/// Every `-----BEGIN PGP …-----` … `-----END PGP …-----` block in `text`.
pub(crate) fn armored_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN PGP ") {
        let after = &rest[start..];
        let Some(end_marker) = after.find("-----END PGP ") else {
            break;
        };
        // The END line runs to its own closing dashes.
        let tail = &after[end_marker + "-----END PGP ".len()..];
        let Some(close) = tail.find("-----") else {
            break;
        };
        let end = end_marker + "-----END PGP ".len() + close + "-----".len();
        blocks.push(&after[..end]);
        rest = &after[end..];
    }
    blocks
}

/// Fingerprints arrive from a window and become file names, so they are
/// checked, not trusted: hex only, of a length a fingerprint has.
fn normalise_fingerprint(input: &str) -> Result<String> {
    let fingerprint: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    if is_fingerprint(&fingerprint) {
        Ok(fingerprint)
    } else {
        Err(PgpError::Invalid(format!(
            "{input:?} is not a key fingerprint"
        )))
    }
}

fn is_fingerprint(candidate: &str) -> bool {
    (32..=64).contains(&candidate.len()) && candidate.chars().all(|c| c.is_ascii_hexdigit())
}

fn random_passphrase() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_come_out_of_user_ids() {
        assert_eq!(
            email_of("Erika Mustermann <Erika@Example.com>").as_deref(),
            Some("erika@example.com")
        );
        assert_eq!(
            email_of("erika@example.com").as_deref(),
            Some("erika@example.com")
        );
        assert_eq!(email_of("Erika Mustermann"), None);
    }

    #[test]
    fn a_fingerprint_from_outside_cannot_name_another_file() {
        assert!(normalise_fingerprint("../../etc/passwd").is_err());
        assert!(normalise_fingerprint("ABCD").is_err());
        assert_eq!(
            normalise_fingerprint("0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567").unwrap(),
            "0123456789ABCDEF0123456789ABCDEF01234567"
        );
    }

    #[test]
    fn armored_blocks_are_found_in_surrounding_text() {
        let text = "hello\n-----BEGIN PGP PUBLIC KEY BLOCK-----\nabc\n-----END PGP PUBLIC KEY BLOCK-----\nbye\n-----BEGIN PGP PRIVATE KEY BLOCK-----\nx\n-----END PGP PRIVATE KEY BLOCK-----";
        let blocks = armored_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].ends_with("-----END PGP PUBLIC KEY BLOCK-----"));
        assert!(blocks[1].starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"));
    }
}
