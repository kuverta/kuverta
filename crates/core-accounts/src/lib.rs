//! Credential acquisition, behind a trait.
//!
//! Auth is pluggable from the first commit on purpose. Day-one accounts use
//! app-specific passwords, but Microsoft 365 has disabled basic auth for IMAP
//! and needs OAuth2, so a concrete `String` password threaded through the sync
//! code would have to be torn out within weeks. See
//! docs/implementation-plan.md section 2.

use std::fmt;

pub mod oauth;

pub use oauth::{DeviceCodePrompt, OAuth2Config, OAuth2Device, TokenStore};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("keychain: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("no credential stored for {account} (service {service})")]
    NotFound { service: String, account: String },

    #[error("environment variable {0} is not set")]
    MissingEnv(String),

    #[error("{0} is not implemented yet")]
    Unimplemented(&'static str),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("no stored login for {0}; run `fuckmail login --email {0}`")]
    NotLoggedIn(String),

    #[error("the stored login for {user} is no longer valid ({reason}); run `fuckmail login --email {user}`")]
    LoginExpired { user: String, reason: String },

    #[error("{provider} rejected the request: {error}{}", .description.as_deref().map(|d| format!(" — {d}")).unwrap_or_default())]
    Provider {
        provider: &'static str,
        error: String,
        description: Option<String>,
    },

    #[error("authorization timed out; the code was not entered in time")]
    DeviceCodeExpired,
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// What gets handed to the IMAP layer to authenticate with.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// LOGIN / AUTHENTICATE PLAIN with a password or app-specific password.
    Password(String),
    /// SASL XOAUTH2 bearer token, for Microsoft 365 and Gmail.
    OAuthBearer { user: String, access_token: String },
}

// Hand-written so a credential can never be logged by accident.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Credential::Password(<redacted>)"),
            Self::OAuthBearer { user, .. } => {
                write!(
                    f,
                    "Credential::OAuthBearer {{ user: {user:?}, access_token: <redacted> }}"
                )
            }
        }
    }
}

/// Supplies a credential, refreshing it if the scheme requires that.
///
/// Async because OAuth2 access tokens expire and refreshing one is an HTTP
/// round trip. A blocking refresh would stall the runtime mid-sync.
///
/// Implementations must never wait for human interaction here: `credential()`
/// is called during sync, so an expired login has to fail with a clear error
/// telling the user to re-authenticate, not hang on a browser prompt. The
/// interactive part lives in [`oauth::OAuth2Device::device_login`].
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync {
    async fn credential(&self) -> Result<Credential>;

    /// Stable name for logs and the `account.auth_method` column.
    fn method(&self) -> &'static str;
}

const KEYRING_SERVICE: &str = "fuckmail";

/// App-specific password held in the OS keychain.
///
/// Credentials never touch a config file. On macOS this is the login keychain,
/// on Linux the Secret Service, on Windows the credential manager.
pub struct KeychainPassword {
    service: String,
    account: String,
}

impl KeychainPassword {
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(&self.service, &self.account)?)
    }

    pub fn store(&self, password: &str) -> Result<()> {
        self.entry()?.set_password(password)?;
        Ok(())
    }

    pub fn delete(&self) -> Result<()> {
        self.entry()?.delete_credential()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl AuthProvider for KeychainPassword {
    async fn credential(&self) -> Result<Credential> {
        match self.entry()?.get_password() {
            Ok(password) => Ok(Credential::Password(password)),
            Err(keyring::Error::NoEntry) => Err(AuthError::NotFound {
                service: self.service.clone(),
                account: self.account.clone(),
            }),
            Err(err) => Err(err.into()),
        }
    }

    fn method(&self) -> &'static str {
        "app_password"
    }
}

/// Reads the password from an environment variable.
///
/// This exists because CI and the dev Docker server must authenticate without a
/// keychain — on macOS every keychain read from a test binary raises a GUI
/// prompt, which would make `cargo test` interactive.
pub struct EnvPassword {
    var: String,
}

impl EnvPassword {
    pub fn new(var: impl Into<String>) -> Self {
        Self { var: var.into() }
    }
}

#[async_trait::async_trait]
impl AuthProvider for EnvPassword {
    async fn credential(&self) -> Result<Credential> {
        std::env::var(&self.var)
            .map(Credential::Password)
            .map_err(|_| AuthError::MissingEnv(self.var.clone()))
    }

    fn method(&self) -> &'static str {
        "app_password"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn env_provider_reads_the_variable() {
        // SAFETY: single-threaded test, no other thread reads the environment.
        unsafe { std::env::set_var("FUCKMAIL_TEST_PW", "devpass") };
        let provider = EnvPassword::new("FUCKMAIL_TEST_PW");
        assert_eq!(
            provider.credential().await.unwrap(),
            Credential::Password("devpass".into())
        );
    }

    #[tokio::test]
    async fn env_provider_reports_a_missing_variable() {
        let provider = EnvPassword::new("FUCKMAIL_TEST_DEFINITELY_UNSET");
        assert!(matches!(
            provider.credential().await,
            Err(AuthError::MissingEnv(_))
        ));
    }

    #[test]
    fn credentials_are_redacted_in_debug_output() {
        // A password reaching a log line would be a real incident; assert on it.
        let password = Credential::Password("hunter2".into());
        assert!(!format!("{password:?}").contains("hunter2"));

        let bearer = Credential::OAuthBearer {
            user: "dev@fuckmail.test".into(),
            access_token: "ya29.secret".into(),
        };
        let rendered = format!("{bearer:?}");
        assert!(!rendered.contains("ya29.secret"));
        assert!(rendered.contains("dev@fuckmail.test"));
    }
}
