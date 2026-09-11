//! OAuth2 authorization code flow with a loopback redirect (RFC 8252).
//!
//! Gmail needs this and cannot use the device flow next door: Google does not
//! grant `https://mail.google.com/` — the IMAP and SMTP scope — to the
//! limited-input device grant, whatever the client asks for. The supported
//! route for a desktop mail client is an authorization code redirected to
//! `http://127.0.0.1:<port>`, which is what this is.
//!
//! It shares [`TokenSession`] with the device flow, so refreshing, caching and
//! rotating the stored token behave identically; only the way the user first
//! authorises differs.
//!
//! Three things make this safe rather than merely working:
//!
//! 1. **PKCE (RFC 7636).** The authorization code is useless without the
//!    verifier, so another process that sees the code — a loopback redirect is
//!    not confidential — cannot exchange it.
//! 2. **`state`.** The callback is rejected unless it carries back the value
//!    sent, so an unrelated request to the listening port cannot inject a code.
//! 3. **Loopback only.** The listener binds `127.0.0.1`, never `0.0.0.0`, so
//!    nothing off this machine can reach it.
//!
//! Both random values come from the OS. Deriving them from the clock would
//! make them guessable, which defeats the point of having them.

use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::oauth::{KeychainTokens, TokenSession, TokenStore};
use crate::{AuthError, AuthProvider, Credential, Result};

/// How long to wait for the user to finish authorising in their browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Endpoints and client identity for a loopback-redirect provider.
#[derive(Debug, Clone)]
pub struct LoopbackConfig {
    pub user: String,
    pub client_id: String,
    /// Google issues one for "Desktop app" clients and requires it in the
    /// token exchange. RFC 8252 is clear that it is not actually a secret —
    /// anyone with the binary has it — which is why PKCE, not this, is what
    /// protects the exchange.
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

impl LoopbackConfig {
    /// Gmail.
    ///
    /// The client id comes from a Google Cloud project with an OAuth client of
    /// type "Desktop app". `https://mail.google.com/` is a restricted scope, so
    /// a *published* client would need Google's CASA assessment — but a client
    /// left in testing with your own address as a test user does not, which is
    /// the whole basis of decision 4 in the plan.
    pub fn google(
        user: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            user: user.into(),
            client_id: client_id.into(),
            client_secret,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            // The one scope that covers IMAP, SMTP submission and nothing else.
            scopes: vec!["https://mail.google.com/".into()],
        }
    }
}

/// What to put in front of the user so they can authorise.
#[derive(Debug, Clone)]
pub struct LoopbackPrompt {
    pub authorization_url: String,
    /// Where the browser will be sent back to. Worth showing: it is the
    /// reassurance that the redirect is local.
    pub redirect_uri: String,
}

pub struct OAuth2Loopback {
    session: TokenSession,
    auth_url: String,
}

impl OAuth2Loopback {
    pub fn new(config: LoopbackConfig) -> Self {
        Self::with_store(config, Box::new(KeychainTokens))
    }

    pub fn with_store(config: LoopbackConfig, tokens: Box<dyn TokenStore>) -> Self {
        Self {
            auth_url: config.auth_url,
            session: TokenSession::new(
                config.user,
                config.client_id,
                config.client_secret,
                config.token_url,
                config.scopes,
                tokens,
            ),
        }
    }

    pub fn user(&self) -> &str {
        self.session.user()
    }

    /// Forgets the stored login.
    pub fn logout(&self) -> Result<()> {
        self.session.logout()
    }

    /// Runs the interactive flow and stores the resulting refresh token.
    ///
    /// `prompt` is called once with the URL to open. The listener is bound
    /// before that, so the port in the redirect URI is real by the time the
    /// user sees it and a browser that is quick cannot arrive early.
    pub async fn login(&self, prompt: impl FnOnce(&LoopbackPrompt)) -> Result<()> {
        let listener =
            TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| AuthError::Provider {
                    provider: "oauth2",
                    error: "cannot listen for the redirect".into(),
                    description: Some(e.to_string()),
                })?;
        let port = listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| AuthError::Provider {
                provider: "oauth2",
                error: "cannot read the redirect port".into(),
                description: Some(e.to_string()),
            })?;
        let redirect_uri = format!("http://127.0.0.1:{port}");

        let verifier = random_urlsafe(64)?;
        let challenge = code_challenge(&verifier);
        let state = random_urlsafe(32)?;
        let scope = self.session.scope();

        let url = reqwest::Url::parse_with_params(
            &self.auth_url,
            &[
                ("client_id", self.session.client_id()),
                ("redirect_uri", redirect_uri.as_str()),
                ("response_type", "code"),
                ("scope", scope.as_str()),
                ("state", state.as_str()),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                // Without offline access the response carries no refresh
                // token, and the login would last an hour.
                ("access_type", "offline"),
                // Google only re-issues a refresh token when consent is shown
                // again; without this, a second login returns none and the
                // stored one is never replaced.
                ("prompt", "consent"),
            ],
        )
        .map_err(|e| AuthError::Provider {
            provider: "oauth2",
            error: "cannot build the authorization URL".into(),
            description: Some(e.to_string()),
        })?;

        prompt(&LoopbackPrompt {
            authorization_url: url.to_string(),
            redirect_uri: redirect_uri.clone(),
        });

        let code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_code(&listener, &state))
            .await
            .map_err(|_| AuthError::DeviceCodeExpired)??;

        let form = self.session.token_form(vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
        ]);
        let response = self
            .session
            .http()
            .post(self.session.token_url())
            .form(&form)
            .send()
            .await?;

        self.session.read_token_response(response).await
    }
}

#[async_trait::async_trait]
impl AuthProvider for OAuth2Loopback {
    async fn credential(&self) -> Result<Credential> {
        self.session.credential().await
    }

    fn method(&self) -> &'static str {
        "oauth2"
    }
}

/// Accepts connections until one carries a usable callback.
///
/// A browser will cheerfully ask for `/favicon.ico` on the same port, and
/// anything else on the machine may probe it, so one unusable request must not
/// end the login.
async fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| AuthError::Provider {
            provider: "oauth2",
            error: "the redirect connection failed".into(),
            description: Some(e.to_string()),
        })?;

        let Some(target) = read_request_target(&mut socket).await else {
            respond(&mut socket, "Not a valid request.").await;
            continue;
        };

        // Parsed as a URL rather than split by hand, so percent-encoding and
        // repeated parameters are somebody else's solved problem.
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut socket, "Not a valid request.").await;
            continue;
        };
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();

        if let Some(error) = params.get("error") {
            respond(
                &mut socket,
                "Authorisation was refused. You can close this tab.",
            )
            .await;
            return Err(AuthError::Provider {
                provider: "google",
                error: error.to_string(),
                description: params.get("error_description").map(|d| d.to_string()),
            });
        }

        let (Some(code), Some(state)) = (params.get("code"), params.get("state")) else {
            // Not the callback — a favicon request, or a stray probe.
            respond(&mut socket, "Waiting for the authorisation redirect…").await;
            continue;
        };

        // Constant-time is not required here: `state` is a freshly generated
        // public nonce, not a secret being verified against a stored one.
        if state.as_ref() != expected_state {
            respond(&mut socket, "That request did not come from this login.").await;
            return Err(AuthError::Provider {
                provider: "oauth2",
                error: "state mismatch".into(),
                description: Some(
                    "the redirect did not carry back the value this login sent, so it was \
                     not the one that started here"
                        .into(),
                ),
            });
        }

        respond(
            &mut socket,
            "Signed in. You can close this tab and go back to the terminal.",
        )
        .await;
        return Ok(code.to_string());
    }
}

/// Reads the request target out of the first line of an HTTP request.
///
/// Only the request line is needed, and reading no further keeps a client that
/// never finishes its headers from holding the login open.
async fn read_request_target(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];

    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(line_end) = buffer.windows(2).position(|w| w == b"\r\n") {
            let line = String::from_utf8_lossy(&buffer[..line_end]).into_owned();
            let mut parts = line.split_whitespace();
            let method = parts.next()?;
            let target = parts.next()?;
            return (method == "GET").then(|| target.to_string());
        }

        // A request line this long is not a browser.
        if buffer.len() > 8192 {
            return None;
        }
    }
}

async fn respond(socket: &mut tokio::net::TcpStream, message: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <title>fuckmail</title>\
         <body style=\"font:16px system-ui;margin:4rem auto;max-width:30rem\">\
         <p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.flush().await;
}

/// RFC 7636 S256: BASE64URL(SHA256(verifier)), unpadded.
fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// `len` bytes of OS randomness, base64url-encoded.
///
/// The result is longer than `len` and uses only unreserved characters, which
/// is what RFC 7636 asks of a code verifier (43–128 of them).
fn random_urlsafe(len: usize) -> Result<String> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|e| AuthError::Provider {
        provider: "oauth2",
        error: "no source of randomness".into(),
        description: Some(e.to_string()),
    })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_challenge_matches_the_rfc_7636_worked_example() {
        // Appendix B of RFC 7636, so the derivation is checked against the
        // specification rather than against itself.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_values_are_unguessable_and_url_safe() {
        let a = random_urlsafe(64).unwrap();
        let b = random_urlsafe(64).unwrap();
        assert_ne!(a, b, "two draws must not collide");

        // RFC 7636 section 4.1: 43-128 characters from the unreserved set.
        assert!((43..=128).contains(&a.len()), "length was {}", a.len());
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')));
    }
}
