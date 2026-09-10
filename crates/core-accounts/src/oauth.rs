//! OAuth2 device authorization grant (RFC 8628).
//!
//! Microsoft 365 has disabled basic authentication for IMAP, so a token is the
//! only way in. The device flow is the right grant for a desktop mail client:
//! it needs no redirect URI, no embedded browser, and no client secret — the
//! user opens a URL, types a short code, and the app polls until that happens.
//!
//! Two rules shape the design:
//!
//! 1. **`credential()` never waits for a human.** It is called during sync. An
//!    expired login fails with an actionable error; only [`OAuth2Device::device_login`]
//!    is interactive.
//! 2. **Only the refresh token is persisted.** Access tokens live about an hour
//!    and are kept in memory, so a stolen database yields nothing usable
//!    without also compromising the keychain.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::{AuthError, AuthProvider, Credential, Result};

/// Refresh a little before expiry so a token cannot lapse mid-sync.
const EXPIRY_MARGIN: Duration = Duration::from_secs(120);

const KEYRING_SERVICE: &str = "fuckmail-oauth";

/// Endpoints and client identity for one provider.
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    pub user: String,
    pub client_id: String,
    pub device_code_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

impl OAuth2Config {
    /// Microsoft 365 / Outlook.com.
    ///
    /// `tenant` is your directory ID, or `"common"` for personal accounts. The
    /// client ID comes from an app registration in Azure AD — for your own
    /// tenant that is a few minutes of clicking, with none of the verification
    /// process Google requires of a distributed client.
    pub fn microsoft(user: impl Into<String>, client_id: impl Into<String>, tenant: &str) -> Self {
        let authority = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0");
        Self {
            user: user.into(),
            client_id: client_id.into(),
            device_code_url: format!("{authority}/devicecode"),
            token_url: format!("{authority}/token"),
            scopes: vec![
                // The IMAP scope proper, plus offline_access — without it the
                // token response carries no refresh token and the user would
                // have to re-authorise every hour.
                "https://outlook.office.com/IMAP.AccessAsUser.All".into(),
                "offline_access".into(),
            ],
        }
    }
}

/// What to show the user so they can authorise the app.
#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_in: Duration,
}

/// Where the refresh token is kept.
///
/// A trait so tests never touch the real keychain — on macOS every keychain
/// read from a test binary raises a GUI prompt, which would make `cargo test`
/// interactive.
pub trait TokenStore: Send + Sync {
    fn load(&self, user: &str) -> Result<Option<String>>;
    fn save(&self, user: &str, refresh_token: &str) -> Result<()>;
    fn clear(&self, user: &str) -> Result<()>;
}

/// Refresh tokens in the OS keychain.
pub struct KeychainTokens;

impl TokenStore for KeychainTokens {
    fn load(&self, user: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, user)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn save(&self, user: &str, refresh_token: &str) -> Result<()> {
        keyring::Entry::new(KEYRING_SERVICE, user)?.set_password(refresh_token)?;
        Ok(())
    }

    fn clear(&self, user: &str) -> Result<()> {
        match keyring::Entry::new(KEYRING_SERVICE, user)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

/// In-memory token store, for tests.
#[derive(Default)]
pub struct MemoryTokens {
    inner: Mutex<Option<String>>,
}

impl TokenStore for MemoryTokens {
    fn load(&self, _user: &str) -> Result<Option<String>> {
        Ok(self.inner.lock().unwrap().clone())
    }

    fn save(&self, _user: &str, refresh_token: &str) -> Result<()> {
        *self.inner.lock().unwrap() = Some(refresh_token.to_string());
        Ok(())
    }

    fn clear(&self, _user: &str) -> Result<()> {
        *self.inner.lock().unwrap() = None;
        Ok(())
    }
}

/// A live access token and when it stops being usable.
#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: SystemTime,
}

impl CachedToken {
    fn usable(&self) -> bool {
        SystemTime::now() + EXPIRY_MARGIN < self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    /// Seconds to wait between polls. Optional per RFC 8628; defaults to 5.
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    /// Seconds. Absent in some implementations; an hour is the usual value.
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
    error_description: Option<String>,
}

pub struct OAuth2Device {
    config: OAuth2Config,
    http: reqwest::Client,
    tokens: Box<dyn TokenStore>,
    cached: Mutex<Option<CachedToken>>,
}

impl OAuth2Device {
    pub fn new(config: OAuth2Config) -> Self {
        Self::with_store(config, Box::new(KeychainTokens))
    }

    pub fn with_store(config: OAuth2Config, tokens: Box<dyn TokenStore>) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            tokens,
            cached: Mutex::new(None),
        }
    }

    pub fn user(&self) -> &str {
        &self.config.user
    }

    /// Runs the interactive device flow and stores the resulting refresh token.
    ///
    /// `prompt` is called once with the code and URL to show the user. Polling
    /// then continues until they authorise, the code expires, or the provider
    /// refuses.
    pub async fn device_login(&self, prompt: impl FnOnce(&DeviceCodePrompt)) -> Result<()> {
        let scope = self.config.scopes.join(" ");
        let response = self
            .http
            .post(&self.config.device_code_url)
            .form(&[
                ("client_id", self.config.client_id.as_str()),
                ("scope", scope.as_str()),
            ])
            .send()
            .await?;

        let device: DeviceCodeResponse = parse_json(response).await?;

        prompt(&DeviceCodePrompt {
            verification_uri: device.verification_uri.clone(),
            user_code: device.user_code.clone(),
            expires_in: Duration::from_secs(device.expires_in),
        });

        let deadline = SystemTime::now() + Duration::from_secs(device.expires_in);
        let mut interval = Duration::from_secs(device.interval.unwrap_or(5));

        loop {
            tokio::time::sleep(interval).await;

            if SystemTime::now() > deadline {
                return Err(AuthError::DeviceCodeExpired);
            }

            let response = self
                .http
                .post(&self.config.token_url)
                .form(&[
                    ("client_id", self.config.client_id.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device.device_code.as_str()),
                ])
                .send()
                .await?;

            match self.read_token_response(response).await {
                Ok(()) => return Ok(()),
                // The user has not finished authorising. Expected, keep polling.
                Err(AuthError::Provider { error, .. }) if error == "authorization_pending" => {}
                // Polling too fast. RFC 8628 says add 5 seconds and continue.
                Err(AuthError::Provider { error, .. }) if error == "slow_down" => {
                    interval += Duration::from_secs(5);
                    tracing::debug!(?interval, "provider asked us to slow down");
                }
                Err(AuthError::Provider { error, .. }) if error == "expired_token" => {
                    return Err(AuthError::DeviceCodeExpired);
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Forgets the stored login.
    pub fn logout(&self) -> Result<()> {
        *self.cached.lock().unwrap() = None;
        self.tokens.clear(&self.config.user)
    }

    async fn refresh(&self) -> Result<()> {
        let Some(refresh_token) = self.tokens.load(&self.config.user)? else {
            return Err(AuthError::NotLoggedIn(self.config.user.clone()));
        };

        let scope = self.config.scopes.join(" ");
        let response = self
            .http
            .post(&self.config.token_url)
            .form(&[
                ("client_id", self.config.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("scope", scope.as_str()),
            ])
            .send()
            .await?;

        match self.read_token_response(response).await {
            Ok(()) => Ok(()),
            // A refresh token that the provider rejects will never work again;
            // dropping it turns every later sync into one clear "log in" error
            // rather than a repeating failed round trip.
            Err(AuthError::Provider {
                error, description, ..
            }) if matches!(
                error.as_str(),
                "invalid_grant" | "invalid_client" | "unauthorized_client"
            ) =>
            {
                self.tokens.clear(&self.config.user)?;
                Err(AuthError::LoginExpired {
                    user: self.config.user.clone(),
                    reason: description.unwrap_or(error),
                })
            }
            Err(other) => Err(other),
        }
    }

    /// Reads a token response, caching the access token and persisting any
    /// refresh token that came with it.
    async fn read_token_response(&self, response: reqwest::Response) -> Result<()> {
        let token: TokenResponse = parse_json(response).await?;

        // Providers may rotate the refresh token; when they do, the old one
        // stops working, so the new one has to replace it.
        if let Some(refresh_token) = token.refresh_token.as_deref() {
            self.tokens.save(&self.config.user, refresh_token)?;
        }

        let lifetime = Duration::from_secs(token.expires_in.unwrap_or(3600));
        *self.cached.lock().unwrap() = Some(CachedToken {
            access_token: token.access_token,
            expires_at: SystemTime::now() + lifetime,
        });

        Ok(())
    }
}

#[async_trait::async_trait]
impl AuthProvider for OAuth2Device {
    async fn credential(&self) -> Result<Credential> {
        // Fast path: a token already in hand and not near expiry.
        if let Some(cached) = self.cached.lock().unwrap().clone() {
            if cached.usable() {
                return Ok(Credential::OAuthBearer {
                    user: self.config.user.clone(),
                    access_token: cached.access_token,
                });
            }
        }

        self.refresh().await?;

        let cached = self
            .cached
            .lock()
            .unwrap()
            .clone()
            .expect("refresh stores a token or returns an error");

        Ok(Credential::OAuthBearer {
            user: self.config.user.clone(),
            access_token: cached.access_token,
        })
    }

    fn method(&self) -> &'static str {
        "oauth2"
    }
}

/// Decodes a response, turning an OAuth error body into a typed error.
///
/// Providers signal `authorization_pending` with HTTP 400, so status alone
/// cannot distinguish "still waiting" from "broken" — the body has to be read
/// either way.
async fn parse_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let body = response.text().await?;

    if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
        return Err(AuthError::Provider {
            provider: "oauth2",
            error: err.error,
            description: err.error_description,
        });
    }

    serde_json::from_str::<T>(&body).map_err(|e| AuthError::Provider {
        provider: "oauth2",
        error: format!("unreadable response (HTTP {status})"),
        description: Some(e.to_string()),
    })
}
