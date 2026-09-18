//! OpenPGP keys, as a settings view and a compose window use them.
//!
//! Thin over [`core_pgp::Keyring`], which keeps certificates as files in the
//! data directory and passphrases in the keychain — not in the store, so none
//! of this touches the schema. What crosses this boundary follows the
//! passwords rule: a passphrase goes in and is never handed back; a view says
//! whether one is stored, not what it is. Secret keys are never exported.
//!
//! Sending signed or encrypted mail is not here: it is two flags on
//! [`crate::DraftInput`], honoured by [`crate::Session::send`], so that
//! anything that sends — the window, the CLI, a scheduled send — protects a
//! message the same way.

use core_pgp::{ImportReport, KeyView, Keyring, RecipientKeys};
use core_store::model::{AccountId, MessageId};

use crate::{Core, Result, RpcError};

impl Core {
    fn keyring(&self) -> Result<&Keyring> {
        self.keyring.as_ref().ok_or_else(|| {
            RpcError::Rejected(
                "this store was opened without its data directory, so it has no keyring".into(),
            )
        })
    }

    /// Every key on file, newest first.
    pub fn pgp_keys(&self) -> Result<Vec<KeyView>> {
        Ok(self.keyring()?.keys()?)
    }

    /// Makes a key pair for an address. Without a passphrase one is made up
    /// and kept in the keychain, so the key is protected on disk regardless.
    pub fn pgp_generate(
        &self,
        name: &str,
        email: &str,
        passphrase: Option<&str>,
    ) -> Result<KeyView> {
        Ok(self.keyring()?.generate(name, email, passphrase)?)
    }

    /// Imports every key in pasted or loaded armored text.
    pub fn pgp_import(&self, armored: &str) -> Result<ImportReport> {
        Ok(self.keyring()?.import(armored)?)
    }

    /// A certificate, armored, to hand to someone. Public half only.
    pub fn pgp_export_public(&self, fingerprint: &str) -> Result<String> {
        Ok(self.keyring()?.export_public(fingerprint)?)
    }

    /// Removes a key, its secret key and its stored passphrase.
    pub fn pgp_delete(&self, fingerprint: &str) -> Result<()> {
        Ok(self.keyring()?.delete(fingerprint)?)
    }

    /// Stores the passphrase of a secret key, once it has been shown to
    /// unlock it. One way: nothing reads it back out.
    pub fn pgp_set_passphrase(&self, fingerprint: &str, passphrase: &str) -> Result<()> {
        Ok(self.keyring()?.set_passphrase(fingerprint, passphrase)?)
    }

    /// Which recipients have keys and whether the sender can sign — what a
    /// compose window needs to offer or grey out Sign and Encrypt.
    pub fn pgp_recipients(&self, sender: &str, recipients: &[String]) -> Result<RecipientKeys> {
        Ok(self.keyring()?.recipients(sender, recipients)?)
    }

    /// Imports the public keys a stored message carries, including ones
    /// inside its encryption.
    pub fn pgp_import_from_message(
        &self,
        account: AccountId,
        id: MessageId,
    ) -> Result<ImportReport> {
        let keyring = self.keyring()?;
        let stored = self
            .store
            .message_by_id(account, id)?
            .ok_or(RpcError::UnknownMessage(id))?;
        let path = stored.body_path.as_deref().ok_or_else(|| {
            RpcError::Rejected(format!("message {id} was stored without its body"))
        })?;
        let raw = self
            .blobs
            .get(path)
            .map_err(|e| RpcError::Rejected(format!("reading the stored body of {id}: {e}")))?;

        let opened = core_pgp::open_message(&raw, Some(keyring));
        if opened.pgp_keys.is_empty() {
            return Err(RpcError::Rejected(
                "this message has no OpenPGP key attached".into(),
            ));
        }
        let mut report = ImportReport::default();
        for armored in &opened.pgp_keys {
            let one = keyring.import(armored)?;
            report.imported.extend(one.imported);
            report.errors.extend(one.errors);
        }
        Ok(report)
    }
}
