//! Loopback-redirect flow, exercised against a scripted OAuth endpoint.
//!
//! The browser is simulated: `login` hands back the authorization URL it would
//! have opened, and the test plays the part of Google by requesting the
//! redirect URI itself. That makes the interesting cases — a refused consent,
//! a mismatched `state`, a browser asking for the favicon on the same port —
//! reproducible, which against the real thing they are not.

mod common;
use common::MockServer;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use core_accounts::loopback::{LoopbackConfig, LoopbackPrompt, OAuth2Loopback};
use core_accounts::oauth::MemoryTokens;
use core_accounts::{AuthError, AuthProvider, Credential};

fn config(server: &MockServer) -> LoopbackConfig {
    LoopbackConfig {
        user: "dev@kuverta.test".into(),
        client_id: "test-client".into(),
        client_secret: Some("not-really-secret".into()),
        auth_url: format!("{}/auth", server.base()),
        token_url: format!("{}/token", server.base()),
        scopes: vec!["https://mail.google.com/".into()],
    }
}

fn tokens_json(access: &str, refresh: &str) -> String {
    format!(r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_in":3600}}"#)
}

fn query_of(url: &str) -> HashMap<String, String> {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn form_of(body: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{body}"))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Plays the browser: requests `path` on the loopback listener and returns the
/// page it was shown.
async fn visit(redirect_uri: &str, query: &str) -> String {
    reqwest::Client::new()
        .get(format!("{redirect_uri}{query}"))
        .send()
        .await
        .expect("the loopback listener should answer")
        .text()
        .await
        .unwrap_or_default()
}

/// Runs `login` while a "browser" drives the redirect, and returns both halves.
async fn login_with<F, Fut>(
    client: &OAuth2Loopback,
    browser: F,
) -> (Result<(), AuthError>, LoopbackPrompt)
where
    F: FnOnce(LoopbackPrompt) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let seen: Arc<std::sync::Mutex<Option<LoopbackPrompt>>> = Arc::default();
    let captured = Arc::clone(&seen);

    let login = client.login(move |prompt| {
        *captured.lock().unwrap() = Some(prompt.clone());
        let _ = tx.send(prompt.clone());
    });

    let drive = async move {
        let prompt = rx.await.expect("login should prompt before waiting");
        browser(prompt).await;
    };

    let (result, ()) = tokio::join!(login, drive);
    let prompt = seen.lock().unwrap().clone().expect("prompted");
    (result, prompt)
}

#[tokio::test]
async fn a_completed_login_stores_a_refresh_token_and_proves_possession_of_the_verifier() {
    // The PKCE check is the point. The mock plays a conforming authorization
    // server: it remembers the challenge from the authorization request and
    // refuses the exchange unless the verifier hashes to it. A client that
    // sent the wrong verifier — or none — would fail here rather than silently
    // work, which is what makes the test worth having.
    let challenge: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let recorded = Arc::clone(&challenge);

    let server = MockServer::start(move |path, body| match path {
        "/token" => {
            let form = form_of(body);
            let expected = recorded
                .lock()
                .unwrap()
                .clone()
                .expect("challenge recorded");
            let verifier = form.get("code_verifier").cloned().unwrap_or_default();

            use base64::Engine;
            use sha2::{Digest, Sha256};
            let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));

            if computed != expected {
                return (
                    400,
                    r#"{"error":"invalid_grant","error_description":"PKCE mismatch"}"#.into(),
                );
            }
            if form.get("code").map(String::as_str) != Some("AUTH-CODE-1") {
                return (400, r#"{"error":"invalid_grant"}"#.into());
            }
            // Google requires the secret from a desktop client too.
            if !form.contains_key("client_secret") {
                return (400, r#"{"error":"invalid_client"}"#.into());
            }
            (200, tokens_json("access-1", "refresh-1"))
        }
        _ => (404, "{}".into()),
    })
    .await;

    let client = OAuth2Loopback::with_store(config(&server), Box::new(MemoryTokens::default()));

    let (result, prompt) = login_with(&client, |prompt| {
        let challenge = Arc::clone(&challenge);
        async move {
            let params = query_of(&prompt.authorization_url);
            *challenge.lock().unwrap() = params.get("code_challenge").cloned();
            let state = params.get("state").cloned().unwrap();
            visit(
                &prompt.redirect_uri,
                &format!("/?code=AUTH-CODE-1&state={state}"),
            )
            .await;
        }
    })
    .await;

    result.expect("login should succeed");

    // The authorization request has to be the one Google documents, or the
    // response carries no refresh token and the login lasts an hour.
    let params = query_of(&prompt.authorization_url);
    assert_eq!(
        params.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        params.get("access_type").map(String::as_str),
        Some("offline")
    );
    assert_eq!(params.get("prompt").map(String::as_str), Some("consent"));
    assert_eq!(
        params.get("response_type").map(String::as_str),
        Some("code")
    );
    assert_eq!(
        params.get("scope").map(String::as_str),
        Some("https://mail.google.com/")
    );
    assert!(prompt.redirect_uri.starts_with("http://127.0.0.1:"));

    // And the session is usable afterwards, from the stored refresh token.
    match client.credential().await.unwrap() {
        Credential::OAuthBearer { access_token, user } => {
            assert_eq!(access_token, "access-1");
            assert_eq!(user, "dev@kuverta.test");
        }
        other => panic!("expected a bearer token, got {other:?}"),
    }
}

#[tokio::test]
async fn a_redirect_carrying_the_wrong_state_is_refused() {
    // Without this check, anything able to reach the listening port could hand
    // the client an authorization code of its choosing.
    let server =
        MockServer::start(|_path, _body| (200, tokens_json("access-x", "refresh-x"))).await;
    let client = OAuth2Loopback::with_store(config(&server), Box::new(MemoryTokens::default()));

    let (result, _) = login_with(&client, |prompt| async move {
        let page = visit(
            &prompt.redirect_uri,
            "/?code=INJECTED&state=not-the-state-we-sent",
        )
        .await;
        assert!(page.contains("did not come from this login"), "{page}");
    })
    .await;

    match result {
        Err(AuthError::Provider { error, .. }) => assert_eq!(error, "state mismatch"),
        other => panic!("expected a state mismatch, got {other:?}"),
    }
    assert_eq!(
        server.request_count(),
        0,
        "the token endpoint must not be called with an unverified code"
    );
}

#[tokio::test]
async fn a_refused_consent_is_reported_rather_than_hung_on() {
    let server = MockServer::start(|_path, _body| (200, tokens_json("a", "r"))).await;
    let client = OAuth2Loopback::with_store(config(&server), Box::new(MemoryTokens::default()));

    let (result, _) = login_with(&client, |prompt| async move {
        visit(&prompt.redirect_uri, "/?error=access_denied").await;
    })
    .await;

    match result {
        Err(AuthError::Provider { error, .. }) => assert_eq!(error, "access_denied"),
        other => panic!("expected the provider's refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unrelated_request_to_the_port_does_not_end_the_login() {
    // Browsers ask for /favicon.ico on whatever port they were sent to. If the
    // first stray request ended the wait, the login would fail roughly half
    // the time, depending on the browser.
    let probes = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&probes);

    let server = MockServer::start(move |path, _body| {
        seen.fetch_add(1, Ordering::SeqCst);
        match path {
            "/token" => (200, tokens_json("access-2", "refresh-2")),
            _ => (404, "{}".into()),
        }
    })
    .await;
    let client = OAuth2Loopback::with_store(config(&server), Box::new(MemoryTokens::default()));

    let (result, _) = login_with(&client, |prompt| async move {
        let page = visit(&prompt.redirect_uri, "/favicon.ico").await;
        assert!(page.contains("Waiting"), "{page}");

        let state = query_of(&prompt.authorization_url)
            .get("state")
            .cloned()
            .unwrap();
        visit(
            &prompt.redirect_uri,
            &format!("/?code=AUTH-CODE-2&state={state}"),
        )
        .await;
    })
    .await;

    result.expect("the login should have survived the favicon request");
    assert_eq!(probes.load(Ordering::SeqCst), 1, "only the token exchange");
}
