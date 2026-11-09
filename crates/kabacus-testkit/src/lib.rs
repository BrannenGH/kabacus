// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared test fixtures. Not shipped; used only from `[dev-dependencies]`.

pub mod provider;

use kabacus_server::config::{AuthConfig, AuthMode, Config, Database};
use kabacus_server::state::{self, AppState};
use kabacus_store::{BookStore, Who};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A private CA and a `localhost` server certificate it signed.
pub struct Pki {
    pub ca_pem: String,
    pub cert_pem: String,
    pub key_pem: String,
}

pub fn make_pki() -> Pki {
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let server_key = rcgen::KeyPair::generate().unwrap();
    let server = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .signed_by(&server_key, &ca, &ca_key)
        .unwrap();
    Pki {
        ca_pem: ca.pem(),
        cert_pem: server.pem(),
        key_pem: server_key.serialize_pem(),
    }
}

/// A real kabacus-server on synthetic scratch books in a `.kabacus` file
/// of its own, listening on its unix socket. Dropping it stops everything.
pub struct TestServer {
    pub dir: tempfile::TempDir,
    pub socket_path: PathBuf,
    pub state: Arc<AppState>,
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self._task.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Start a server in `--auth=none` mode unless `customize` says otherwise.
pub async fn spawn_server(customize: impl FnOnce(&mut Config)) -> TestServer {
    spawn_server_with_journal("", customize).await
}

/// The scratch books every test server starts from, as a journal, with
/// `extra` appended.
pub fn scratch_journal(extra: &str) -> String {
    format!(
        "commodity USD1000.00\n\
         account assets:checking      ; ynab-account:Everyday Checking\n\
         account expenses:groceries   ; ynab-envelope:Groceries\n\
         account expenses:household\n\
         account equity:opening\n\
         account assets:receivable:jordan\n\
         account liabilities:payable:jordan\n\
         \n\
         2026-01-01 Opening\n    assets:checking  1000.00 USD\n    equity:opening  -1000.00 USD\n\n{extra}"
    )
}

/// Synthetic USD checking, ETH and XMR wallets with full on-chain precision.
pub const WALLETS: &str = include_str!("../fixtures/wallets.journal");

/// Import `journal` into `store`'s served ledger (made if missing), as
/// the server user.
pub async fn import_journal(store: &kabacus_store::AnyStore, ledger: Option<&str>, journal: &str) {
    let main = Path::new("/scratch/main.journal");
    let src =
        kabacus_ledger::MemorySource(BTreeMap::from([(main.to_path_buf(), journal.to_string())]));
    let outcome = kabacus_ledger::import(&src, main).expect("the scratch journal imports");
    let id = state::find_or_create_ledger(store, ledger).await.unwrap();
    let user = store.local_user(state::SERVER_USER).await.unwrap();
    let who = Who {
        user,
        actor: "import".to_string(),
        kind: "import".to_string(),
        summary: "import: main.journal".to_string(),
        details: serde_json::json!({}),
    };
    store.import(id, &outcome.book, &who).await.unwrap();
}

/// [`spawn_server`], with `extra` appended to the books' journal before
/// it is imported: more account declarations or transactions for one
/// test.
pub async fn spawn_server_with_journal(
    extra: &str,
    customize: impl FnOnce(&mut Config),
) -> TestServer {
    let dir = tempfile::tempdir().unwrap();

    // Unix socket paths are limited to ~108 bytes; keep this one short.
    let socket_path =
        std::env::temp_dir().join(format!("khl-kit-{}.sock", uuid::Uuid::new_v4().simple()));
    let mut config = Config {
        store: Database::Sqlite(dir.path().join("books.kabacus")),
        socket_path: socket_path.clone(),
        auth: AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        },
        ..Default::default()
    };
    customize(&mut config);
    config.validate().unwrap();
    // `customize` may have moved the socket.
    let socket_path = config.socket_path.clone();

    let store = state::open_store(&config).await.unwrap();
    import_journal(&store, config.ledger.as_deref(), &scratch_journal(extra)).await;
    let state = Arc::new(AppState::with_store(config, store).await.unwrap());

    let _ = std::fs::remove_file(&socket_path);
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let app = kabacus_server::api::router(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        dir,
        socket_path,
        state,
        _task: task,
    }
}

/// A [`TestServer`] in OAuth mode against `provider`, mapping its
/// `ledger-read` and `ledger-write` roles, with the app's client
/// `kabacus-app`.
pub async fn spawn_oauth_server(provider: &provider::FakeProvider) -> TestServer {
    let issuer = provider.issuer.clone();
    spawn_server(move |cfg| {
        cfg.auth = AuthConfig {
            mode: AuthMode::Oauth,
            issuer: Some(issuer),
            audience: Some("kabacus-server".into()),
            role_map: [
                ("ledger-read".to_string(), "ledger-read".to_string()),
                ("ledger-write".to_string(), "ledger-write".to_string()),
            ]
            .into(),
            ..Default::default()
        };
        cfg.auth.clients.app = Some("kabacus-app".into());
    })
    .await
}

/// A [`TestServer`] that also serves the remote HTTPS listener
/// with a certificate from a private CA, in OAuth mode against a stand-in
/// provider that grants `ledger-read`.
pub struct RemoteServer {
    pub server: TestServer,
    pub url: String,
    pub provider: provider::FakeProvider,
    pub pki: Pki,
    _certs: tempfile::TempDir,
}

pub async fn spawn_remote_server() -> RemoteServer {
    use kabacus_server::config::RemoteConfig;

    let provider = provider::FakeProvider::start(provider::ProviderOptions::default()).await;
    let pki = make_pki();
    let certs = tempfile::tempdir().unwrap();
    let remote = RemoteConfig {
        bind_addr: "127.0.0.1:0".to_string(),
        cert_path: certs.path().join("server.crt"),
        key_path: certs.path().join("server.key"),
    };
    std::fs::write(&remote.cert_path, &pki.cert_pem).unwrap();
    std::fs::write(&remote.key_path, &pki.key_pem).unwrap();

    let issuer = provider.issuer.clone();
    let remote_for_config = remote.clone();
    let server = spawn_server(move |cfg| {
        cfg.auth = AuthConfig {
            mode: AuthMode::Oauth,
            issuer: Some(issuer),
            audience: Some("kabacus-server".to_string()),
            role_map: [("ledger-read".to_string(), "ledger-read".to_string())].into(),
            ..Default::default()
        };
        cfg.remote = Some(remote_for_config);
    })
    .await;

    let tls = kabacus_server::remote::tls_config(&remote).unwrap();
    let tcp = tokio::net::TcpListener::bind(&remote.bind_addr)
        .await
        .unwrap();
    let port = tcp.local_addr().unwrap().port();
    tokio::spawn(kabacus_server::remote::serve(
        tcp,
        tls,
        kabacus_server::api::router(server.state.clone()),
    ));

    RemoteServer {
        server,
        url: format!("https://localhost:{port}"),
        provider,
        pki,
        _certs: certs,
    }
}
