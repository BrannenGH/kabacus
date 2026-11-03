// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use clap::Parser;
use kabacus_server::config::{AuthMode, Config};
use kabacus_server::document::{host::Host, Session};
use kabacus_server::state::AppState;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};

/// kabacus-server: the gateway daemon. Only this process touches
/// the books.
#[derive(Parser, Debug)]
struct Args {
    /// Path to the TOML config file.
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Explicit no-auth mode, for local/test use only.
    #[arg(long)]
    auth: Option<String>,

    /// The first rewrite gate: refuse the rewrite/delete endpoints and tools
    /// for everyone, whatever their roles.
    #[arg(long)]
    no_rewrite: bool,

    /// Deprecated: rewrites are allowed by default now. Still accepted so
    /// existing start commands keep working.
    #[arg(long, hide = true, conflicts_with = "no_rewrite")]
    allow_rewrite: bool,

    /// Override the unix socket path.
    #[arg(long)]
    socket: Option<std::path::PathBuf>,

    /// Override the books: the `.kabacus` file to keep them in.
    #[arg(long)]
    database: Option<std::path::PathBuf>,

    /// Open a portable file in an isolated, recoverable document session.
    #[arg(long, conflicts_with = "document_url")]
    document: Option<std::path::PathBuf>,

    /// Open an existing WebDAV file (for example on Nextcloud).
    #[arg(long)]
    document_url: Option<String>,

    /// Private working copies, outside any synchronized folder.
    #[arg(long)]
    recovery_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Export the configured ledger to a portable file, including its history.
    Export { file: std::path::PathBuf },
    /// Read hledger journals (the main journal and what it includes) into
    /// an empty ledger, then exit.
    Import {
        /// The main journal.
        journal: std::path::PathBuf,
        /// The ledger to import into, made if missing; by default the
        /// server's ledger.
        #[arg(long)]
        ledger: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let args = Args::parse();
    // In a container this is PID 1, which gets no default action for these
    // signals: without handlers, stopping the container hangs.
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;

    let mut config = match &args.config {
        Some(path) => Config::load_from_file(path)?,
        None => Config::default(),
    };
    if args.auth.as_deref() == Some("none") {
        config.auth.mode = AuthMode::None;
    }
    if args.allow_rewrite {
        config.allow_rewrite = true;
    }
    if args.no_rewrite {
        config.allow_rewrite = false;
    }
    if let Some(socket) = args.socket {
        config.socket_path = socket;
    }
    if let Some(database) = args.database {
        config.database = Some(database);
        config.database_url = None;
    }
    let has_document = args.document.is_some() || args.document_url.is_some();
    if !has_document {
        config.resolve_store()?;
    }

    if args.command.is_some() && (args.document.is_some() || args.document_url.is_some()) {
        anyhow::bail!(
            "import and export use --database or the configured database, not a document session"
        );
    }

    if let Some(Command::Export { file }) = &args.command {
        use kabacus_store::BookStore;
        if file.exists() {
            anyhow::bail!("the export destination already exists");
        }
        let store = kabacus_server::state::open_store(&config).await?;
        let ledgers = store.ledgers().await?;
        let ledger = match config.ledger.as_deref() {
            Some(name) => ledgers.iter().find(|l| l.name == name),
            None => ledgers.first(),
        }
        .ok_or_else(|| anyhow::anyhow!("no ledger to export"))?;
        let scratch = tempfile::NamedTempFile::new_in(
            file.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new(".")),
        )?;
        store.export_file(&[ledger.id], scratch.path()).await?;
        scratch.persist_noclobber(file).map_err(|e| e.error)?;
        return Ok(());
    }

    // Importing needs only the store, not the listeners' auth settings.
    if let Some(Command::Import { journal, ledger }) = args.command {
        let store = kabacus_server::state::open_store(&config).await?;
        let ledger = ledger.or(config.ledger.clone());
        let (id, revision, outcome) =
            kabacus_server::import::import_from_files(&store, ledger.as_deref(), &journal).await?;
        println!(
            "imported {} into ledger {id} at revision {revision}",
            journal.display()
        );
        for line in kabacus_server::import::summary(&outcome) {
            println!("  {line}");
        }
        return Ok(());
    }

    if !has_document {
        config.validate()?;
    }

    let session = match (&args.document, &args.document_url) {
        (Some(path), None) => Some(Host::local(path.clone())?),
        (None, Some(url)) => Some(Host::webdav(
            url,
            std::env::var("KABACUS_DOCUMENT_TOKEN").ok(),
        )?),
        _ => None,
    };
    let (app, reloading) = if let Some(host) = session {
        let root = args
            .recovery_dir
            .unwrap_or_else(kabacus_server::document::recovery_root);
        let session = Session::open(config.clone(), host, &root).await?;
        tracing::info!("document recovery: {}", session.recovery_path().display());
        let state = session.state.clone();
        (kabacus_server::document::router(session), state)
    } else {
        let state = Arc::new(AppState::new(config.clone()).await?);
        (kabacus_server::api::router(state.clone()), state)
    };

    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&config.socket_path);
    let listener = tokio::net::UnixListener::bind(&config.socket_path)?;
    // Mode 0600 on the unix socket.
    std::fs::set_permissions(
        &config.socket_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;

    tracing::info!(
        "kabacus-server listening on {}",
        config.socket_path.display()
    );
    // SIGHUP re-reads the account kind rules from the config.
    tokio::spawn(async move {
        while sighup.recv().await.is_some() {
            match reloading.reload_kinds() {
                Ok(()) => tracing::info!("reloaded the account kind rules"),
                Err(e) => tracing::warn!("kept the account kind rules; reload failed: {e}"),
            }
        }
    });

    // Off by default; when configured, the same API is also
    // served over HTTPS for phones and tablets.
    if let Some(remote) = &config.remote {
        let tls = kabacus_server::remote::tls_config(remote)?;
        let tcp = tokio::net::TcpListener::bind(&remote.bind_addr).await?;
        tracing::info!(
            "kabacus-server also listening on https://{}",
            remote.bind_addr
        );
        tokio::spawn(kabacus_server::remote::serve(tcp, tls, app.clone()));
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = sigint.recv() => {}
            }
            tracing::info!("kabacus-server stopping");
        })
        .await?;
    let _ = std::fs::remove_file(&config.socket_path);
    Ok(())
}

/// `RUST_LOG` picks what's logged, as usual; unset, it's `info`: startup,
/// one line per request, warnings and errors. Under systemd (or podman's
/// conmon, for a Quadlet unit) stderr isn't a terminal: if the journal's
/// socket is there (mount `/run/systemd/journal/socket` into the
/// container), log to it directly, so each line keeps its priority and
/// fields. Otherwise plain text on stderr, which still reaches the journal
/// but all at one priority; colors only on a terminal.
fn init_logging() {
    use std::io::IsTerminal;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    let tty = std::io::stderr().is_terminal();
    if !tty {
        // Fails when no journal is listening; then stderr it is.
        if let Ok(journald) = tracing_journald::layer() {
            registry.with(journald).init();
            return;
        }
    }
    registry
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(tty),
        )
        .init();
}
