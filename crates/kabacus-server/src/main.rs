// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use clap::Parser;
use kabacus_server::config::{AuthMode, Config};
use kabacus_server::state::AppState;
use std::process::ExitCode;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tracing_journald::{Priority, PriorityMappings};
use tracing_subscriber::EnvFilter;

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
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    init_logging();
    std::panic::set_hook(Box::new(|_| tracing::error!("server task panicked")));
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(stage) => {
            tracing::error!(stage, "server failed");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), &'static str> {
    // In a container this is PID 1, which gets no default action for these
    // signals: without handlers, stopping the container hangs.
    let mut sigterm = signal(SignalKind::terminate()).map_err(|_| "register SIGTERM")?;
    let mut sigint = signal(SignalKind::interrupt()).map_err(|_| "register SIGINT")?;
    let mut sighup = signal(SignalKind::hangup()).map_err(|_| "register SIGHUP")?;

    let mut config = match &args.config {
        Some(path) => Config::load_from_file(path).map_err(|_| "read configuration")?,
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
    config.resolve_store().map_err(|_| "resolve book store")?;

    config.validate().map_err(|_| "validate configuration")?;

    let state = Arc::new(
        AppState::new(config.clone())
            .await
            .map_err(|_| "initialize book store or authentication")?,
    );

    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "create socket directory")?;
    }
    let _ = std::fs::remove_file(&config.socket_path);
    let listener =
        tokio::net::UnixListener::bind(&config.socket_path).map_err(|_| "bind local socket")?;
    // Mode 0600 on the unix socket.
    std::fs::set_permissions(
        &config.socket_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .map_err(|_| "set socket permissions")?;

    tracing::info!("local listener ready");
    // SIGHUP re-reads the account kind rules from the config.
    let reloading = state.clone();
    tokio::spawn(async move {
        while sighup.recv().await.is_some() {
            match reloading.reload_kinds() {
                Ok(()) => tracing::info!("reloaded the account kind rules"),
                Err(_) => tracing::warn!("could not reload account kind rules"),
            }
        }
    });
    let app = kabacus_server::api::router(state);

    // Off by default; when configured, the same API is also
    // served over HTTPS for phones and tablets.
    if let Some(remote) = &config.remote {
        let tls = kabacus_server::remote::tls_config(remote).map_err(|_| "configure TLS")?;
        let tcp = tokio::net::TcpListener::bind(&remote.bind_addr)
            .await
            .map_err(|_| "bind HTTPS listener")?;
        tracing::info!("HTTPS listener ready");
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
        .await
        .map_err(|_| "serve local requests")?;
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
    let filter = EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();
    let registry = log_registry(filter);
    let tty = std::io::stderr().is_terminal();
    if !tty {
        // Fails when no journal is listening; then stderr it is.
        if let Ok(journald) = tracing_journald::layer() {
            registry
                .with(journald.with_priority_mappings(PriorityMappings {
                    info: Priority::Informational,
                    debug: Priority::Debug,
                    ..PriorityMappings::new()
                }))
                .init();
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

fn log_registry(
    filter: EnvFilter,
) -> impl tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> {
    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::filter_fn(kabacus_log_target))
        .with(filter)
}

fn kabacus_log_target(metadata: &tracing::Metadata<'_>) -> bool {
    matches!(
        metadata.target().split("::").next(),
        Some("kabacus_server" | "kabacus_service")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use kabacus_testkit::{provider::FakeProvider, spawn_oauth_server};
    use tower::ServiceExt;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;

    async fn capture(level: &str, exercise: impl std::future::Future<Output = ()>) -> String {
        let output = tempfile::NamedTempFile::new().unwrap();
        let subscriber = log_registry(EnvFilter::new(level)).with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(output.reopen().unwrap()),
        );
        exercise.with_subscriber(subscriber).await;
        std::fs::read_to_string(output.path()).unwrap()
    }

    #[tokio::test]
    async fn trace_only_enables_reviewed_log_targets() {
        let logs = capture("trace", async {
            tracing::trace!("server diagnostic");
            tracing::warn!(target: "kabacus_service::simplefin", "service diagnostic");
            tracing::error!(target: "rmcp::transport", "private MCP detail");
            tracing::debug!(target: "hyper::proto", "private HTTP detail");
        })
        .await;
        assert!(logs.contains("server diagnostic"), "{logs}");
        assert!(logs.contains("service diagnostic"), "{logs}");
        assert!(!logs.contains("private"), "{logs}");
    }

    #[tokio::test]
    async fn rejected_credentials_and_permissions_are_safe_at_warn() {
        let provider = FakeProvider::start(Default::default()).await;
        let server = spawn_oauth_server(&provider).await;
        let app = kabacus_server::api::router(server.state.clone());
        let token = provider.mint("Marina Lopez", "cedar-bookkeeping");
        let logs = capture("warn", async {
            for (method, path, authorization, expected) in [
                ("POST", "/mcp", None, 401),
                ("POST", "/mcp", Some("north-harbor-expired"), 401),
                (
                    "DELETE",
                    "/transactions/Marina-Coffee-Roasters",
                    Some(token.as_str()),
                    403,
                ),
                (
                    "GET",
                    "/register/Marina-Coffee-Roasters",
                    Some(token.as_str()),
                    200,
                ),
            ] {
                let mut request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json");
                if let Some(authorization) = authorization {
                    request = request.header("authorization", format!("Bearer {authorization}"));
                }
                let request = request.body(Body::from(r#"{"dry_run":true}"#)).unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), expected);
            }
        })
        .await;
        assert_eq!(logs.lines().count(), 2, "{logs}");
        assert!(
            logs.contains("access denied method=POST route=/mcp status=401"),
            "{logs}"
        );
        assert!(
            logs.contains("access denied method=DELETE route=/transactions/{id} status=403"),
            "{logs}"
        );
        for private in [
            "Marina",
            "cedar-bookkeeping",
            "north-harbor-expired",
            &token,
        ] {
            assert!(!logs.contains(private), "{logs}");
        }
    }
}
