// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use kabacus_server::automation::Gateway;
use kabacus_server::config::{AuthMode, Config};
use kabacus_server::document::{host::Host, Session};
use kabacus_store::BookStore;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "File-based Kabacus automation and bundled MCP")]
struct Args {
    /// A local document. Connect to the desktop's socket if it already has a session.
    #[arg(long, conflicts_with_all = ["url", "socket"])]
    file: Option<PathBuf>,
    /// An existing HTTPS WebDAV document; bearer token from KABACUS_DOCUMENT_TOKEN.
    #[arg(long, conflicts_with = "socket")]
    url: Option<String>,
    /// Use an existing local or hosted gateway's Unix socket.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// TOML config for account-kind rules and rewrite policy.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Private recovery directory; never a synchronized folder.
    #[arg(long)]
    recovery_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new local .kabacus file without replacing an existing file.
    Create {
        #[arg(long, default_value = "Household")]
        name: String,
    },
    /// Run the bundled MCP server over newline-delimited JSON on stdio.
    Mcp,
    /// Call any existing MCP tool; mutation tools default to a dry run.
    Call {
        tool: String,
        #[arg(default_value = "{}")]
        arguments: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if let Command::Create { name } = &args.command {
        if args.url.is_some() || args.socket.is_some() {
            bail!("create requires --file");
        }
        let file = args.file.as_ref().context("create requires --file")?;
        let _host = Host::local(file.clone())?;
        if file.exists() {
            bail!("the document already exists");
        }
        let store = kabacus_store::SqliteStore::in_memory()?;
        let user = store.local_user("Kabacus").await?;
        let ledger = store.create_ledger(name, user).await?;
        let scratch = tempfile::NamedTempFile::new_in(file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")))?;
        store.export_file(&[ledger], scratch.path()).await?;
        scratch.as_file().sync_all()?;
        scratch.persist_noclobber(file).map_err(|e| e.error)?;
        return Ok(());
    }
    let gateway = if let Some(socket) = &args.socket {
        Gateway::socket(socket)?
    } else {
        let host = match (&args.file, &args.url) {
            (Some(file), None) => Host::local(file.clone())?,
            (None, Some(url)) => Host::webdav(url, std::env::var("KABACUS_DOCUMENT_TOKEN").ok())?,
            _ => bail!("choose --file, --url or --socket"),
        };
        let mut config = match &args.config {
            Some(path) => Config::load_from_file(path)?,
            None => Config::default(),
        };
        // Explicit headless file mode, without any network listener.
        config.auth.mode = AuthMode::None;
        config.remote = None;
        let root = args.recovery_dir.clone().unwrap_or_else(kabacus_server::document::recovery_root);
        let session = Session::open(config, host, &root).await?;
        eprintln!("Recovery: {}", session.recovery_path().display());
        Gateway::Embedded(kabacus_server::document::router(session))
    };
    match args.command {
        Command::Mcp => gateway.stdio().await?,
        Command::Call { tool, arguments } => {
            let result = gateway.call(&tool, serde_json::from_str(&arguments)?).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result.get("error").is_some() || result.pointer("/result/isError").and_then(serde_json::Value::as_bool) == Some(true) {
                std::process::exit(1);
            }
        }
        Command::Create { .. } => unreachable!(),
    }
    Ok(())
}
