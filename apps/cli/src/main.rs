//! Development driver for the fuckmail core.
//!
//! Not the product — the product is the triage UI. This exists so the sync
//! path can be exercised end to end before any UI exists, and so the store can
//! be inspected without a SQLite client.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use clap::{Args, Parser, Subcommand, ValueEnum};
use core_accounts::oauth::{OAuth2Config, OAuth2Device};
use core_accounts::{AuthProvider, EnvPassword, KeychainPassword};
use core_proto::{ImapClient, ImapConfig};
use core_store::model::{ImapSecurity, NewAccount, SmtpConfig, SmtpSecurity};
use core_store::{Blobs, Store};

#[derive(Parser)]
#[command(
    name = "fuckmail",
    about = "Read-only mail triage core (development CLI)"
)]
struct Cli {
    /// Where the database and message blobs live.
    #[arg(long, env = "FUCKMAIL_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register an account.
    AddAccount(AddAccount),
    /// Store an app-specific password in the OS keychain.
    SetPassword {
        #[arg(long)]
        email: String,
    },
    /// Authorise an OAuth2 account via the device flow.
    ///
    /// Prints a short code and a URL; the account is usable once you have
    /// entered the code there. Only the refresh token is kept, in the keychain.
    Login {
        #[arg(long)]
        email: String,
    },
    /// Forget a stored OAuth2 login.
    Logout {
        #[arg(long)]
        email: String,
    },
    /// List registered accounts.
    Accounts,
    /// Fetch new mail into the local store.
    Sync {
        /// Defaults to every registered account.
        #[arg(long)]
        email: Option<String>,
        /// Read the password from this environment variable instead of the
        /// keychain. Used by the dev server and CI, where a keychain prompt
        /// would block.
        #[arg(long)]
        password_env: Option<String>,
    },
    /// Show the most recent messages.
    List {
        #[arg(long)]
        email: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Full-text search.
    Search {
        query: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Show each message's category as the rules classified it during sync.
    Triage {
        #[arg(long)]
        email: Option<String>,
        #[arg(short = 'n', long, default_value_t = 40)]
        limit: usize,
        /// Only show this category (e.g. transactional).
        #[arg(long)]
        category: Option<String>,
    },
    /// Configure (or clear) where an account submits outgoing mail.
    ///
    /// Accounts registered before sending existed have no endpoint; this is how
    /// they gain one without being re-created.
    SetSmtp {
        #[arg(long)]
        email: String,
        /// Remove the endpoint, turning sending off for this account.
        #[arg(long, conflicts_with = "smtp_host")]
        clear: bool,
        #[command(flatten)]
        smtp: SmtpArgs,
    },
    /// Summarise what is in the store.
    Status,
}

#[derive(Args)]
struct AddAccount {
    #[arg(long)]
    email: String,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 993)]
    port: u16,
    #[arg(long, value_enum, default_value_t = Security::Tls)]
    security: Security,
    /// Defaults to the email address.
    #[arg(long)]
    username: Option<String>,
    /// How to authenticate. Microsoft 365 requires oauth2; basic auth for IMAP
    /// is disabled there.
    #[arg(long, value_enum, default_value_t = Auth::AppPassword)]
    auth: Auth,
    /// Azure AD application id. Required for --auth oauth2.
    #[arg(long)]
    client_id: Option<String>,
    /// Directory id, or "common" for personal Microsoft accounts.
    #[arg(long, default_value = "common")]
    tenant: String,
    #[command(flatten)]
    smtp: SmtpArgs,
}

/// Submission endpoint. Optional: an account with no `--smtp-host` syncs but
/// cannot send, which is every account registered before send existed.
#[derive(Args, Clone)]
struct SmtpArgs {
    #[arg(long)]
    smtp_host: Option<String>,
    #[arg(long, default_value_t = 465)]
    smtp_port: u16,
    #[arg(long, value_enum, default_value_t = Security::Tls)]
    smtp_security: Security,
}

impl SmtpArgs {
    /// Validates and resolves the endpoint, if one was given.
    fn resolve(&self) -> Result<Option<SmtpConfig>> {
        let Some(host) = self.smtp_host.clone() else {
            return Ok(None);
        };
        let security: SmtpSecurity = self.smtp_security.into();
        if security == SmtpSecurity::Plaintext && !is_loopback(&host) {
            bail!(
                "refusing to configure plaintext SMTP for remote host {host}; \
                 cleartext is only allowed against localhost"
            );
        }
        Ok(Some(SmtpConfig {
            host,
            port: self.smtp_port,
            security,
        }))
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Auth {
    AppPassword,
    Oauth2,
}

impl Auth {
    fn as_str(self) -> &'static str {
        match self {
            Self::AppPassword => "app_password",
            Self::Oauth2 => "oauth2",
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum Security {
    Tls,
    Starttls,
    Plaintext,
}

impl From<Security> for SmtpSecurity {
    fn from(value: Security) -> Self {
        match value {
            Security::Tls => Self::Tls,
            Security::Starttls => Self::StartTls,
            Security::Plaintext => Self::Plaintext,
        }
    }
}

impl From<Security> for ImapSecurity {
    fn from(value: Security) -> Self {
        match value {
            Security::Tls => Self::Tls,
            Security::Starttls => Self::StartTls,
            Security::Plaintext => Self::Plaintext,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;

    let store = Store::open(data_dir.join("fuckmail.db"))
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    let blobs = Blobs::new(data_dir.join("blobs"));

    match cli.command {
        Command::AddAccount(args) => add_account(&store, args),
        Command::SetPassword { email } => set_password(&email),
        Command::Login { email } => login(&store, &email).await,
        Command::Logout { email } => logout(&store, &email),
        Command::Accounts => list_accounts(&store),
        Command::Sync {
            email,
            password_env,
        } => sync(&store, &blobs, email.as_deref(), password_env.as_deref()).await,
        Command::List { email, limit } => list_messages(&store, email.as_deref(), limit),
        Command::Search {
            query,
            email,
            limit,
        } => search(&store, &query, email.as_deref(), limit),
        Command::Triage {
            email,
            limit,
            category,
        } => triage(&store, email.as_deref(), limit, category.as_deref()),
        Command::SetSmtp { email, clear, smtp } => set_smtp(&store, &email, clear, &smtp),
        Command::Status => status(&store, &blobs),
    }
}

fn add_account(store: &Store, args: AddAccount) -> Result<()> {
    if store.account_by_email(&args.email)?.is_some() {
        bail!("account {} already exists", args.email);
    }

    let security: ImapSecurity = args.security.into();
    if security == ImapSecurity::Plaintext && !is_loopback(&args.host) {
        bail!(
            "refusing to configure plaintext IMAP for remote host {}; \
             cleartext is only allowed against localhost",
            args.host
        );
    }

    if args.auth == Auth::Oauth2 && args.client_id.is_none() {
        bail!(
            "--auth oauth2 needs --client-id (the application id from your Azure AD \
             app registration)"
        );
    }

    let smtp = args.smtp.resolve()?;

    let id = store.add_account(&NewAccount {
        label: args.label.unwrap_or_else(|| args.email.clone()),
        email: args.email.clone(),
        imap_host: args.host,
        imap_port: args.port,
        imap_security: security,
        username: args.username.unwrap_or_else(|| args.email.clone()),
        auth_method: args.auth.as_str().into(),
        oauth_client_id: args.client_id,
        oauth_tenant: (args.auth == Auth::Oauth2).then_some(args.tenant),
        smtp,
    })?;

    println!("added account {} (id {id})", args.email);
    match args.auth {
        Auth::AppPassword => println!(
            "store a password with: fuckmail set-password --email {}",
            args.email
        ),
        Auth::Oauth2 => println!("authorise it with: fuckmail login --email {}", args.email),
    }
    Ok(())
}

fn set_smtp(store: &Store, email: &str, clear: bool, args: &SmtpArgs) -> Result<()> {
    let account = store
        .account_by_email(email)?
        .with_context(|| format!("no account {email}"))?;

    let smtp = if clear { None } else { args.resolve()? };
    if smtp.is_none() && !clear {
        bail!("give --smtp-host to configure an endpoint, or --clear to remove one");
    }

    store.set_smtp(account.id, smtp.as_ref())?;
    match &smtp {
        Some(config) => println!(
            "{email} now submits via {}:{} ({})",
            config.host,
            config.port,
            config.security.as_str()
        ),
        None => println!("{email} can no longer send"),
    }
    Ok(())
}

fn set_password(email: &str) -> Result<()> {
    // Read from stdin so the password never lands in shell history or the
    // process table, where a --password flag would put it.
    if std::io::stdin().is_terminal() {
        eprintln!("Reading password from stdin. Paste it and press Ctrl-D:");
    }
    let mut password = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut password)?;
    let password = password.trim_end_matches(['\n', '\r']);

    if password.is_empty() {
        bail!("no password given");
    }

    KeychainPassword::new(email).store(password)?;
    println!("stored password for {email} in the OS keychain");
    Ok(())
}

/// Builds the auth provider an account is configured for.
fn provider_for(
    account: &core_store::model::Account,
    password_env: Option<&str>,
) -> Result<Box<dyn AuthProvider>> {
    // The override exists for the dev server and CI, where reading the keychain
    // would raise a GUI prompt and block.
    if let Some(var) = password_env {
        return Ok(Box::new(EnvPassword::new(var)));
    }

    match account.auth_method.as_str() {
        "oauth2" => Ok(Box::new(OAuth2Device::new(oauth_config(account)?))),
        _ => Ok(Box::new(KeychainPassword::new(&account.email))),
    }
}

fn oauth_config(account: &core_store::model::Account) -> Result<OAuth2Config> {
    let client_id = account
        .oauth_client_id
        .as_deref()
        .with_context(|| format!("account {} has no OAuth client id", account.email))?;
    let tenant = account.oauth_tenant.as_deref().unwrap_or("common");
    Ok(OAuth2Config::microsoft(
        &account.username,
        client_id,
        tenant,
    ))
}

async fn login(store: &Store, email: &str) -> Result<()> {
    let account = store
        .account_by_email(email)?
        .with_context(|| format!("no account {email}"))?;

    if account.auth_method != "oauth2" {
        bail!(
            "account {email} uses {}; `login` is only for oauth2 accounts \
             (use `set-password` instead)",
            account.auth_method
        );
    }

    let device = OAuth2Device::new(oauth_config(&account)?);

    println!("Waiting for authorization…");
    device
        .device_login(|prompt| {
            println!();
            println!("  1. open {}", prompt.verification_uri);
            println!("  2. enter the code: {}", prompt.user_code);
            println!(
                "  (the code is valid for about {} minutes)",
                prompt.expires_in.as_secs() / 60
            );
            println!();
        })
        .await?;

    println!("authorised {email}; the refresh token is in the OS keychain");
    Ok(())
}

fn logout(store: &Store, email: &str) -> Result<()> {
    let account = store
        .account_by_email(email)?
        .with_context(|| format!("no account {email}"))?;
    OAuth2Device::new(oauth_config(&account)?).logout()?;
    println!("forgot the stored login for {email}");
    Ok(())
}

fn list_accounts(store: &Store) -> Result<()> {
    let accounts = store.accounts()?;
    if accounts.is_empty() {
        println!("no accounts yet");
        return Ok(());
    }
    for account in accounts {
        println!(
            "{:>3}  {:<32} {}:{} ({}) as {} [{}]",
            account.id,
            account.email,
            account.imap_host,
            account.imap_port,
            account.imap_security.as_str(),
            account.username,
            account.auth_method,
        );
    }
    Ok(())
}

async fn sync(
    store: &Store,
    blobs: &Blobs,
    email: Option<&str>,
    password_env: Option<&str>,
) -> Result<()> {
    let accounts = match email {
        Some(email) => vec![store
            .account_by_email(email)?
            .with_context(|| format!("no account {email}"))?],
        None => store.accounts()?,
    };

    if accounts.is_empty() {
        bail!("no accounts registered; start with `fuckmail add-account`");
    }

    for account in accounts {
        let auth = provider_for(&account, password_env)?;

        let config = ImapConfig {
            host: account.imap_host.clone(),
            port: account.imap_port,
            security: account.imap_security.clone(),
            username: account.username.clone(),
        };

        tracing::info!(email = %account.email, "connecting");
        let mut client = ImapClient::connect(&config, auth.as_ref())
            .await
            .with_context(|| format!("connecting to {}", account.imap_host))?;

        let report = core_proto::sync_account(&mut client, store, blobs, account.id).await?;
        client.logout().await.ok();

        println!(
            "{}: {} folders synced, {} unchanged | {} new, {} deduplicated, \
             {} flag changes, {} expunged, {} unparseable{}",
            account.email,
            report.folders_synced,
            report.folders_skipped,
            report.inserted,
            report.deduplicated,
            report.flag_updates,
            report.expunged,
            report.unparseable,
            if report.invalidated > 0 {
                format!(
                    ", {} folders rebuilt after UIDVALIDITY change",
                    report.invalidated
                )
            } else {
                String::new()
            },
        );
    }

    Ok(())
}

fn list_messages(store: &Store, email: Option<&str>, limit: usize) -> Result<()> {
    let account = resolve_account(store, email)?;
    for message in store.recent(account, limit)? {
        print_summary(&message);
    }
    Ok(())
}

fn search(store: &Store, query: &str, email: Option<&str>, limit: usize) -> Result<()> {
    let account = resolve_account(store, email)?;
    let hits = store.search(account, query, limit)?;
    if hits.is_empty() {
        println!("no matches for {query:?}");
    }
    for message in hits {
        print_summary(&message);
    }
    Ok(())
}

fn triage(store: &Store, email: Option<&str>, limit: usize, category: Option<&str>) -> Result<()> {
    let account = resolve_account(store, email)?;
    let rows = store.recent_with_category(account, limit)?;

    for row in rows {
        let verdict = row.category.as_deref().unwrap_or("-");
        if let Some(wanted) = category {
            if verdict != wanted {
                continue;
            }
        }

        let sender = row
            .summary
            .from_name
            .as_deref()
            .or(row.summary.from_addr.as_deref())
            .unwrap_or("(unknown)");

        let confidence = row
            .confidence
            .map(|c| format!("{:>3.0}%", c * 100.0))
            .unwrap_or_else(|| "  -".into());

        println!(
            "{:<14} {confidence}  {:<26.26} {}",
            verdict,
            sender,
            row.summary.subject.as_deref().unwrap_or("(no subject)"),
        );
    }
    Ok(())
}

fn status(store: &Store, blobs: &Blobs) -> Result<()> {
    let accounts = store.accounts()?;
    if accounts.is_empty() {
        println!("no accounts yet");
        return Ok(());
    }

    for account in &accounts {
        let messages = store.message_count(account.id)?;
        let locations = store.location_count(account.id)?;
        println!("{}", account.email);
        println!("  messages   {messages}");
        // Locations above messages is the dedup working, not a fault: it means
        // the server showed the same mail in more than one folder.
        println!("  locations  {locations}");

        for folder in store.folders(account.id)? {
            println!(
                "  {:<24} uidvalidity={} uidnext={} modseq={}",
                folder.name,
                folder
                    .uid_validity
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                folder
                    .uid_next
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                folder
                    .highest_modseq
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
            );
        }

        let disagreements = store.disagreements(account.id)?;
        if !disagreements.is_empty() {
            println!("  rules/model disagreements: {}", disagreements.len());
        }
    }

    println!("blobs in {}", blobs.root().display());
    Ok(())
}

fn resolve_account(store: &Store, email: Option<&str>) -> Result<i64> {
    match email {
        Some(email) => Ok(store
            .account_by_email(email)?
            .with_context(|| format!("no account {email}"))?
            .id),
        None => {
            let accounts = store.accounts()?;
            match accounts.len() {
                0 => bail!("no accounts registered"),
                1 => Ok(accounts[0].id),
                _ => bail!("several accounts registered; pass --email"),
            }
        }
    }
}

fn print_summary(message: &core_store::MessageSummary) {
    let when = message
        .date_utc
        .and_then(|ts| Local.timestamp_opt(ts, 0).single())
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "                ".into());

    let sender = message
        .from_name
        .as_deref()
        .or(message.from_addr.as_deref())
        .unwrap_or("(unknown)");

    println!(
        "{when}  {:<28.28} {}{}{}",
        sender,
        message.subject.as_deref().unwrap_or("(no subject)"),
        if message.has_attachments {
            "  [attachment]"
        } else {
            ""
        },
        message
            .list_id
            .as_deref()
            .map(|l| format!("  [list: {l}]"))
            .unwrap_or_default(),
    );
}

/// Cleartext IMAP is only ever acceptable against the local dev server.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn default_data_dir() -> PathBuf {
    // Deliberately simple; a proper platform data dir arrives with the app
    // shell, which has to agree with the Tauri bundle identifier anyway.
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/fuckmail")
}
