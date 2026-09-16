//! Account configuration.
//!
//! Everything the CLI's `add-account`, `set-password`, `set-smtp` and
//! `exclude` do, as one surface a settings window can drive.
//!
//! ## Passwords go one way
//!
//! [`set_password`] writes to the keychain. Nothing here reads one back:
//! [`AccountSettings`] carries `has_password`, a boolean, and never the
//! secret. A settings form has no business round-tripping a credential
//! through a webview in order to redisplay it, and a form that cannot show a
//! password cannot leak one into a screenshot, a log, or a crash report.
//!
//! ## Cleartext stays a loopback affordance
//!
//! The CLI refuses to configure plaintext for a remote host. So does this: the
//! check belongs to the setting, not to the front end that happens to be
//! collecting it.

use core_accounts::KeychainPassword;
use core_store::model::{Account, AccountId, ImapSecurity, NewAccount, SmtpConfig, SmtpSecurity};
use serde::{Deserialize, Serialize};

use crate::{Core, Result, RpcError};

/// An account as a settings form shows it.
///
/// No password, ever — see the module note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSettings {
    pub id: AccountId,
    pub label: String,
    pub email: String,
    pub username: String,

    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: String,

    /// `None` when the account cannot send.
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,

    /// `"app_password"` or `"oauth2"`.
    pub auth_method: String,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_tenant: Option<String>,

    /// Whether a credential is stored, not what it is.
    pub has_password: bool,
    /// Folder names or `\`-prefixed attributes this account does not sync.
    pub excluded_folders: Vec<String>,
}

/// What a settings form sends back.
///
/// `id` is `None` for a new account. Everything else is the form's own state,
/// so saving is one call rather than a sequence a half-filled form could stop
/// in the middle of.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountInput {
    pub id: Option<AccountId>,
    pub label: String,
    pub email: String,
    /// Defaults to the email address, which is right for nearly every provider.
    pub username: Option<String>,

    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: String,

    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,

    pub auth_method: String,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_tenant: Option<String>,

    pub excluded_folders: Vec<String>,
}

fn reject(message: impl Into<String>) -> RpcError {
    RpcError::Rejected(message.into())
}

/// Hosts a cleartext connection may be made to.
///
/// The dev server and nothing else. A password sent in the clear to anywhere
/// else is a password disclosed, whichever front end collected it.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn parse_imap_security(value: &str, host: &str) -> Result<ImapSecurity> {
    let security = ImapSecurity::parse(value)
        .ok_or_else(|| reject(format!("{value:?} is not a transport security")))?;
    if security == ImapSecurity::Plaintext && !is_loopback(host) {
        return Err(reject(format!(
            "refusing plaintext IMAP for {host}; cleartext is only allowed against localhost"
        )));
    }
    Ok(security)
}

fn parse_smtp_security(value: &str, host: &str) -> Result<SmtpSecurity> {
    let security = SmtpSecurity::parse(value)
        .ok_or_else(|| reject(format!("{value:?} is not a transport security")))?;
    if security == SmtpSecurity::Plaintext && !is_loopback(host) {
        return Err(reject(format!(
            "refusing plaintext SMTP for {host}; cleartext is only allowed against localhost"
        )));
    }
    Ok(security)
}

/// Where an app password is kept for an account.
pub fn password_entry(email: &str) -> KeychainPassword {
    KeychainPassword::new(email)
}

impl Core {
    pub fn account_settings(&self, id: AccountId) -> Result<AccountSettings> {
        let account = self
            .store()
            .accounts()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| RpcError::UnknownAccount(id.to_string()))?;
        self.settings_of(&account)
    }

    pub fn all_account_settings(&self) -> Result<Vec<AccountSettings>> {
        self.store()
            .accounts()?
            .iter()
            .map(|account| self.settings_of(account))
            .collect()
    }

    fn settings_of(&self, account: &Account) -> Result<AccountSettings> {
        Ok(AccountSettings {
            id: account.id,
            label: account.label.clone(),
            email: account.email.clone(),
            username: account.username.clone(),
            imap_host: account.imap_host.clone(),
            imap_port: account.imap_port,
            imap_security: account.imap_security.as_str().to_string(),
            smtp_host: account.smtp.as_ref().map(|s| s.host.clone()),
            smtp_port: account.smtp.as_ref().map(|s| s.port),
            smtp_security: account
                .smtp
                .as_ref()
                .map(|s| s.security.as_str().to_string()),
            auth_method: account.auth_method.clone(),
            oauth_provider: account.oauth_provider.clone(),
            oauth_client_id: account.oauth_client_id.clone(),
            oauth_tenant: account.oauth_tenant.clone(),
            // Whether one is stored, never which. A keychain that will not
            // answer is reported as "no password" rather than failing the
            // whole form — the user can always set one again.
            has_password: password_entry(&account.email)
                .peek()
                .ok()
                .flatten()
                .is_some(),
            excluded_folders: self.store().folder_exclusions(account.id)?,
        })
    }

    /// Creates or updates an account, and returns its id.
    pub fn save_account(&self, input: &AccountInput) -> Result<AccountId> {
        if input.email.trim().is_empty() {
            return Err(reject("an account needs an email address"));
        }
        if input.imap_host.trim().is_empty() {
            return Err(reject("an account needs an IMAP host"));
        }

        let imap_security = parse_imap_security(&input.imap_security, &input.imap_host)?;

        // The three SMTP fields are one setting: a host without a port is not
        // half a configuration, it is a mistake worth naming.
        let smtp = match input.smtp_host.as_deref().filter(|h| !h.trim().is_empty()) {
            Some(host) => {
                let port = input
                    .smtp_port
                    .ok_or_else(|| reject("an SMTP host needs a port"))?;
                let security =
                    parse_smtp_security(input.smtp_security.as_deref().unwrap_or("tls"), host)?;
                Some(SmtpConfig {
                    host: host.to_string(),
                    port,
                    security,
                })
            }
            None => None,
        };

        if input.auth_method == "oauth2" && input.oauth_client_id.is_none() {
            return Err(reject(
                "an OAuth2 account needs a client id from its provider's app registration",
            ));
        }

        let record = NewAccount {
            label: if input.label.trim().is_empty() {
                input.email.clone()
            } else {
                input.label.clone()
            },
            email: input.email.trim().to_string(),
            imap_host: input.imap_host.trim().to_string(),
            imap_port: input.imap_port,
            imap_security,
            username: input
                .username
                .clone()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| input.email.trim().to_string()),
            auth_method: input.auth_method.clone(),
            oauth_client_id: input.oauth_client_id.clone(),
            oauth_tenant: input.oauth_tenant.clone(),
            smtp: smtp.clone(),
            oauth_provider: input.oauth_provider.clone(),
        };

        let id = match input.id {
            Some(id) => {
                self.store().update_account(id, &record)?;
                // `update_account` deliberately leaves the endpoint alone, so
                // that clearing it is an explicit act rather than a side
                // effect of saving a form that never showed it.
                self.store().set_smtp(id, smtp.as_ref())?;
                id
            }
            None => {
                if self.store().account_by_email(&record.email)?.is_some() {
                    return Err(reject(format!("{} is already registered", record.email)));
                }
                self.store().add_account(&record)?
            }
        };

        self.set_exclusions(id, &input.excluded_folders)?;
        Ok(id)
    }

    /// Forgets an account, its mail, and its stored credential.
    pub fn delete_account(&self, id: AccountId) -> Result<()> {
        let settings = self.account_settings(id)?;
        self.store().delete_account(id)?;

        // After the row, so a failure to reach the keychain cannot leave an
        // account that exists but can no longer log in. An orphaned keychain
        // entry is untidy; a half-deleted account is worse.
        if let Err(err) = password_entry(&settings.email).delete() {
            tracing::warn!(%err, email = %settings.email, "could not remove the stored password");
        }
        Ok(())
    }

    /// Stores an app password in the OS keychain.
    pub fn set_password(&self, email: &str, password: &str) -> Result<()> {
        if password.is_empty() {
            return Err(reject("an empty password is not a password"));
        }
        password_entry(email)
            .store(password)
            .map_err(|e| RpcError::Auth(e.to_string()))
    }

    pub fn clear_password(&self, email: &str) -> Result<()> {
        password_entry(email)
            .delete()
            .map_err(|e| RpcError::Auth(e.to_string()))
    }

    /// Replaces the set of folders this account does not sync.
    pub fn set_exclusions(&self, id: AccountId, patterns: &[String]) -> Result<()> {
        let wanted: Vec<String> = patterns
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        for existing in self.store().folder_exclusions(id)? {
            if !wanted.contains(&existing) {
                self.store().include_folder(id, &existing)?;
            }
        }
        for pattern in &wanted {
            self.store().exclude_folder(id, pattern)?;
        }
        Ok(())
    }
}
