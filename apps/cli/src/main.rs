//! Development driver for the kuverta core.
//!
//! Not the product — the product is the triage UI. This exists so the sync
//! path can be exercised end to end before any UI exists, and so the store can
//! be inspected without a SQLite client.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use clap::{Args, Parser, Subcommand, ValueEnum};
use core_accounts::loopback::OAuth2Loopback;
use core_accounts::oauth::OAuth2Device;
use core_accounts::KeychainPassword;
use core_proto::{ImapClient, ImapConfig};
use core_rpc::session::{google_client_secret, google_config, microsoft_config, provider_for};
use core_store::model::{
    ImapSecurity, NewAccount, NewOperation, OperationKind, SmtpConfig, SmtpSecurity,
};
use core_store::{Blobs, Store};

#[derive(Parser)]
#[command(
    name = "kuverta",
    about = "Read-only mail triage core (development CLI)"
)]
struct Cli {
    /// Where the database and message blobs live.
    #[arg(long, env = "KUVERTA_DATA_DIR", global = true)]
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
    /// Authorise an OAuth2 account.
    ///
    /// Microsoft takes the device flow — a short code and a URL to enter it at.
    /// Google cannot: it does not issue the mail scope to that grant, so it
    /// gets a browser redirect back to this process instead. Either way only
    /// the refresh token is kept, in the keychain.
    Login {
        #[arg(long)]
        email: String,
        /// Google issues one for "Desktop app" OAuth clients and requires it
        /// even from an installed app. Stored in the keychain; only needed the
        /// first time.
        #[arg(long)]
        client_secret: Option<String>,
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
    /// Read a physical address's post, the way `list` reads a mailbox.
    ///
    /// A postal address is an account and Paperless-ngx is its server: the
    /// correspondent wrote it, the title is the subject, the OCR text is the
    /// body. The same classifier files it.
    Paper(Paper),
    /// Connect to an account and report what the server actually supports.
    ///
    /// Run this before the first sync of a real account: it says which
    /// extensions are available, where sent, archived and deleted mail will
    /// go, and — with --measure — how much a first sync will download.
    Check {
        #[arg(long)]
        email: Option<String>,
        /// Also count messages and total bytes per folder. An extra pass over
        /// the mailbox, so it is not the default.
        #[arg(long)]
        measure: bool,
        #[arg(long)]
        password_env: Option<String>,
    },
    /// Stop syncing a folder.
    ///
    /// Takes a folder name, or an RFC 6154 attribute like '\All'. On Gmail,
    /// excluding \All (its "All Mail") is the difference between downloading
    /// your mailbox once and downloading it once per label.
    Exclude {
        /// Folder name, or an attribute such as '\All' or '\Junk'.
        pattern: String,
        #[arg(long)]
        email: Option<String>,
    },
    /// Start syncing a folder again. The next sync fetches it.
    Include {
        pattern: String,
        #[arg(long)]
        email: Option<String>,
    },
    /// Create a folder on the server.
    ///
    /// Needed on servers that ship without an Archive folder — plain Dovecot
    /// setups usually do — where `archive` has nowhere to file to.
    CreateFolder {
        /// Folder name, as the server should see it.
        name: String,
        /// RFC 6154 attribute to mark it with, e.g. \Archive. Ignored by
        /// servers without CREATE-SPECIAL-USE, which then match it by name.
        #[arg(long)]
        r#use: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password_env: Option<String>,
    },
    /// Move a message to the Archive folder.
    Archive(Mutation),
    /// Move a message to the Trash folder.
    ///
    /// Nothing here deletes mail permanently — see `docs/implementation-plan.md`
    /// section 1a.
    Delete(Mutation),
    /// Move a message to a named folder.
    Move {
        #[command(flatten)]
        common: Mutation,
        /// Destination folder, as the server names it.
        #[arg(long)]
        to: String,
    },
    /// Mark a message read.
    Read(Mutation),
    /// Mark a message unread.
    Unread(Mutation),
    /// Cancel the most recent change that has not yet reached the server.
    Undo {
        #[arg(long)]
        email: Option<String>,
    },
    /// Show changes queued but not yet sent.
    Queue {
        #[arg(long)]
        email: Option<String>,
    },
    /// Compose and send a message.
    ///
    /// The body is read from stdin, so it composes with an editor or a
    /// heredoc rather than needing one of its own.
    Send(SendArgs),
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
    /// Run the local model over mail it has not classified yet.
    ///
    /// Records its verdict beside the rules' and never changes what the list
    /// shows. Brief §3.3: run it after sync, not during, and log both.
    Classify {
        #[arg(long)]
        email: Option<String>,
        /// The model to ask. Without it, the one chosen for sorting mail in the
        /// app's settings (llama3.2:3b until one is chosen).
        #[arg(long)]
        model: Option<String>,
        /// Ask the Ollama at this address instead of the provider chosen in settings.
        #[arg(long, env = "OLLAMA_URL")]
        ollama: Option<String>,
        #[arg(short = 'n', long, default_value_t = 100)]
        limit: usize,
    },
    /// Where the rules and the model filed a message differently.
    Disagreements {
        #[arg(long)]
        email: Option<String>,
    },
    /// Senders whose mail can be unsubscribed from, and how.
    ///
    /// Read-only: it lists, and unsubscribes from nothing — that is done in
    /// the window's Cleanup, where each sender is chosen. Mail synced before
    /// the store kept `List-Unsubscribe` has it read back from disk first.
    Unsubscribable {
        #[arg(long)]
        email: Option<String>,
    },
    /// Ask the assistant something about an account's mail, or to do something
    /// with it — the same assistant as the window's, with the same tools.
    Ask {
        #[arg(long)]
        email: Option<String>,
        /// What to ask.
        message: String,
    },
    /// Run the account's tasks now: those that need no model, and with
    /// --model those that ask the model too.
    RunTasks {
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        model: bool,
    },
    /// Inbox mail that needs you soonest, as kuverta's urgency agent sees it.
    ///
    /// The rules judge every recent message at once; with --model the model
    /// chosen for sorting mail (settings → Models) judges each in turn, told
    /// what kuverta checked about it. Verdicts are kept, so each message is
    /// asked about once.
    Urgent {
        #[arg(long)]
        email: Option<String>,
        /// Also ask the model, not only the rules.
        #[arg(long)]
        model: bool,
        /// Show everything down to this urgency (0–3).
        #[arg(long, default_value_t = 2)]
        min: i64,
    },
    /// A message's attachments: listed, or one saved with --save.
    Attachments {
        /// The message, by its id in the store (`kuverta list` shows them).
        #[arg(long)]
        id: i64,
        #[arg(long)]
        email: Option<String>,
        /// Save this attachment (its number in the list).
        #[arg(long)]
        save: Option<usize>,
        /// Where to save it. Defaults to the current directory; an existing
        /// file is never overwritten.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Messages that look like one — what the window offers to delete with it.
    ///
    /// Read-only, for seeing what the likeness rules make of real mail.
    Similar {
        /// The message, by its id in the store (`kuverta list` shows them).
        #[arg(long)]
        id: i64,
        #[arg(long)]
        email: Option<String>,
    },
    /// Score the rules, a prompted model and embeddings against labelled mail.
    ///
    /// Every method is scored on the same held-out half. Generate the input with
    /// `python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/dump-facts.js > labelled.jsonl`.
    Eval {
        /// Facts with a `label`, one JSON object per line.
        facts: PathBuf,
        #[arg(long, default_value = "llama3.2:3b")]
        model: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, env = "OLLAMA_URL", default_value = "http://127.0.0.1:11434")]
        ollama: String,
        /// How many of the nearest filings the embedding classifier consults.
        #[arg(long, default_value_t = 5)]
        k: usize,
        /// Hold out every other message, or every other sender within each
        /// category — the second scores only senders never filed before.
        #[arg(long, value_enum, default_value_t = Split::Alternate)]
        split: Split,
        /// List every message a method got wrong, with its sender and subject.
        #[arg(long)]
        show_wrong: bool,
        #[arg(long)]
        skip_prompt: bool,
        #[arg(long)]
        skip_embeddings: bool,
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
    /// Which OAuth2 provider, for --auth oauth2. The grants differ: Microsoft
    /// uses the device flow, Gmail cannot and needs a loopback redirect.
    #[arg(long, value_enum, default_value_t = OauthProvider::Microsoft)]
    oauth_provider: OauthProvider,
    #[command(flatten)]
    smtp: SmtpArgs,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum OauthProvider {
    Microsoft,
    Google,
}

impl OauthProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Microsoft => "microsoft",
            Self::Google => "google",
        }
    }
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

/// Everything a mailbox mutation needs beyond what it does.
#[derive(Args)]
struct Mutation {
    /// The message: its number from `list`, or its Message-ID.
    message: String,
    #[arg(long)]
    email: Option<String>,
    /// Seconds to hold the change before it may be sent, so `undo` can still
    /// reach it. The change is cancellable for as long as it is queued; this
    /// is the floor, not the whole grace period.
    #[arg(long, default_value_t = 10)]
    undo_window: u64,
}

/// What the user asked for, before it is resolved to a folder or a flag.
enum Intent {
    Archive,
    Trash,
    MoveTo(String),
    Seen(bool),
}

#[derive(Args)]
struct SendArgs {
    /// Which account to send from. Optional when only one is registered.
    #[arg(long)]
    email: Option<String>,
    /// Repeat for several recipients.
    #[arg(long)]
    to: Vec<String>,
    #[arg(long)]
    cc: Vec<String>,
    #[arg(long)]
    bcc: Vec<String>,
    #[arg(long)]
    subject: Option<String>,
    /// Reply to the message with this Message-ID. It must already be in the
    /// local store, so `sync` first.
    #[arg(long, conflicts_with = "forward")]
    reply_to: Option<String>,
    /// Reply to everyone on the original rather than only its author.
    #[arg(long, requires = "reply_to")]
    reply_all: bool,
    /// Forward the message with this Message-ID. Needs at least one --to.
    #[arg(long)]
    forward: Option<String>,
    /// Sign with the account's OpenPGP key (PGP/MIME).
    #[arg(long)]
    sign: bool,
    /// Encrypt with OpenPGP to every recipient and to yourself. Refused with
    /// --bcc, which the encrypted message would disclose.
    #[arg(long)]
    encrypt: bool,
    /// Print the message and its envelope instead of sending it.
    #[arg(long)]
    dry_run: bool,
    /// Do not file a copy in the Sent folder.
    #[arg(long)]
    no_save_to_sent: bool,
    /// Read the password from this environment variable instead of the
    /// keychain, as `sync` does.
    #[arg(long)]
    password_env: Option<String>,
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

/// Where an address's post lives, and which of it is that address's.
#[derive(Args)]
struct Paper {
    /// The Paperless-ngx instance, e.g. http://localhost:8000
    #[arg(long, env = "PAPERLESS_URL", default_value = "http://localhost:8000")]
    url: String,

    /// An API token. Taken from the environment by preference so it never
    /// lands in a shell history the way an argument does.
    #[arg(long, env = "PAPERLESS_TOKEN")]
    token: String,

    /// Only post carrying this tag — one address of several in one instance.
    #[arg(long, group = "selector")]
    tag: Option<String>,

    /// Only post from this correspondent.
    #[arg(long, group = "selector")]
    correspondent: Option<String>,

    /// Only post filed under this storage path.
    #[arg(long, group = "selector")]
    storage_path: Option<String>,

    /// Report what is there instead of listing it.
    #[arg(long)]
    check: bool,

    /// Full-text search, over the OCR'd text.
    #[arg(long)]
    query: Option<String>,

    #[arg(short = 'n', long, default_value_t = 25)]
    limit: usize,
}

impl Paper {
    fn selector(&self) -> core_paper::Selector {
        if let Some(tag) = &self.tag {
            core_paper::Selector::Tag(tag.clone())
        } else if let Some(name) = &self.correspondent {
            core_paper::Selector::Correspondent(name.clone())
        } else if let Some(path) = &self.storage_path {
            core_paper::Selector::StoragePath(path.clone())
        } else {
            core_paper::Selector::Everything
        }
    }
}

/// Post, listed like mail and classified like mail.
async fn paper(args: &Paper) -> Result<()> {
    let client = core_paper::Paperless::new(&args.url, &args.token)
        .with_context(|| format!("{} is not a usable Paperless URL", args.url))?;
    let selector = args.selector();

    if args.check {
        let report = client.check(&selector).await?;
        println!("{}", args.url);
        println!("  documents      {}", report.documents_total);
        println!("  this address   {}", report.documents_matching);
        println!("  tags           {}", join(&report.tags));
        println!("  correspondents {}", join(&report.correspondents));
        for note in &report.notes {
            println!("  ! {note}");
        }
        return Ok(());
    }

    let page = client
        .documents(&selector, 0, args.limit, args.query.as_deref())
        .await?;

    // Without history: the CLI has no store to load corrections from, and a
    // verdict that silently differed from the app's would be worse than one
    // that is plainly the rules alone.
    let classifier = core_rules::Classifier::without_history();

    println!("{} of {} document(s)", page.documents.len(), page.total);
    for document in &page.documents {
        let verdict = classifier.classify(&document.facts());
        let when = document
            .created_utc
            .and_then(|secs| Local.timestamp_opt(secs, 0).single())
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "          ".to_string());

        println!(
            "{:<14} {when}  {:<26} {}",
            verdict.category.as_str(),
            truncate(document.sender().unwrap_or("—"), 26),
            truncate(document.subject(), 58),
        );
    }
    Ok(())
}

fn join(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// Cuts on a character boundary, because correspondents have umlauts in them.
fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let cut: String = value.chars().take(width.saturating_sub(1)).collect();
    format!("{cut}…")
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

    let store = Store::open(data_dir.join("kuverta.db"))
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    let blobs = Blobs::new(data_dir.join("blobs"));

    match cli.command {
        Command::AddAccount(args) => add_account(&store, args),
        Command::SetPassword { email } => set_password(&email),
        Command::Login {
            email,
            client_secret,
        } => login(&store, &email, client_secret).await,
        Command::Logout { email } => logout(&store, &email),
        Command::Accounts => list_accounts(&store),
        Command::Sync {
            email,
            password_env,
        } => sync(&store, &data_dir, email.as_deref(), password_env.as_deref()).await,
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
        Command::Paper(args) => paper(&args).await,
        Command::Check {
            email,
            measure,
            password_env,
        } => check(&store, email.as_deref(), measure, password_env.as_deref()).await,
        Command::Exclude { pattern, email } => {
            set_exclusion(&store, email.as_deref(), &pattern, true)
        }
        Command::Include { pattern, email } => {
            set_exclusion(&store, email.as_deref(), &pattern, false)
        }
        Command::CreateFolder {
            name,
            r#use,
            email,
            password_env,
        } => {
            create_folder(
                &store,
                email.as_deref(),
                &name,
                r#use.as_deref(),
                password_env.as_deref(),
            )
            .await
        }
        Command::Archive(args) => mutate(&store, args, Intent::Archive),
        Command::Delete(args) => mutate(&store, args, Intent::Trash),
        Command::Move { common, to } => mutate(&store, common, Intent::MoveTo(to)),
        Command::Read(args) => mutate(&store, args, Intent::Seen(true)),
        Command::Unread(args) => mutate(&store, args, Intent::Seen(false)),
        Command::Undo { email } => undo(&store, email.as_deref()),
        Command::Queue { email } => show_queue(&store, email.as_deref()),
        Command::Send(args) => send(&store, &data_dir, args).await,
        Command::SetSmtp { email, clear, smtp } => set_smtp(&store, &email, clear, &smtp),
        Command::Classify {
            email,
            model,
            ollama,
            limit,
        } => {
            model_classify(
                &data_dir,
                &store,
                email.as_deref(),
                model.as_deref(),
                ollama.as_deref(),
                limit,
            )
            .await
        }
        Command::Disagreements { email } => list_disagreements(&store, email.as_deref()),
        Command::Ask { email, message } => {
            let account = resolve_account(&store, email.as_deref())?;
            let session = core_rpc::Session::new(&data_dir);
            let turn = session
                .assistant_turn(account, Vec::new(), &message, |event| {
                    println!("  · {}", describe_event(event));
                })
                .await?;
            println!(
                "\n{}\n\n({}{})",
                turn.reply,
                turn.model,
                if turn.local { "" } else { ", hosted" }
            );
            Ok(())
        }
        Command::RunTasks { email, model } => {
            let account = resolve_account(&store, email.as_deref())?;
            let session = core_rpc::Session::new(&data_dir);
            let runs = session.run_tasks(account, model, |_, _| {}).await?;
            for run in &runs {
                println!("{:<30} {}", truncate(&run.name, 30), run.summary_text());
            }
            if runs.is_empty() {
                println!("no tasks on this account");
            }
            Ok(())
        }
        Command::Attachments {
            id,
            email,
            save,
            out,
        } => {
            let account = resolve_account(&store, email.as_deref())?;
            let core = core_rpc::Core::new(store, blobs);
            match save {
                None => {
                    let detail = core.message(account, id)?;
                    if detail.attachments.is_empty() {
                        println!("no attachments");
                    }
                    for a in &detail.attachments {
                        println!(
                            "{:>3}  {:<40} {:<28} {:>9}{}",
                            a.index + 1,
                            truncate(&a.name, 40),
                            a.content_type,
                            a.size,
                            if a.risky {
                                "  (could run something)"
                            } else {
                                ""
                            }
                        );
                    }
                }
                Some(number) => {
                    let found = core.attachment(account, id, number.saturating_sub(1))?;
                    let path = out
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join(&found.view.name);
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .with_context(|| format!("cannot create {}", path.display()))?;
                    std::io::Write::write_all(&mut file, &found.bytes)?;
                    println!("saved {}", path.display());
                }
            }
            Ok(())
        }
        Command::Urgent { email, model, min } => {
            let account = resolve_account(&store, email.as_deref())?;
            let core = core_rpc::Core::new(store, blobs);
            let judged = core.judge_by_rules(account)?;
            if judged > 0 {
                println!("the rules judged {judged} message(s)");
            }
            if model {
                let choice = core.ai_for(core_rpc::Task::Chat)?;
                let jobs = core.urgency_jobs(account, true, 150)?;
                println!("asking {} about {} message(s)…", choice.model, jobs.len());
                let mut failures = 0;
                for job in &jobs {
                    match core_rpc::urgency::ask_model(&choice.provider, &choice.model, job).await {
                        Ok(urgency) => {
                            failures = 0;
                            core.record_urgency(job.id, &urgency)?;
                        }
                        Err(err) => {
                            eprintln!("  {}: {err}", truncate(&job.subject, 40));
                            failures += 1;
                            if failures >= 3 {
                                eprintln!("stopped after three failures in a row");
                                break;
                            }
                        }
                    }
                }
            }
            let rows = core.urgent(account, min, 100)?;
            if rows.is_empty() {
                println!("nothing needs you at urgency {min} or above");
            }
            for r in &rows {
                println!(
                    "{}  {:<6} {:<26} {:<44} {}{}",
                    r.urgency.score,
                    r.urgency.action.as_deref().unwrap_or(""),
                    truncate(&r.row.from, 26),
                    truncate(&r.row.subject, 44),
                    r.urgency.reason,
                    r.urgency
                        .deadline
                        .as_deref()
                        .map(|d| format!(" (by {d})"))
                        .unwrap_or_default()
                );
            }
            Ok(())
        }
        Command::Similar { id, email } => {
            let account = resolve_account(&store, email.as_deref())?;
            let core = core_rpc::Core::new(store, blobs);
            let trash = core.special_folders(account)?.trash;
            let report = core.similar(account, id, trash.as_deref())?;
            for m in &report.matches {
                println!(
                    "{:>6}  {:<28} {:<50} {}",
                    m.id,
                    truncate(&m.from, 28),
                    truncate(&m.subject, 50),
                    m.reason
                );
            }
            println!(
                "\n{} alike; a smart mailbox for them: {} — {:?}",
                report.matches.len(),
                report.suggestion.name,
                report.suggestion.query.rules
            );
            Ok(())
        }
        Command::Unsubscribable { email } => {
            list_unsubscribable(store, blobs, &data_dir, email.as_deref())
        }
        Command::Eval {
            facts,
            model,
            embed_model,
            ollama,
            k,
            split,
            show_wrong,
            skip_prompt,
            skip_embeddings,
        } => {
            let options = EvalOptions {
                k,
                split,
                show_wrong,
                skip_prompt,
                skip_embeddings,
            };
            evaluate_models(&facts, &model, &embed_model, &ollama, &options).await
        }
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
        oauth_provider: (args.auth == Auth::Oauth2).then(|| args.oauth_provider.as_str().into()),
    })?;

    println!("added account {} (id {id})", args.email);
    match args.auth {
        Auth::AppPassword => println!(
            "store a password with: kuverta set-password --email {}",
            args.email
        ),
        Auth::Oauth2 => println!("authorise it with: kuverta login --email {}", args.email),
    }
    Ok(())
}

/// Reports what a server actually offers, before anything depends on it.
async fn check(
    store: &Store,
    email: Option<&str>,
    measure: bool,
    password_env: Option<&str>,
) -> Result<()> {
    let account = match email {
        Some(email) => store
            .account_by_email(email)?
            .with_context(|| format!("no account {email}"))?,
        None => {
            let mut accounts = store.accounts()?;
            match accounts.len() {
                0 => bail!("no accounts registered; start with `kuverta add-account`"),
                1 => accounts.remove(0),
                _ => bail!("several accounts registered; say which with --email"),
            }
        }
    };

    println!("{}", account.email);
    let auth = provider_for(&account, password_env)?;

    println!(
        "  IMAP  {}:{} ({})",
        account.imap_host,
        account.imap_port,
        account.imap_security.as_str()
    );
    let config = ImapConfig {
        host: account.imap_host.clone(),
        port: account.imap_port,
        security: account.imap_security.clone(),
        username: account.username.clone(),
    };
    let mut client = ImapClient::connect(&config, auth.as_ref())
        .await
        .with_context(|| format!("connecting to {}", account.imap_host))?;
    println!("        connected and logged in as {}", account.username);

    let ext = client.extensions().await?;
    println!(
        "        CONDSTORE {}  MOVE {}  UIDPLUS {}  QRESYNC {}  IDLE {}",
        yes_no(ext.condstore),
        yes_no(ext.r#move),
        yes_no(ext.uidplus),
        yes_no(ext.qresync),
        yes_no(ext.idle),
    );
    if !ext.condstore {
        println!("        note: without CONDSTORE every sync rescans every folder");
    }
    if !ext.r#move && !ext.uidplus {
        println!("        note: without MOVE or UIDPLUS, archiving leaves a flagged copy behind");
    }

    let folders = client.folders().await?;
    let selectable: Vec<_> = folders.iter().filter(|f| f.selectable).collect();
    let exclusions = store.folder_exclusions(account.id)?;
    println!("  folders ({})", selectable.len());

    let mut total_messages = 0u64;
    let mut total_bytes = 0u64;
    // What excluding All Mail would save, which is the number that makes the
    // Gmail trade-off concrete rather than theoretical.
    let mut all_mail: Option<(String, u64, u64)> = None;

    for folder in &selectable {
        let special = folder.special_use.as_deref().unwrap_or("");
        let skipped = core_store::folder_is_excluded(
            &exclusions,
            &folder.name,
            folder.special_use.as_deref(),
        );
        let marker = if skipped { "  (excluded)" } else { "" };

        let (count, bytes) = if measure {
            client.examine(&folder.name).await?;
            let sizes = client.uid_sizes(1).await?;
            let bytes: u64 = sizes.iter().map(|(_, size)| *size as u64).sum();
            println!(
                "    {:<32} {:<10} {:>7} messages  {}{marker}",
                folder.name,
                special,
                sizes.len(),
                human_bytes(bytes)
            );
            (sizes.len() as u64, bytes)
        } else {
            let state = client.examine(&folder.name).await?;
            println!(
                "    {:<32} {:<10} {:>7} messages{marker}",
                folder.name, special, state.exists
            );
            (state.exists as u64, 0)
        };

        // Only worth mentioning if there is actually something in it.
        if special == "\\All" && !skipped && count > 0 {
            all_mail = Some((folder.name.clone(), count, bytes));
        }
        if !skipped {
            total_messages += count;
            total_bytes += bytes;
        }
    }

    if !exclusions.is_empty() {
        println!("    not synced: {}", exclusions.join(", "));
    }

    let pairs: Vec<(&str, Option<&str>)> = selectable
        .iter()
        .map(|f| (f.name.as_str(), f.special_use.as_deref()))
        .collect();
    println!("  where mail will go");
    report_target(
        "sent",
        core_proto::client::find_sent(&folders).map(|f| f.name.as_str()),
        "`send` will not be able to file a copy",
    );
    report_target(
        "archived",
        core_proto::client::find_archive(pairs.iter().copied()),
        "`archive` will fail; use `move --to <folder>`",
    );
    report_target(
        "deleted",
        core_proto::client::find_trash(pairs.iter().copied()),
        "`delete` will fail; use `move --to <folder>`",
    );

    println!("  first sync");
    if measure {
        println!(
            "    {total_messages} messages, {} to download",
            human_bytes(total_bytes)
        );
    } else {
        println!("    {total_messages} messages (pass --measure for the download size)");
    }

    if let Some((name, count, bytes)) = all_mail {
        // Gmail's All Mail holds a copy of everything, so a mailbox whose mail
        // averages two labels is fetched twice over. Dedup keeps one row and
        // one body; the bytes still cross the wire.
        println!(
            "    {name} holds a copy of every message ({count}{}), so most of the above \
             is downloaded twice.",
            if measure {
                format!(", {}", human_bytes(bytes))
            } else {
                String::new()
            }
        );
        println!(
            "    `kuverta exclude '\\All' --email {}` syncs it once instead — at the cost \
             that archived mail, which lives only there, drops out of the local store.",
            account.email
        );
    }

    client.logout().await.ok();

    match &account.smtp {
        Some(smtp) => {
            println!(
                "  SMTP  {}:{} ({})",
                smtp.host,
                smtp.port,
                smtp.security.as_str()
            );
            match core_smtp::verify(smtp, &account.username, auth.as_ref()).await {
                Ok(()) => println!("        connected and authenticated"),
                // Not fatal: everything above still holds, and a broken
                // submission endpoint should not hide a working mailbox.
                Err(err) => println!("        FAILED: {err}"),
            }
        }
        None => println!("  SMTP  not configured — this account cannot send"),
    }

    Ok(())
}

fn report_target(what: &str, folder: Option<&str>, consequence: &str) {
    match folder {
        Some(name) => println!("    {what:<10} {name}"),
        None => println!("    {what:<10} (none found) — {consequence}"),
    }
}

fn yes_no(present: bool) -> &'static str {
    if present {
        "yes"
    } else {
        "NO "
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn set_exclusion(store: &Store, email: Option<&str>, pattern: &str, exclude: bool) -> Result<()> {
    let account = resolve_account(store, email)?;

    if exclude {
        if store.exclude_folder(account, pattern)? {
            println!("{pattern} will no longer be synced");
            // Saying this once, here, is the difference between a deliberate
            // trade-off and a surprise later.
            if pattern.eq_ignore_ascii_case("\\All") {
                println!(
                    "  note: archived mail lives only in All Mail, so it will drop out of \
                     the local store — along with any corrections recorded against it"
                );
            }
            println!("  already-synced messages stay until the next sync drops them");
        } else {
            println!("{pattern} was already excluded");
        }
    } else if store.include_folder(account, pattern)? {
        println!("{pattern} will be synced again from the next `kuverta sync`");
    } else {
        println!("{pattern} was not excluded");
    }

    let remaining = store.folder_exclusions(account)?;
    if remaining.is_empty() {
        println!("nothing is excluded");
    } else {
        println!("excluded: {}", remaining.join(", "));
    }
    Ok(())
}

async fn create_folder(
    store: &Store,
    email: Option<&str>,
    name: &str,
    special_use: Option<&str>,
    password_env: Option<&str>,
) -> Result<()> {
    let account_id = resolve_account(store, email)?;
    let account = store
        .accounts()?
        .into_iter()
        .find(|a| a.id == account_id)
        .expect("resolve_account returned an id that is not in the store");

    let auth = provider_for(&account, password_env)?;
    let config = ImapConfig {
        host: account.imap_host.clone(),
        port: account.imap_port,
        security: account.imap_security.clone(),
        username: account.username.clone(),
    };

    let mut client = ImapClient::connect(&config, auth.as_ref())
        .await
        .with_context(|| format!("connecting to {}", account.imap_host))?;
    let marked = client.create_folder(name, special_use).await?;
    client.logout().await.ok();

    print!("created {name}");
    match (special_use, marked) {
        (Some(attribute), true) => println!(" as {attribute}"),
        (Some(attribute), false) => println!(
            " (the server does not support CREATE-SPECIAL-USE, so it is not marked \
             {attribute}; it will be found by name instead)"
        ),
        (None, _) => println!(),
    }
    println!("  run `kuverta sync` to pick it up");
    Ok(())
}

/// Queues a mailbox mutation.
///
/// Nothing is sent here. The change is recorded, the undo window starts, and
/// the next `sync` is what puts it on the server — which is also what makes
/// `undo` meaningful rather than a race.
fn mutate(store: &Store, args: Mutation, intent: Intent) -> Result<()> {
    let account = resolve_account(store, args.email.as_deref())?;
    let message = resolve_message(store, account, &args.message)?;
    let folders = store.folders(account)?;

    let pairs = folder_pairs(&folders);
    let kind = match &intent {
        Intent::Archive => OperationKind::Move {
            target_folder: core_proto::client::find_archive(pairs.iter().copied())
                .map(str::to_string)
                .context(
                    "this account has no Archive folder. Create one with \
                     `kuverta create-folder Archive --use '\\Archive'`, or file this \
                     message somewhere that exists with `move --to <folder>`",
                )?,
        },
        Intent::Trash => OperationKind::Move {
            target_folder: core_proto::client::find_trash(pairs.iter().copied())
                .map(str::to_string)
                .context("this account has no Trash folder; use `move --to <folder>`")?,
        },
        Intent::MoveTo(folder) => OperationKind::Move {
            target_folder: folder.clone(),
        },
        Intent::Seen(set) => OperationKind::Flag {
            flag: "\\Seen".into(),
            set: *set,
        },
    };

    let target = match &kind {
        OperationKind::Move { target_folder } => Some(target_folder.as_str()),
        OperationKind::Flag { .. } => None,
    };
    let (location, folder) = pick_location(store, &folders, message.id, target)?;

    let op = store.enqueue_operation(&NewOperation {
        account_id: account,
        message_id: message.id,
        kind: kind.clone(),
        source_folder_id: folder.id,
        source_uid: location.uid,
        source_uid_validity: folder.uid_validity,
        // What the executor will verify before it touches anything. Without a
        // Message-ID there is nothing to check the UID against, and the
        // operation is only as safe as the UID being untouched.
        expect_message_id: message.rfc822_message_id.clone(),
        execute_after: now_utc() + args.undo_window as i64,
    })?;

    let what = match &kind {
        OperationKind::Move { target_folder } => format!("move to {target_folder}"),
        OperationKind::Flag { set: true, .. } => "mark read".into(),
        OperationKind::Flag { .. } => "mark unread".into(),
    };
    println!(
        "queued: {what} — {} (from {})",
        message.subject.as_deref().unwrap_or("(no subject)"),
        folder.name
    );
    if args.undo_window > 0 {
        println!("  `kuverta undo` cancels it; `kuverta sync` sends it (op {op})");
    } else {
        println!("  sends at the next `kuverta sync` (op {op})");
    }
    Ok(())
}

fn undo(store: &Store, email: Option<&str>) -> Result<()> {
    let account = resolve_account(store, email)?;
    match store.cancel_latest_operation(account)? {
        Some(op) => {
            let what = match &op.kind {
                OperationKind::Move { target_folder } => format!("move to {target_folder}"),
                OperationKind::Flag { flag, set: true } => format!("set {flag}"),
                OperationKind::Flag { flag, .. } => format!("clear {flag}"),
            };
            println!("undone: {what} (op {})", op.id);
        }
        // Deliberately not an error: "nothing to undo" is an answer.
        None => println!("nothing queued to undo"),
    }
    Ok(())
}

fn show_queue(store: &Store, email: Option<&str>) -> Result<()> {
    let account = resolve_account(store, email)?;
    let pending = store.pending_operations(account)?;
    if pending.is_empty() {
        println!("nothing queued");
        return Ok(());
    }

    let now = now_utc();
    for op in &pending {
        let what = match &op.kind {
            OperationKind::Move { target_folder } => format!("move to {target_folder}"),
            OperationKind::Flag { flag, set: true } => format!("set {flag}"),
            OperationKind::Flag { flag, .. } => format!("clear {flag}"),
        };
        let when = if op.execute_after > now {
            format!("holds for {}s", op.execute_after - now)
        } else {
            "ready".into()
        };
        println!("  {:>4}  {what:<28} {when}", op.id);
        if let Some(error) = &op.last_error {
            println!("        last attempt: {error}");
        }
    }
    println!("{} queued; `kuverta sync` sends them", pending.len());
    Ok(())
}

/// The shape `core-proto`'s folder resolvers take, from what sync recorded.
fn folder_pairs(folders: &[core_store::model::Folder]) -> Vec<(&str, Option<&str>)> {
    folders
        .iter()
        .map(|f| (f.name.as_str(), f.special_use.as_deref()))
        .collect()
}

/// Chooses which copy of a message to act on.
///
/// One message can be in several folders — that is the whole point of the
/// dedup design, and on Gmail it is routine. Archiving means getting it out of
/// the inbox, so INBOX wins when there is a choice; a copy already sitting in
/// the destination is never the one to move.
fn pick_location(
    store: &Store,
    folders: &[core_store::model::Folder],
    message_id: i64,
    target: Option<&str>,
) -> Result<(core_store::model::Location, core_store::model::Folder)> {
    let by_id = |id: i64| folders.iter().find(|f| f.id == id).cloned();

    let mut candidates: Vec<_> = store
        .locations_of(message_id)?
        .into_iter()
        .filter_map(|location| by_id(location.folder_id).map(|folder| (location, folder)))
        .filter(|(_, folder)| target != Some(folder.name.as_str()))
        .collect();

    if candidates.is_empty() {
        match target {
            Some(folder) => bail!("that message is already in {folder}"),
            None => bail!("that message is not in any folder this account has synced"),
        }
    }

    candidates.sort_by_key(|(_, folder)| folder.name != "INBOX");
    Ok(candidates.remove(0))
}

/// Accepts either the number `list` prints or an RFC 5322 Message-ID.
///
/// Both because neither alone is usable: the number is short enough to type
/// but means nothing outside this store, and the Message-ID is stable but
/// nobody wants to type one.
fn resolve_message(
    store: &Store,
    account: i64,
    handle: &str,
) -> Result<core_store::model::StoredMessage> {
    if let Ok(id) = handle.parse::<i64>() {
        if let Some(message) = store.message_by_id(account, id)? {
            return Ok(message);
        }
        bail!("no message {id} in this account; `kuverta list` shows the numbers");
    }

    store
        .message_by_rfc822_id(account, handle)?
        .with_context(|| format!("no message <{handle}> in the store; sync first"))
}

fn now_utc() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn send(store: &Store, data_dir: &std::path::Path, args: SendArgs) -> Result<()> {
    let account = match &args.email {
        Some(email) => store
            .account_by_email(email)?
            .with_context(|| format!("no account {email}"))?,
        None => {
            let mut accounts = store.accounts()?;
            match accounts.len() {
                0 => bail!("no accounts registered; start with `kuverta add-account`"),
                1 => accounts.remove(0),
                _ => bail!("several accounts registered; say which with --email"),
            }
        }
    };

    // The flags take a Message-ID because that is what a person can copy out
    // of `list`; the core addresses messages by row id, so resolve here.
    let resolve =
        |handle: &String| -> Result<i64> { Ok(resolve_message(store, account.id, handle)?.id) };

    let input = core_rpc::DraftInput {
        to: args.to,
        cc: args.cc,
        bcc: args.bcc,
        subject: args.subject.unwrap_or_default(),
        body: read_body()?,
        reply_to: args.reply_to.as_ref().map(resolve).transpose()?,
        reply_all: args.reply_all,
        forward: args.forward.as_ref().map(resolve).transpose()?,
        sign: args.sign,
        encrypt: args.encrypt,
    };

    let session = core_rpc::Session::new(data_dir).with_password_env(args.password_env.clone());

    if args.dry_run {
        let preview = session.preview(&account.email, &input)?;
        println!("-- envelope --");
        println!("MAIL FROM: <{}>", preview.from);
        for recipient in &preview.recipients {
            println!("RCPT TO:   <{recipient}>");
        }
        println!("-- message --");
        print!("{}", preview.rfc822);
        return Ok(());
    }

    let sent = session
        .send(&account.email, &input, !args.no_save_to_sent)
        .await?;
    println!(
        "sent to {} recipient(s) as <{}>",
        sent.recipients.len(),
        sent.message_id
    );
    match (&sent.filed_in, &sent.filing_error) {
        (Some(folder), _) => println!("filed a copy in {folder}"),
        (_, Some(err)) => eprintln!("warning: sent, but could not file a copy in Sent: {err}"),
        _ => {}
    }
    Ok(())
}

fn read_body() -> Result<String> {
    if std::io::stdin().is_terminal() {
        eprintln!("Reading the message body from stdin. Type it and press Ctrl-D:");
    }
    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)?;
    Ok(body)
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

async fn login(store: &Store, email: &str, client_secret: Option<String>) -> Result<()> {
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

    if account.oauth_provider.as_deref() == Some("google") {
        if let Some(secret) = client_secret {
            google_client_secret(email).store(&secret)?;
        }

        let client = OAuth2Loopback::new(google_config(&account)?);
        println!("Waiting for authorization…");
        client
            .login(|prompt| {
                println!();
                println!("  1. open this in a browser signed in as {email}:");
                println!();
                println!("     {}", prompt.authorization_url);
                println!();
                println!(
                    "  2. approve it. Google will redirect to {}, which is this",
                    prompt.redirect_uri
                );
                println!("     process listening on your own machine — nothing leaves it.");
                println!();
            })
            .await?;
    } else {
        let device = OAuth2Device::new(microsoft_config(&account)?);

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
    }

    println!("authorised {email}; the refresh token is in the OS keychain");
    Ok(())
}

fn logout(store: &Store, email: &str) -> Result<()> {
    let account = store
        .account_by_email(email)?
        .with_context(|| format!("no account {email}"))?;
    // Whichever grant it used, the refresh token is in the same place.
    if account.oauth_provider.as_deref() == Some("google") {
        OAuth2Loopback::new(google_config(&account)?).logout()?;
    } else {
        OAuth2Device::new(microsoft_config(&account)?).logout()?;
    }
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
    data_dir: &std::path::Path,
    email: Option<&str>,
    password_env: Option<&str>,
) -> Result<()> {
    // The orchestration lives in `core-rpc` so the window runs the same code
    // rather than a second copy of it that drifts.
    let session =
        core_rpc::Session::new(data_dir).with_password_env(password_env.map(str::to_string));

    let emails: Vec<String> = match email {
        Some(email) => vec![
            store
                .account_by_email(email)?
                .with_context(|| format!("no account {email}"))?
                .email,
        ],
        None => store.accounts()?.into_iter().map(|a| a.email).collect(),
    };
    if emails.is_empty() {
        bail!("no accounts registered; start with `kuverta add-account`");
    }

    for email in emails {
        tracing::info!(%email, "connecting");
        let summary = session.sync_account(&email).await?;
        report_sync(&summary);
    }
    Ok(())
}

fn report_sync(s: &core_rpc::SyncSummary) {
    let changes = s.changes_sent + s.changes_obsolete + s.changes_refused + s.changes_retryable;
    if changes > 0 {
        println!(
            "{}: {} change(s) sent{}{}{}",
            s.email,
            s.changes_sent,
            option(s.changes_obsolete, "no longer applied"),
            option(s.changes_refused, "refused (see `kuverta queue`)"),
            option(s.changes_retryable, "will be retried"),
        );
    }
    if s.non_atomic_moves > 0 {
        eprintln!(
            "warning: {} move(s) left a copy behind — this server supports neither \
             MOVE nor UIDPLUS",
            s.non_atomic_moves
        );
    }

    println!(
        "{}: {} folders synced, {} unchanged | {} new, {} deduplicated, \
         {} flag changes, {} expunged, {} unparseable{}{}",
        s.email,
        s.folders_synced,
        s.folders_skipped,
        s.inserted,
        s.deduplicated,
        s.flag_updates,
        s.expunged,
        s.unparseable,
        option(s.deleted, "dropped"),
        if s.folders_excluded > 0 {
            format!(" | {} folder(s) not synced", s.folders_excluded)
        } else {
            String::new()
        },
    );
    if s.invalidated > 0 {
        println!(
            "  {} folder(s) rebuilt after a UIDVALIDITY change",
            s.invalidated
        );
    }
}

/// `", 3 dropped"`, or nothing at all when the count is zero.
fn option(count: usize, label: &str) -> String {
    if count > 0 {
        format!(", {count} {label}")
    } else {
        String::new()
    }
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
    let hits = store.search_typed(account, query, limit)?;
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

    // The number is the handle the mutation commands take.
    println!(
        "{:>5}  {when}  {:<28.28} {}{}{}",
        message.id,
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
    // Shared with the app, so both agree on where an instance's data lives.
    core_accounts::default_data_dir()
}

// -- the local model ----------------------------------------------------------

async fn model_classify(
    data_dir: &std::path::Path,
    store: &Store,
    email: Option<&str>,
    model: Option<&str>,
    ollama_url: Option<&str>,
    limit: usize,
) -> Result<()> {
    let account = resolve_account(store, email)?;
    let core = core_rpc::Core::open(data_dir)
        .with_context(|| format!("opening the store in {}", data_dir.display()))?;
    // An address on the command line is an Ollama asked on purpose; otherwise
    // the provider and model chosen in settings, which a flag's model overrides.
    let (server, model) = match ollama_url {
        Some(url) => (
            core_ai::Provider::Ollama(core_ai::Ollama::new(url)?),
            model
                .map(str::to_string)
                .unwrap_or_else(|| core_rpc::Task::Chat.default_model()),
        ),
        None => {
            let choice = core.ai_for(core_rpc::Task::Chat)?;
            if !choice.local {
                println!(
                    "sorting with {} at {}: senders, subjects and the start of each message go there",
                    choice.model, choice.provider_label
                );
            }
            (
                choice.provider,
                model.map(str::to_string).unwrap_or(choice.model),
            )
        }
    };
    let classifier = core_ai::PromptClassifier::new(model.clone());

    println!("asking {model} about up to {limit} message(s) it has not seen");
    let pass = core
        .model_pass(account, &server, &classifier, limit)
        .await?;

    println!(
        "{} of {} classified, {} with no usable answer, {} failed{}",
        pass.classified,
        pass.waiting,
        pass.unparseable,
        pass.failed,
        pass.mean_latency_ms
            .map(|ms| format!(", {ms} ms each on average"))
            .unwrap_or_default()
    );
    if let Some(reason) = pass.stopped {
        println!("stopped after three failures in a row: {reason}");
    }
    let disagreements = store.disagreements(account)?.len();
    println!("{disagreements} disagreement(s) with the rules — `kuverta disagreements` lists them");
    Ok(())
}

fn list_disagreements(store: &Store, email: Option<&str>) -> Result<()> {
    let account = resolve_account(store, email)?;
    let rows = store.disagreements(account)?;
    if rows.is_empty() {
        println!(
            "no disagreements: either they agree, or the model has not run (`kuverta classify`)"
        );
        return Ok(());
    }
    println!("{:<14}   {:<14} subject", "rules", "model");
    for row in &rows {
        println!(
            "{:<14} → {:<14} {}",
            row.rules_category,
            row.model_category,
            truncate(row.subject.as_deref().unwrap_or("(no subject)"), 60)
        );
    }
    println!("\n{} message(s)", rows.len());
    Ok(())
}

fn describe_event(event: &core_rpc::AssistantEvent) -> String {
    use core_rpc::AssistantEvent as E;
    match event {
        E::Looked { what } | E::Failed { what } => what.clone(),
        E::Changed { what, .. } => format!("{what} (queued, undoable)"),
        E::Draft {
            to, subject, body, ..
        } => format!("drafted a reply to {to}: {subject}\n{body}"),
        E::TaskCreated {
            name,
            what,
            rules,
            matching,
            ..
        } => format!(
            "created the task {name}: {what} — for mail where {rules} ({matching} match now)"
        ),
        E::Message {
            id,
            subject,
            from,
            attachments,
            note,
            ..
        } => {
            let mut line = format!("found message {id}: {subject} — {from}");
            if let Some(note) = note {
                line.push_str(&format!("\n    {note}"));
            }
            for a in attachments {
                line.push_str(&format!(
                    "\n    attachment {}: {} (kuverta attachments --id {id} --save {})",
                    a.index + 1,
                    a.name,
                    a.index + 1
                ));
            }
            line
        }
    }
}

fn list_unsubscribable(
    store: Store,
    blobs: Blobs,
    data_dir: &std::path::Path,
    email: Option<&str>,
) -> Result<()> {
    let account = resolve_account(&store, email)?;
    let address = store
        .accounts()?
        .into_iter()
        .find(|a| a.id == account)
        .map(|a| a.email)
        .context("the account has gone")?;
    let session = core_rpc::Session::new(data_dir);
    while session.backfill_headers(&address, 2_000)? > 0 {}

    let core = core_rpc::Core::new(store, blobs);
    let senders = core.unsubscribe_senders(account)?;
    if senders.is_empty() {
        println!("no mail here says how to unsubscribe");
        return Ok(());
    }
    println!("{:>5} {:>6}  {:<12} sender", "mail", "unread", "how");
    for sender in &senders {
        let how = match sender.method {
            core_rpc::UnsubscribeMethod::OneClick { .. } => "one click",
            core_rpc::UnsubscribeMethod::Mailto { .. } => "by mail",
            core_rpc::UnsubscribeMethod::Browser { .. } => "web page",
        };
        let done = match &sender.last_attempt {
            Some(attempt) if attempt.state == "done" => "  (unsubscribed)",
            Some(attempt) if attempt.state == "opened" => "  (page opened)",
            Some(_) => "  (last attempt failed)",
            None => "",
        };
        println!(
            "{:>5} {:>6}  {:<12} {} — {}{done}",
            sender.messages,
            sender.unread,
            how,
            truncate(&sender.name, 30),
            sender
                .list_id
                .as_deref()
                .or(sender.address.as_deref())
                .unwrap_or(""),
        );
    }
    let counts = core.cleanup_counts(account)?;
    println!(
        "\n{} sender(s); {} bulk message(s) still in the Inbox",
        senders.len(),
        counts.bulk_in_inbox
    );
    Ok(())
}

/// Scores the rules, a prompted model and embeddings on the same labelled mail.
///
/// The gate for brief stage 5 — "you can say whether it beats the rules, with
/// numbers" — and the embeddings-against-prompting spike plan §4 asks for before
/// either is committed to.
async fn evaluate_models(
    path: &std::path::Path,
    model: &str,
    embed_model: &str,
    ollama_url: &str,
    options: &EvalOptions,
) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut rows: Vec<(LabelledFacts, core_rules::Category)> = Vec::new();
    let mut unlabelled = 0usize;
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: LabelledFacts = serde_json::from_str(line)
            .with_context(|| format!("line {} of {}", n + 1, path.display()))?;
        match row.label.as_deref().and_then(core_rules::Category::parse) {
            Some(label) => rows.push((row, label)),
            None => unlabelled += 1,
        }
    }
    if rows.len() < 4 {
        bail!(
            "{} holds {} labelled message(s); generate some with\n  \
             python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/dump-facts.js > labelled.jsonl",
            path.display(),
            rows.len()
        );
    }

    // Every method is scored on the same held-out messages, so none is marked on
    // mail it has already seen. *How* they are held out is the choice that
    // matters, and neither answer is the whole truth:
    //
    // - alternate: every other message. Most scored messages then have a
    //   sibling from the same sender among the filings — which is what a mailbox
    //   looks like once it has been used for a while, and on a generated corpus
    //   whose senders repeat templates, an upper bound for anything that learns
    //   from filings.
    // - sender: every other sender within each category is held out whole, so
    //   each scored message comes from a sender never filed before. The cold
    //   start, and the harder test for anything that learns.
    //
    // Neither changes anything for the rules or for prompting, which learn
    // nothing from the filings. Only embeddings are flattered by the first.
    type Row = (LabelledFacts, core_rules::Category);
    let (train, test): (Vec<&Row>, Vec<&Row>) = match options.split {
        Split::Alternate => (
            rows.iter().step_by(2).collect(),
            rows.iter().skip(1).step_by(2).collect(),
        ),
        Split::Sender => {
            let mut senders: std::collections::BTreeMap<&str, std::collections::BTreeSet<String>> =
                std::collections::BTreeMap::new();
            for (row, label) in &rows {
                senders
                    .entry(label.as_str())
                    .or_default()
                    .insert(row.sender());
            }
            // Within each category, so every category keeps senders to learn
            // from wherever it has more than one.
            let held_out: std::collections::HashSet<String> = senders
                .values()
                .flat_map(|names| names.iter().skip(1).step_by(2).cloned())
                .collect();
            rows.iter()
                .partition(|(row, _)| !held_out.contains(&row.sender()))
        }
    };
    let truth: Vec<core_rules::Category> = test.iter().map(|(_, label)| *label).collect();

    println!(
        "{} labelled message(s): {} to learn from, {} to score on{}",
        rows.len(),
        train.len(),
        test.len(),
        if unlabelled > 0 {
            format!(" ({unlabelled} unlabelled, skipped)")
        } else {
            String::new()
        }
    );

    println!("held out: {}", options.split.describe());

    let rules = core_rules::Classifier::without_history();
    let predicted: Vec<Option<core_rules::Category>> = test
        .iter()
        .map(|(row, _)| Some(rules.classify(&row.facts()).category))
        .collect();
    score("rules, no corrections", &truth, &predicted, None);
    if options.show_wrong {
        print_wrong(&test, &predicted);
    }

    let ollama = core_ai::Ollama::new(ollama_url)?;

    // Kept per message, not just scored, so the combined classifier below can
    // be tried at every threshold without asking the model a second time.
    let mut prompted: Option<(Vec<Option<core_rules::Category>>, Vec<i64>)> = None;
    let mut embedded_nearest: Option<(Vec<Option<core_ai::Nearest>>, Vec<i64>)> = None;

    if !options.skip_prompt {
        let classifier = core_ai::PromptClassifier::new(model);
        // One call first, untimed. The first request loads the model, and a
        // twenty-five second cold start averaged into a hundred warm answers
        // would say nothing true about either.
        classifier
            .classify(&ollama, &test[0].0.facts())
            .await
            .with_context(|| {
                format!("asking {model} at {ollama_url} — is Ollama running with it pulled?")
            })?;

        let mut predicted = Vec::with_capacity(test.len());
        let mut latency = Vec::with_capacity(test.len());
        for (row, _) in &test {
            let verdict = classifier.classify(&ollama, &row.facts()).await?;
            predicted.push(verdict.category);
            latency.push(verdict.latency_ms);
        }
        score(
            &format!("prompting {model}"),
            &truth,
            &predicted,
            Some(&latency),
        );
        if options.show_wrong {
            print_wrong(&test, &predicted);
        }
        prompted = Some((predicted, latency));
    }

    if !options.skip_embeddings {
        let mut neighbours = core_ai::Neighbours::new(options.k);
        for batch in train.chunks(16) {
            let inputs: Vec<String> = batch
                .iter()
                .map(|(row, _)| core_ai::embedding_input(embed_model, &row.facts()))
                .collect();
            let embedded = ollama.embed(embed_model, &inputs).await.with_context(|| {
                format!("embedding with {embed_model} — is it pulled? `ollama pull {embed_model}`")
            })?;
            for (vector, (_, label)) in embedded.vectors.into_iter().zip(batch) {
                neighbours.add(vector, *label);
            }
        }

        let mut predicted = Vec::with_capacity(test.len());
        let mut nearest = Vec::with_capacity(test.len());
        let mut latency = Vec::with_capacity(test.len());
        for batch in test.chunks(16) {
            let inputs: Vec<String> = batch
                .iter()
                .map(|(row, _)| core_ai::embedding_input(embed_model, &row.facts()))
                .collect();
            let embedded = ollama.embed(embed_model, &inputs).await?;
            let each = embedded.latency_ms / batch.len().max(1) as i64;
            for vector in &embedded.vectors {
                let found = neighbours.nearest(vector);
                predicted.push(found.map(|found| found.category));
                nearest.push(found);
                latency.push(each);
            }
        }
        score(
            &format!(
                "embeddings {embed_model}, {} nearest of {} filings",
                options.k,
                neighbours.len()
            ),
            &truth,
            &predicted,
            Some(&latency),
        );
        embedded_nearest = Some((nearest, latency));
    }

    if let (Some((prompted, prompt_ms)), Some((nearest, embed_ms))) = (&prompted, &embedded_nearest)
    {
        combined(&truth, prompted, prompt_ms, nearest, embed_ms);
    }

    Ok(())
}

/// The nearest filings when they are close and agree, the model otherwise.
///
/// Tried at every threshold from answers already in hand, so choosing one is a
/// matter of reading a table rather than guessing. Every message pays for its
/// embedding; the ones the filings cannot answer pay for the model as well.
fn combined(
    truth: &[core_rules::Category],
    prompted: &[Option<core_rules::Category>],
    prompt_ms: &[i64],
    nearest: &[Option<core_ai::Nearest>],
    embed_ms: &[i64],
) {
    let total = truth.len().max(1);
    println!();
    println!("combined: nearest filings when close and agreed (share ≥ 0.8), the model otherwise");
    println!(
        "  {:>12} {:>9} {:>11} {:>10}",
        "similarity", "accuracy", "by filings", "mean ms"
    );

    for threshold in [0.60f32, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, f32::INFINITY] {
        let policy = core_ai::Hybrid {
            min_similarity: threshold,
            min_share: 0.8,
        };
        let (mut correct, mut by_filings, mut spent) = (0usize, 0usize, 0i64);

        for (i, expected) in truth.iter().enumerate() {
            let (answer, cost) = match nearest[i].filter(|found| policy.accepts(found)) {
                Some(found) => {
                    by_filings += 1;
                    (Some(found.category), embed_ms[i])
                }
                None => (prompted[i], embed_ms[i] + prompt_ms[i]),
            };
            if answer == Some(*expected) {
                correct += 1;
            }
            spent += cost;
        }

        let label = if threshold.is_finite() {
            format!("≥ {threshold:.2}")
        } else {
            "model only".to_string()
        };
        println!(
            "  {label:>12} {:>8.1}% {:>10.0}% {:>10}",
            100.0 * correct as f64 / total as f64,
            100.0 * by_filings as f64 / total as f64,
            spent / total as i64
        );
    }
}

fn score(
    name: &str,
    truth: &[core_rules::Category],
    predicted: &[Option<core_rules::Category>],
    latency: Option<&[i64]>,
) {
    let total = truth.len();
    let correct = truth
        .iter()
        .zip(predicted)
        .filter(|(expected, got)| **got == Some(**expected))
        .count();
    let unanswered = predicted.iter().filter(|got| got.is_none()).count();

    println!();
    println!("{name}");
    println!(
        "  accuracy  {:>5.1}%  ({correct}/{total}){}",
        100.0 * correct as f64 / total.max(1) as f64,
        if unanswered > 0 {
            format!(", {unanswered} with no usable answer")
        } else {
            String::new()
        }
    );

    if let Some(latency) = latency.filter(|l| !l.is_empty()) {
        let mut sorted = latency.to_vec();
        sorted.sort_unstable();
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        println!("  latency   p50 {} ms, p95 {} ms", at(0.5), at(0.95));
    }

    for category in core_rules::Category::ALL {
        let expected = truth.iter().filter(|t| **t == category).count();
        if expected == 0 {
            continue;
        }
        let hit = truth
            .iter()
            .zip(predicted)
            .filter(|(t, got)| **t == category && **got == Some(category))
            .count();
        println!("  {:<14} {hit:>3}/{expected:<3}", category.as_str());
    }

    let mut confusions: std::collections::HashMap<
        (core_rules::Category, Option<core_rules::Category>),
        usize,
    > = std::collections::HashMap::new();
    for (expected, got) in truth.iter().zip(predicted) {
        if *got != Some(*expected) {
            *confusions.entry((*expected, *got)).or_default() += 1;
        }
    }
    let mut worst: Vec<_> = confusions.into_iter().collect();
    worst.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0 .0.as_str().cmp(b.0 .0.as_str()))
    });
    for ((expected, got), count) in worst.into_iter().take(3) {
        println!(
            "  wrong ×{count:<3} {} read as {}",
            expected.as_str(),
            got.map(|c| c.as_str()).unwrap_or("nothing")
        );
    }
}

/// Every scored message a method filed differently from its label.
///
/// A count says how often a method is wrong; this says whether it is wrong in a
/// way that matters. Three misses on invoices and three on a message that sits
/// on the line between two categories are the same number and not the same
/// result — brief §3.3's "record where they disagree and which was right".
fn print_wrong(
    test: &[&(LabelledFacts, core_rules::Category)],
    predicted: &[Option<core_rules::Category>],
) {
    for ((row, expected), got) in test.iter().map(|pair| (&pair.0, pair.1)).zip(predicted) {
        if *got != Some(expected) {
            println!(
                "    {:<13} → {:<13} {:<22} {}",
                expected.as_str(),
                got.map(|category| category.as_str()).unwrap_or("nothing"),
                truncate(
                    row.from_name
                        .as_deref()
                        .or(row.from_addr.as_deref())
                        .unwrap_or("?"),
                    22
                ),
                truncate(row.subject.as_deref().unwrap_or("(no subject)"), 50)
            );
        }
    }
}

/// How `eval` chooses the messages it scores on.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Split {
    /// Every other message.
    Alternate,
    /// Every other sender within each category, held out whole.
    Sender,
}

impl Split {
    fn describe(self) -> &'static str {
        match self {
            Self::Alternate => {
                "every other message (most have a same-sender sibling among the filings)"
            }
            Self::Sender => "every other sender per category (no scored sender was ever filed)",
        }
    }
}

struct EvalOptions {
    k: usize,
    split: Split,
    show_wrong: bool,
    skip_prompt: bool,
    skip_embeddings: bool,
}

/// One line of `dump-facts.js` output: the facts a classifier reads, and the
/// answer it is scored against, kept apart.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelledFacts {
    from_addr: Option<String>,
    from_name: Option<String>,
    subject: Option<String>,
    list_id: Option<String>,
    list_unsubscribe: Option<String>,
    precedence: Option<String>,
    auto_submitted: Option<String>,
    in_reply_to: Option<String>,
    #[serde(default)]
    has_attachments: bool,
    #[serde(default)]
    recipient_count: usize,
    snippet: Option<String>,
    label: Option<String>,
}

impl LabelledFacts {
    /// Who sent it, for holding senders out of training whole.
    fn sender(&self) -> String {
        self.from_addr
            .clone()
            .or_else(|| self.from_name.clone())
            .unwrap_or_default()
            .to_lowercase()
    }

    fn facts(&self) -> core_rules::MessageFacts<'_> {
        core_rules::MessageFacts {
            from_addr: self.from_addr.as_deref(),
            from_name: self.from_name.as_deref(),
            subject: self.subject.as_deref(),
            list_id: self.list_id.as_deref(),
            list_unsubscribe: self.list_unsubscribe.as_deref(),
            precedence: self.precedence.as_deref(),
            auto_submitted: self.auto_submitted.as_deref(),
            in_reply_to: self.in_reply_to.as_deref(),
            has_attachments: self.has_attachments,
            recipient_count: self.recipient_count,
            snippet: self.snippet.as_deref(),
        }
    }
}
