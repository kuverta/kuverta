//! SMTP submission.
//!
//! Deliberately thin: it opens a connection, authenticates with the account's
//! existing [`AuthProvider`], and hands over the bytes [`crate::compose`]
//! produced. No retry, no queue, no scheduling — a failed submission is
//! reported to the caller, which is the only honest thing to do while there is
//! a person watching. The queue arrives with stage 2 (plan section 1a), where
//! it is needed for mailbox mutations anyway.

use std::sync::Arc;

use core_accounts::{AuthProvider, Credential};
use core_store::model::{SmtpConfig, SmtpSecurity};
use mail_send::smtp::message::Message;
use mail_send::{Credentials, SmtpClient, SmtpClientBuilder};
use rustls::ClientConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
// Brings `with_platform_verifier` into scope on the config *builder*, which is
// the form needed when the provider is named explicitly.
use rustls_platform_verifier::BuilderVerifierExt as _;

use crate::compose::BuiltMessage;

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("smtp: {0}")]
    Smtp(String),

    #[error("credentials: {0}")]
    Auth(#[from] core_accounts::AuthError),

    #[error(
        "refusing to submit over an unencrypted connection to {0}; \
         plaintext SMTP is only allowed against localhost"
    )]
    InsecureTransport(String),

    #[error("tls: {0}")]
    Tls(String),
}

type Result<T> = std::result::Result<T, SubmitError>;

/// mail-send's own default is an hour, which is not a timeout so much as a
/// hang. A submission that has not completed in a minute has failed.
const SMTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

// mail_send::Error is not std::error::Error, so it cannot be `#[from]`-ed.
impl From<mail_send::Error> for SubmitError {
    fn from(err: mail_send::Error) -> Self {
        SubmitError::Smtp(err.to_string())
    }
}

/// Submits `message` and returns once the server has accepted it.
///
/// `username` is the SMTP AUTH identity, which is the account's IMAP username:
/// providers use one login for both, and where they do not, the account row is
/// wrong rather than this function.
pub async fn submit(
    config: &SmtpConfig,
    username: &str,
    auth: &dyn AuthProvider,
    message: &BuiltMessage,
) -> Result<()> {
    // Same rule as the IMAP side: cleartext is a dev-server affordance, not a
    // configuration option. Checked here as well as at the point of
    // registration, because a database edited by hand must not be able to send
    // a real password over the wire in the clear.
    if config.security == SmtpSecurity::Plaintext && !is_loopback(&config.host) {
        return Err(SubmitError::InsecureTransport(config.host.clone()));
    }

    let builder = client_builder(config, credentials_for(username, auth).await?)?;

    let envelope = Message::new(
        message.sender.clone(),
        message.recipients.clone(),
        message.rfc822.clone(),
    );

    tracing::debug!(
        host = %config.host,
        port = config.port,
        recipients = message.recipients.len(),
        message_id = %message.message_id,
        "submitting"
    );

    match config.security {
        // `connect_plain` speaks no TLS at all, which is what the local sink
        // wants. It is not `implicit_tls(false)`: that one still requires
        // STARTTLS and fails if the server does not offer it — the right
        // behaviour for a real submission endpoint, and the reason STARTTLS
        // cannot silently degrade to cleartext here.
        SmtpSecurity::Plaintext => deliver(builder.connect_plain().await?, envelope).await,
        _ => deliver(builder.connect().await?, envelope).await,
    }
}

/// Connects and authenticates, then hangs up without sending anything.
///
/// The preflight check for an account's submission endpoint: it proves the
/// host, port, transport and credentials all work, which is everything that
/// can go wrong before a message exists. Deliberately separate from [`submit`]
/// so checking cannot accidentally deliver.
pub async fn verify(config: &SmtpConfig, username: &str, auth: &dyn AuthProvider) -> Result<()> {
    if config.security == SmtpSecurity::Plaintext && !is_loopback(&config.host) {
        return Err(SubmitError::InsecureTransport(config.host.clone()));
    }

    let builder = client_builder(config, credentials_for(username, auth).await?)?;
    match config.security {
        SmtpSecurity::Plaintext => builder.connect_plain().await?.quit().await?,
        _ => builder.connect().await?.quit().await?,
    }
    Ok(())
}

async fn deliver<T: AsyncRead + AsyncWrite + Unpin>(
    mut client: SmtpClient<T>,
    envelope: Message<'_>,
) -> Result<()> {
    client.send(envelope).await?;
    // QUIT is best-effort: the server has already accepted the message by the
    // time DATA completes, so a failure to close cleanly is not a send failure.
    if let Err(err) = client.quit().await {
        tracing::debug!(%err, "smtp server did not acknowledge QUIT");
    }
    Ok(())
}

/// Builds the TLS connector, naming the crypto provider explicitly.
///
/// rustls picks a process-wide default provider only when exactly one is
/// compiled in. This workspace has two — reqwest's rustls feature pulls
/// aws-lc-rs, mail-send pulls ring — so anything relying on that default
/// panics at the first handshake rather than failing at build time. Naming the
/// provider here avoids both the panic and the process-global install, which a
/// library has no business performing on its caller's behalf.
///
/// `core-proto::client::open_transport` does the same thing for IMAP, for the
/// same reason. Keep the two in step.
fn tls_connector() -> Result<TlsConnector> {
    let tls = |err: rustls::Error| SubmitError::Tls(err.to_string());

    // The OS trust store rather than a bundled root list, so enterprise or
    // pinned roots keep working — matching the IMAP transport.
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(tls)?
            .with_platform_verifier()
            .map_err(tls)?
            .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

async fn credentials_for(username: &str, auth: &dyn AuthProvider) -> Result<Credentials<String>> {
    Ok(match auth.credential().await? {
        Credential::Password(password) => Credentials::Plain {
            username: username.to_string(),
            secret: password,
        },
        // XOAUTH2 rather than OAUTHBEARER: it is what Microsoft 365 and Gmail
        // implement for submission, and it is what the IMAP side already uses.
        Credential::OAuthBearer { user, access_token } => Credentials::XOauth2 {
            username: user,
            secret: access_token,
        },
    })
}

/// Built field by field rather than via `SmtpClientBuilder::new`, which
/// constructs its own TLS connector eagerly — and panics doing so, for the
/// reason [`tls_connector`] explains. Every field is part of mail-send's public
/// API.
fn client_builder(
    config: &SmtpConfig,
    credentials: Credentials<String>,
) -> Result<SmtpClientBuilder<String>> {
    Ok(SmtpClientBuilder {
        addr: format!("{}:{}", config.host, config.port),
        timeout: SMTP_TIMEOUT,
        tls_connector: tls_connector()?,
        tls_hostname: config.host.clone(),
        tls_implicit: config.security == SmtpSecurity::Tls,
        credentials: Some(credentials),
        is_lmtp: false,
        say_ehlo: true,
        // An address literal rather than this machine's hostname, which RFC
        // 5321 section 4.1.3 allows and which keeps the sender's laptop name
        // out of the Received chain. Submission is authenticated, so the
        // server has no reason to care what we call ourselves; a relay would.
        local_host: "[127.0.0.1]".to_string(),
        local_ip: None,
    })
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_accounts::EnvPassword;

    fn message() -> BuiltMessage {
        BuiltMessage {
            message_id: "test@kuverta.test".into(),
            rfc822: b"Subject: hi\r\n\r\nhi".to_vec(),
            recipients: vec!["jane@example.com".into()],
            sender: "erika@kuverta.test".into(),
        }
    }

    #[test]
    fn a_tls_connector_can_be_built() {
        // Regression test with no server in it: this is the call that panics
        // when the crypto provider is left to rustls to infer, and every TLS
        // submission goes through it. The Mailpit tests are plaintext, so
        // nothing else here would catch it.
        assert!(tls_connector().is_ok());
    }

    #[tokio::test]
    async fn plaintext_submission_to_a_remote_host_is_refused() {
        // The check that stops a real password going out in the clear because
        // somebody copied the dev server's settings onto a real account.
        let config = SmtpConfig {
            host: "smtp.example.com".into(),
            port: 25,
            security: SmtpSecurity::Plaintext,
        };
        let auth = EnvPassword::new("KUVERTA_TEST_UNSET");
        let result = submit(&config, "me@example.com", &auth, &message()).await;

        assert!(
            matches!(result, Err(SubmitError::InsecureTransport(_))),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn plaintext_submission_to_localhost_gets_past_the_transport_check() {
        // Reaches the credential lookup, which fails because the variable is
        // unset — proving the transport check let it through rather than that
        // the connection succeeded.
        let config = SmtpConfig {
            host: "127.0.0.1".into(),
            port: 1025,
            security: SmtpSecurity::Plaintext,
        };
        let auth = EnvPassword::new("KUVERTA_TEST_UNSET");
        let result = submit(&config, "dev@kuverta.test", &auth, &message()).await;

        assert!(
            matches!(result, Err(SubmitError::Auth(_))),
            "got {result:?}"
        );
    }
}
