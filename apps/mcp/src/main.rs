//! `fuckmail-mcp` — the mailbox over MCP, on stdio.
//!
//!     fuckmail-mcp                   # read-only
//!     fuckmail-mcp --allow-writes    # also file, archive, trash, mark read
//!
//! Stdout is the protocol and nothing else. Every log line goes to stderr,
//! because a single stray `println!` would be a malformed message to the client
//! and a very confusing afternoon.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use fuckmail_mcp::{Config, Server};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser)]
#[command(
    name = "fuckmail-mcp",
    about = "The fuckmail mailbox as an MCP server on stdio"
)]
struct Args {
    /// The store to open. The same one the window uses, unless told otherwise.
    #[arg(long, env = "FUCKMAIL_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Offer the tools that change anything. Off by default.
    #[arg(long)]
    allow_writes: bool,

    /// Seconds an assistant's changes are held before they may reach the server.
    #[arg(long, default_value_t = 300)]
    undo_window: i64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(core_rpc::default_data_dir);
    let core = core_rpc::Core::open(&data_dir)
        .with_context(|| format!("could not open the store at {}", data_dir.display()))?;

    let server = Server::new(
        core,
        Config {
            allow_writes: args.allow_writes,
            undo_window_secs: args.undo_window.max(0),
            data_dir: Some(data_dir.clone()),
        },
    );
    tracing::info!(store = %data_dir.display(), writes = args.allow_writes, "serving");

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line).await {
            stdout.write_all(reply.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
