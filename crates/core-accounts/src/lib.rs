//! Credential acquisition, behind a trait.
//!
//! Auth is pluggable from the first commit on purpose. Day-one accounts use
//! app-specific passwords, but Microsoft 365 has disabled basic auth for IMAP
//! and needs OAuth2, so a concrete `String` password threaded through the sync
//! code would have to be torn out within weeks. See
//! docs/implementation-plan.md section 2.

use std::fmt;

pub mod loopback;
pub mod oauth;

pub use loopback::{LoopbackConfig, LoopbackPrompt, OAuth2Loopback};
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

    #[error("no stored login for {0}; run `kuverta login --email {0}`")]
    NotLoggedIn(String),

    #[error("the stored login for {user} is no longer valid ({reason}); run `kuverta login --email {user}`")]
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

const KEYRING_SERVICE: &str = "kuverta";

/// Which kuverta this is, from `KUVERTA_INSTANCE`: `None` for the one people
/// use, `Some("dev")` for the one run against the Docker dev stack.
///
/// An instance keeps its own keychain entries and, unless told otherwise, its
/// own data directory, so a development build can never read or overwrite the
/// credentials of the installed app — even for an address configured in both.
pub fn instance() -> Option<String> {
    instance_from(std::env::var("KUVERTA_INSTANCE").ok().as_deref())
}

/// `instance` for a given value, so it can be tested without the environment.
pub fn instance_from(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    let usable = !value.is_empty()
        && value != "default"
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    usable.then_some(value)
}

/// A keychain service name, kept apart for an instance: `kuverta` for the one
/// people use, `kuverta-dev` for the dev instance.
pub fn keyring_service(base: &str) -> String {
    service_for(base, instance().as_deref())
}

pub fn service_for(base: &str, instance: Option<&str>) -> String {
    match instance {
        Some(instance) => format!("{base}-{instance}"),
        None => base.to_string(),
    }
}

/// Where an instance keeps its data unless `KUVERTA_DATA_DIR` says otherwise:
/// `~/.local/share/kuverta`, or `~/.local/share/kuverta-dev` for the dev one.
pub fn default_data_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("KUVERTA_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".local/share")
        .join(service_for("kuverta", instance().as_deref()))
}

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
            service: keyring_service(KEYRING_SERVICE),
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(&self.service, &self.account)?)
    }

    /// Reads the entry without the async ceremony of [`AuthProvider`].
    ///
    /// The keychain is not async and never was; the trait is, because token
    /// refresh is. A caller that only wants to know whether something is
    /// stored should not have to reach for a runtime to find out.
    pub fn peek(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
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

    #[test]
    fn an_instance_keeps_its_own_keychain_service() {
        assert_eq!(instance_from(None), None);
        assert_eq!(instance_from(Some("")), None);
        assert_eq!(instance_from(Some("default")), None);
        assert_eq!(instance_from(Some(" Dev ")), Some("dev".to_string()));
        assert_eq!(instance_from(Some("../evil")), None, "not a name");

        assert_eq!(service_for("kuverta", None), "kuverta");
        assert_eq!(service_for("kuverta", Some("dev")), "kuverta-dev");
        assert_eq!(
            service_for("kuverta-oauth", Some("dev")),
            "kuverta-oauth-dev"
        );
    }

    #[tokio::test]
    async fn env_provider_reads_the_variable() {
        // SAFETY: single-threaded test, no other thread reads the environment.
        unsafe { std::env::set_var("KUVERTA_TEST_PW", "devpass") };
        let provider = EnvPassword::new("KUVERTA_TEST_PW");
        assert_eq!(
            provider.credential().await.unwrap(),
            Credential::Password("devpass".into())
        );
    }

    #[tokio::test]
    async fn env_provider_reports_a_missing_variable() {
        let provider = EnvPassword::new("KUVERTA_TEST_DEFINITELY_UNSET");
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
            user: "dev@kuverta.test".into(),
            access_token: "ya29.secret".into(),
        };
        let rendered = format!("{bearer:?}");
        assert!(!rendered.contains("ya29.secret"));
        assert!(rendered.contains("dev@kuverta.test"));
    }
}
