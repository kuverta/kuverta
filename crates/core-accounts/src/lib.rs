//! Credential acquisition, behind a trait.
//!
//! Auth is pluggable from the first commit on purpose. Day-one accounts use
//! app-specific passwords, but Microsoft 365 has disabled basic auth for IMAP
//! and needs OAuth2, so a concrete `String` password threaded through the sync
//! code would have to be torn out within weeks. See
//! docs/implementation-plan.md section 2.

use std::fmt;

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
pub trait AuthProvider: Send + Sync {
    fn credential(&self) -> Result<Credential>;

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

impl AuthProvider for KeychainPassword {
    fn credential(&self) -> Result<Credential> {
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

impl AuthProvider for EnvPassword {
    fn credential(&self) -> Result<Credential> {
        std::env::var(&self.var)
            .map(Credential::Password)
            .map_err(|_| AuthError::MissingEnv(self.var.clone()))
    }

    fn method(&self) -> &'static str {
        "app_password"
    }
}

/// OAuth2 device-code flow, for Microsoft 365 and Gmail.
///
/// Not implemented yet — scheduled for month 3, once two accounts already sync.
/// The type exists now so the trait shape is proven against a second scheme
/// rather than being retrofitted around `KeychainPassword`.
pub struct OAuth2Device {
    pub user: String,
    pub tenant: String,
    pub client_id: String,
}

impl AuthProvider for OAuth2Device {
    fn credential(&self) -> Result<Credential> {
        Err(AuthError::Unimplemented("OAuth2 device-code flow"))
    }

    fn method(&self) -> &'static str {
        "oauth2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_provider_reads_the_variable() {
        // SAFETY: single-threaded test, no other thread reads the environment.
        unsafe { std::env::set_var("FUCKMAIL_TEST_PW", "devpass") };
        let provider = EnvPassword::new("FUCKMAIL_TEST_PW");
        assert_eq!(
            provider.credential().unwrap(),
            Credential::Password("devpass".into())
        );
    }

    #[test]
    fn env_provider_reports_a_missing_variable() {
        let provider = EnvPassword::new("FUCKMAIL_TEST_DEFINITELY_UNSET");
        assert!(matches!(
            provider.credential(),
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
