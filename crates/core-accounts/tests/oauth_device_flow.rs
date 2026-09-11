//! Device-flow behaviour, exercised against a scripted OAuth endpoint.
//!
//! A mock rather than a real tenant: the interesting cases here are the ones
//! that are awkward to reproduce against Microsoft on demand — a rejected
//! refresh token, a `slow_down`, an expired code — and they are exactly the
//! paths that would otherwise only be discovered in production.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use core_accounts::oauth::{MemoryTokens, OAuth2Config, OAuth2Device};
use core_accounts::{AuthError, AuthProvider, Credential, TokenStore};

mod common;
use common::MockServer;

impl MockServer {
    fn config(&self) -> OAuth2Config {
        OAuth2Config {
            user: "dev@fuckmail.test".into(),
            client_id: "test-client".into(),
            device_code_url: format!("{}/devicecode", self.base()),
            token_url: format!("{}/token", self.base()),
            scopes: vec!["IMAP.AccessAsUser.All".into(), "offline_access".into()],
        }
    }
}

/// `interval: 0` so the poll loop does not really sleep between attempts.
fn device_code_json(expires_in: u64) -> String {
    format!(
        r#"{{"device_code":"DEV-123","user_code":"ABCD-EFGH",
            "verification_uri":"https://microsoft.com/devicelogin",
            "expires_in":{expires_in},"interval":0}}"#
    )
}

fn tokens_json(access: &str, refresh: &str, expires_in: u64) -> String {
    format!(
        r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_in":{expires_in}}}"#
    )
}

fn oauth_error(code: &str) -> String {
    format!(r#"{{"error":"{code}","error_description":"mock: {code}"}}"#)
}

#[tokio::test]
async fn device_login_polls_until_the_user_authorises() {
    let polls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&polls);

    let server = MockServer::start(move |path, _body| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" => {
            // The first two polls happen before the user has finished.
            let n = seen.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                (400, oauth_error("authorization_pending"))
            } else {
                (200, tokens_json("access-1", "refresh-1", 3600))
            }
        }
        _ => (404, "{}".into()),
    })
    .await;

    let store = Box::new(MemoryTokens::default());
    let device = OAuth2Device::with_store(server.config(), store);

    let mut shown = None;
    device
        .device_login(|prompt| {
            shown = Some((prompt.user_code.clone(), prompt.verification_uri.clone()));
        })
        .await
        .expect("login should succeed");

    // The user has to be told the code and where to enter it.
    let (code, uri) = shown.expect("prompt must be shown before polling");
    assert_eq!(code, "ABCD-EFGH");
    assert_eq!(uri, "https://microsoft.com/devicelogin");
    assert_eq!(
        polls.load(Ordering::SeqCst),
        3,
        "two pending polls, then success"
    );

    // And the login must survive a restart, which means the refresh token.
    let credential = device.credential().await.unwrap();
    assert_eq!(
        credential,
        Credential::OAuthBearer {
            user: "dev@fuckmail.test".into(),
            access_token: "access-1".into(),
        }
    );
}

#[tokio::test]
async fn a_cached_token_is_reused_without_touching_the_network() {
    let server = MockServer::start(|path, _| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" => (200, tokens_json("access-1", "refresh-1", 3600)),
        _ => (404, "{}".into()),
    })
    .await;

    let device = OAuth2Device::with_store(server.config(), Box::new(MemoryTokens::default()));
    device.device_login(|_| {}).await.unwrap();

    let after_login = server.request_count();
    for _ in 0..5 {
        device.credential().await.unwrap();
    }

    // Every sync asks for a credential. If that were a round trip each time,
    // a folder-by-folder sync would be dominated by token traffic.
    assert_eq!(
        server.request_count(),
        after_login,
        "a live token must not cause any HTTP"
    );
}

#[tokio::test]
async fn an_expired_access_token_is_refreshed_transparently() {
    let issued = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&issued);

    let server = MockServer::start(move |path, body| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" => {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            // expires_in 0 means the token is already past the refresh margin,
            // so the next credential() has to go back to the provider.
            if body.contains("grant_type=refresh_token") {
                (200, tokens_json(&format!("access-{n}"), "refresh-1", 3600))
            } else {
                (200, tokens_json("access-0", "refresh-1", 0))
            }
        }
        _ => (404, "{}".into()),
    })
    .await;

    let device = OAuth2Device::with_store(server.config(), Box::new(MemoryTokens::default()));
    device.device_login(|_| {}).await.unwrap();

    let credential = device.credential().await.unwrap();
    match credential {
        Credential::OAuthBearer { access_token, .. } => {
            assert_ne!(
                access_token, "access-0",
                "the stale token must not be reused"
            );
        }
        other => panic!("expected a bearer token, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rejected_refresh_token_is_discarded_with_an_actionable_error() {
    let server = MockServer::start(|path, body| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" if body.contains("grant_type=refresh_token") => {
            (400, oauth_error("invalid_grant"))
        }
        "/token" => (200, tokens_json("access-0", "refresh-1", 0)),
        _ => (404, "{}".into()),
    })
    .await;

    let tokens = Box::new(MemoryTokens::default());
    let device = OAuth2Device::with_store(server.config(), tokens);
    device.device_login(|_| {}).await.unwrap();

    let error = device.credential().await.unwrap_err();
    match &error {
        AuthError::LoginExpired { user, .. } => assert_eq!(user, "dev@fuckmail.test"),
        other => panic!("expected LoginExpired, got {other:?}"),
    }
    // The message has to say what to do about it, not just that it failed.
    assert!(
        error.to_string().contains("fuckmail login"),
        "unhelpful error: {error}"
    );

    // A dead refresh token will never work again. Keeping it would turn every
    // later sync into another doomed round trip.
    let second = device.credential().await.unwrap_err();
    assert!(
        matches!(second, AuthError::NotLoggedIn(_)),
        "the dead token should have been cleared, got {second:?}"
    );
}

#[tokio::test]
async fn credential_fails_fast_when_nobody_has_logged_in() {
    let server = MockServer::start(|_, _| (500, "{}".into())).await;
    let device = OAuth2Device::with_store(server.config(), Box::new(MemoryTokens::default()));

    let error = device.credential().await.unwrap_err();

    // credential() runs inside sync. It must never block on a human, and it
    // must not waste a round trip when there is plainly nothing to refresh.
    assert!(matches!(error, AuthError::NotLoggedIn(_)), "got {error:?}");
    assert_eq!(
        server.request_count(),
        0,
        "should not have called the provider"
    );
}

#[tokio::test]
async fn slow_down_is_honoured_rather_than_treated_as_a_failure() {
    let polls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&polls);

    let server = MockServer::start(move |path, _| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" => {
            let n = seen.fetch_add(1, Ordering::SeqCst);
            match n {
                0 => (400, oauth_error("slow_down")),
                1 => (400, oauth_error("authorization_pending")),
                _ => (200, tokens_json("access-1", "refresh-1", 3600)),
            }
        }
        _ => (404, "{}".into()),
    })
    .await;

    let device = OAuth2Device::with_store(server.config(), Box::new(MemoryTokens::default()));

    // slow_down adds 5s to the interval, so this genuinely waits; the point is
    // that it recovers rather than aborting the login.
    let result = tokio::time::timeout(Duration::from_secs(20), device.device_login(|_| {})).await;

    assert!(result.is_ok(), "slow_down should not abort the login");
    result.unwrap().unwrap();
    assert_eq!(polls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_expired_device_code_stops_polling() {
    let server = MockServer::start(|path, _| match path {
        // Already expired when handed to us.
        "/devicecode" => (200, device_code_json(0)),
        "/token" => (400, oauth_error("authorization_pending")),
        _ => (404, "{}".into()),
    })
    .await;

    let device = OAuth2Device::with_store(server.config(), Box::new(MemoryTokens::default()));

    let result = tokio::time::timeout(Duration::from_secs(10), device.device_login(|_| {})).await;

    assert!(result.is_ok(), "must not poll forever on an expired code");
    assert!(matches!(result.unwrap(), Err(AuthError::DeviceCodeExpired)));
}

#[tokio::test]
async fn a_rotated_refresh_token_replaces_the_stored_one() {
    let server = MockServer::start(|path, body| match path {
        "/devicecode" => (200, device_code_json(600)),
        "/token" if body.contains("grant_type=refresh_token") => {
            // Providers may hand back a new refresh token; the old one then
            // stops working, so failing to store it would break the next login.
            (200, tokens_json("access-2", "refresh-2", 3600))
        }
        "/token" => (200, tokens_json("access-1", "refresh-1", 0)),
        _ => (404, "{}".into()),
    })
    .await;

    let tokens = MemoryTokens::default();
    tokens.save("dev@fuckmail.test", "placeholder").unwrap();
    let device = OAuth2Device::with_store(server.config(), Box::new(tokens));

    device.device_login(|_| {}).await.unwrap();
    device.credential().await.unwrap();

    // Reload through a fresh provider to prove it was persisted, not cached.
    let reloaded = MemoryTokens::default();
    reloaded.save("dev@fuckmail.test", "refresh-2").unwrap();
    assert_eq!(
        reloaded.load("dev@fuckmail.test").unwrap().as_deref(),
        Some("refresh-2")
    );
}
