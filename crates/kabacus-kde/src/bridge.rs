// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cxx-qt bridge: exposes [`crate::views`] and [`crate::writes`] to
//! QML as the `Ledger` element. Data crosses as JSON strings. Screen loads
//! (`load`), `refresh` and writes (`startPreview`, `startCommit`) run on
//! the tokio runtime and come back through cxx-qt's `Threading`
//! (`loaded`, `revision`, `previewed`, `committed`); so does waiting for
//! the browser sign-in. A write's dry run takes seconds on big books, and
//! the form previews as you type: blocking on it froze typing. Everything
//! else blocks the GUI thread while the user waits on it: `editDraft`,
//! the connection test, saving settings, starting sign-in, signing out,
//! and learning the auth mode at startup (up to `AUTH_TIMEOUT`).

// QML invokables take one argument per form field.
#![allow(clippy::too_many_arguments)]

use crate::auth::{Auth, AuthStatus};
use crate::loads::Loads;
use crate::model;
use crate::secrets::{SecretServiceStore, SecretStore};
use crate::settings::{self, ConnectionKind, Settings};
use crate::views;
use crate::writes;
use kabacus_client::KabacusClient;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, error)]
        /// The raw error behind `error`, for its Details view.
        #[qproperty(QString, error_details, cxx_name = "errorDetails")]
        /// The sign-in: `{status, username, issuer}`,
        /// with `signing_in` while the browser is open and `error` if the
        /// last attempt failed. `status` is `off`, `signed_out`,
        /// `signed_in`, `expired`, or `unknown` (server unreachable).
        #[qproperty(QString, auth_json, cxx_name = "authJson")]
        /// Bumped when a refresh succeeds (one follows every successful
        /// write) and when the session or server changes; pages reload
        /// their data when it changes.
        #[qproperty(i32, revision)]
        /// A refresh is under way.
        #[qproperty(bool, refreshing)]
        /// A commit (`startCommit`) is under way.
        #[qproperty(bool, saving)]
        type Ledger = super::LedgerRust;

        #[qinvokable]
        #[cxx_name = "canonicalBooksPath"]
        fn canonical_books_path(self: &Ledger, path: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "localBooksInfo"]
        fn local_books_info(self: &Ledger) -> QString;
        #[qinvokable]
        #[cxx_name = "openBooks"]
        fn open_books(self: Pin<&mut Ledger>, path: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "newBooks"]
        fn new_books(
            self: Pin<&mut Ledger>,
            path: &QString,
            name: &QString,
            currency: &QString,
            account: &QString,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "saveCopy"]
        fn save_copy(self: &Ledger, path: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "workspaceJson"]
        fn workspace_json(self: &Ledger) -> QString;
        #[qinvokable]
        #[cxx_name = "rememberWindow"]
        fn remember_window(self: &Ledger, state: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "forgetWindow"]
        fn forget_window(self: &Ledger, id: &QString);
        #[qinvokable]
        #[cxx_name = "savePreferences"]
        fn save_preferences(
            self: Pin<&mut Ledger>,
            reopen: bool,
            automation: bool,
            port: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "agentActivity"]
        fn agent_activity(self: &Ledger) -> QString;
        #[qinvokable]
        #[cxx_name = "commandCatalog"]
        fn command_catalog(self: &Ledger) -> QString;

        /// A screen's data: `view` with its `args` (a JSON object).
        /// Returns `{"pending": bool, "reply": …}` at once, `reply` being
        /// the last one for this view and args (null if none, and possibly
        /// from before the current revision). When `pending`, `loaded`
        /// brings the current one, unless a refresh brings a newer one
        /// first (then `revision` changes instead). Replies are
        /// `{"ok": …}` or `{"error": …}`.
        #[qinvokable]
        fn load(self: Pin<&mut Ledger>, view: &QString, args: &QString) -> QString;

        /// `key` is `view|args`, as passed to `load`.
        #[qsignal]
        fn loaded(self: Pin<&mut Ledger>, key: QString, reply: QString);

        /// Dry-run a draft (`writes::Draft` as JSON) without blocking.
        /// Nothing is written. Returns a ticket at once; `previewed`
        /// brings `{"ok": Preview}` or `{"error": …}` with it. Tickets
        /// only grow, so a form keeps the answer to its latest ask.
        #[qinvokable]
        #[cxx_name = "startPreview"]
        fn start_preview(self: Pin<&mut Ledger>, draft: &QString) -> i32;

        #[qsignal]
        fn previewed(self: Pin<&mut Ledger>, ticket: i32, reply: QString);

        /// The preview's Save: commit the draft against the preview's
        /// `head`, without blocking. `saving` holds until `committed`
        /// brings `{"ok": Saved}` or `{"error": …}`. Ignored while
        /// one is under way: a second press mustn't write twice.
        #[qinvokable]
        #[cxx_name = "startCommit"]
        fn start_commit(self: Pin<&mut Ledger>, draft: &QString, head: &QString);

        #[qsignal]
        fn committed(self: Pin<&mut Ledger>, reply: QString);

        /// Check the server and preload the main screens; bumps
        /// `revision` once they're in, or sets `error`. Called while one
        /// is under way, it runs again after that one; a result for a
        /// server or sign-in that has since changed is dropped.
        #[qinvokable]
        fn refresh(self: Pin<&mut Ledger>);

        /// Refresh if the books changed since the last refresh: a hand
        /// edit, MCP or another device. Cheap: one `/version`.
        #[qinvokable]
        fn check(self: Pin<&mut Ledger>);

        /// The editor's fields for transaction `id`, as JSON, or
        /// `{"error": …}` if it can't be edited here.
        #[qinvokable]
        #[cxx_name = "editDraft"]
        fn edit_draft(self: &Ledger, id: &QString) -> QString;

        #[qinvokable]
        fn today(self: &Ledger) -> QString;

        /// What's wrong with a typed amount, as one sentence;
        /// empty when it's fine. Pure: no I/O, so it runs on every key.
        #[qinvokable]
        #[cxx_name = "checkAmount"]
        fn check_amount(self: &Ledger, text: &QString, places: i32, positive: bool) -> QString;

        /// The connection settings as JSON, plus the default
        /// socket as `defaultSocket`.
        #[qinvokable]
        #[cxx_name = "settingsJson"]
        fn settings_json(self: &Ledger) -> QString;

        /// The connected server's version, exact commit and where its
        /// source is, as JSON: `{"server_version", "commit", "source_url"}`
        /// or `{"error": …}`.
        #[qinvokable]
        #[cxx_name = "aboutJson"]
        fn about_json(self: &Ledger) -> QString;

        /// Try the given settings without saving them. Returns JSON:
        /// `{"status": …, "message": …}` or `{"error": …}`.
        #[qinvokable]
        #[cxx_name = "testConnection"]
        fn test_connection(
            self: &Ledger,
            kind: &QString,
            socket_path: &QString,
            server_url: &QString,
            private_ca: &QString,
        ) -> QString;

        /// Save the settings and reconnect. Returns an error message, or
        /// an empty string once saved.
        #[qinvokable]
        #[cxx_name = "saveSettings"]
        fn save_settings(
            self: Pin<&mut Ledger>,
            kind: &QString,
            socket_path: &QString,
            server_url: &QString,
            private_ca: &QString,
        ) -> QString;

        /// Start signing in through the system browser. Returns
        /// `{"url": …}` for QML to open with `Qt.openUrlExternally`, or
        /// `{"error": …}`. `authJson` changes once the browser comes back.
        #[qinvokable]
        #[cxx_name = "beginSignIn"]
        fn begin_sign_in(self: Pin<&mut Ledger>) -> QString;

        /// Stop waiting for the browser.
        #[qinvokable]
        #[cxx_name = "cancelSignIn"]
        fn cancel_sign_in(self: Pin<&mut Ledger>);

        /// Sign out: revoke at the provider and clear the secret store.
        /// Returns an error message, or an empty string.
        #[qinvokable]
        #[cxx_name = "signOut"]
        fn sign_out(self: Pin<&mut Ledger>) -> QString;
    }

    impl cxx_qt::Threading for Ledger {}
}

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

/// How long startup waits to learn the server's auth mode.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

pub struct LedgerRust {
    settings: Settings,
    control: Option<kabacus_client::control::ControlChannel>,
    window_id: OnceLock<String>,
    client: KabacusClient,
    error: QString,
    error_details: QString,
    auth_json: QString,
    revision: i32,
    refreshing: bool,
    saving: bool,
    /// The last ticket `startPreview` gave out.
    preview_ticket: i32,
    /// `load`'s cache and the loads under way. (`refresh` is guarded by
    /// `sign_in_generation` instead.)
    loads: Loads,
    /// `refresh` was called while one was under way: run it again after.
    refresh_again: bool,
    /// The server's `books` as of the last refresh (`check` compares).
    books_seen: Option<String>,
    /// `None` until the server has said how it authenticates.
    auth: Option<Auth>,
    store: Arc<dyn SecretStore>,
    /// The browser sign-in under way, if any.
    sign_in_task: Option<tokio::task::AbortHandle>,
    /// Bumped whenever a sign-in starts, is cancelled or ends in sign-out,
    /// or the server changes, so a late sign-in or refresh result from
    /// before is ignored.
    sign_in_generation: u64,
}

/// A client for `settings`, or, if they can't make one (say, the private
/// CA file is gone), the default local socket, so the app still opens and
/// the settings page can fix them.
fn connect(settings: &Settings) -> (KabacusClient, Option<String>) {
    match settings.connect() {
        Ok(client) => (client, None),
        Err(e) => (
            Settings::default().connect().expect("local"),
            Some(e.to_string()),
        ),
    }
}

/// Learn how the server at `settings` authenticates and resume a stored
/// session; `None` if it can't be reached right now.
fn connect_auth(
    settings: &Settings,
    base: &KabacusClient,
    store: &Arc<dyn SecretStore>,
) -> Option<Auth> {
    runtime().block_on(connect_auth_async(settings, base, store))
}

async fn connect_auth_async(
    settings: &Settings,
    base: &KabacusClient,
    store: &Arc<dyn SecretStore>,
) -> Option<Auth> {
    let server = settings.server_id();
    let connecting = Auth::connect(base, &server, store.clone());
    match tokio::time::timeout(AUTH_TIMEOUT, connecting).await {
        Ok(Ok(auth)) => Some(auth),
        Ok(Err(e)) => {
            log::warn!("can't set up sign-in: {e}");
            None
        }
        Err(_) => None,
    }
}

impl LedgerRust {
    /// The server's address as the user knows it.
    fn server(&self) -> String {
        let id = self.settings.server_id();
        id.strip_prefix("unix:").unwrap_or(&id).to_string()
    }

    /// Build the client from the settings and the current sign-in.
    fn rebuild_client(&mut self) {
        let (base, _) = connect(&self.settings);
        self.client = match &self.auth {
            Some(auth) => auth.client(base),
            None => base,
        };
    }

    fn auth_view(&self, error: Option<&str>) -> QString {
        let mut value = match &self.auth {
            Some(auth) => serde_json::to_value(auth.view()).unwrap_or_default(),
            None => serde_json::json!({
                "status": "unknown", "username": "", "issuer": "", "warning": ""
            }),
        };
        value["signing_in"] = self.sign_in_task.is_some().into();
        if let Some(error) = error {
            value["error"] = error.into();
        }
        json(&value)
    }
}

impl Default for LedgerRust {
    fn default() -> Self {
        let settings = Settings::for_app();
        let (base, problem) = connect(&settings);
        let store: Arc<dyn SecretStore> = Arc::new(SecretServiceStore);
        let has_books = settings.kind == ConnectionKind::Remote
            || !settings.books_path.is_empty()
            || !settings.socket_path.is_empty();
        let auth = if problem.is_none() && has_books {
            connect_auth(&settings, &base, &store)
        } else {
            None
        };
        let mut this = LedgerRust {
            settings,
            control: None,
            window_id: OnceLock::new(),
            client: base,
            error: QString::from(problem.unwrap_or_default().as_str()),
            error_details: QString::default(),
            auth_json: QString::default(),
            revision: 0,
            refreshing: false,
            saving: false,
            preview_ticket: 0,
            loads: Loads::default(),
            refresh_again: false,
            books_seen: None,
            auth,
            store,
            sign_in_task: None,
            sign_in_generation: 0,
        };
        this.rebuild_client();
        this.auth_json = this.auth_view(None);
        this
    }
}

impl qobject::Ledger {
    /// Recompute `authJson`.
    fn publish_auth(mut self: Pin<&mut Self>, error: Option<&str>) {
        let view = self.rust().auth_view(error);
        self.as_mut().set_auth_json(view);
    }

    /// Learn the server's auth mode if that failed before (it was
    /// unreachable), and reconnect with it.
    fn ensure_auth(mut self: Pin<&mut Self>) {
        if self.rust().settings.kind == ConnectionKind::Local
            && self.rust().settings.books_path.is_empty()
            && self.rust().settings.socket_path.is_empty()
        {
            return;
        }
        if self.rust().auth.is_some() {
            return;
        }
        let auth = {
            let this = self.rust();
            let (base, problem) = connect(&this.settings);
            if problem.is_some() {
                return;
            }
            connect_auth(&this.settings, &base, &this.store)
        };
        if auth.is_some() {
            let mut this = self.as_mut().rust_mut();
            this.auth = auth;
            this.rebuild_client();
            self.publish_auth(None);
        }
    }

    /// After a failed request: show "Sign in again" if the session ended.
    fn note_error(mut self: Pin<&mut Self>, error: &model::ModelError) {
        let changed = match self.as_mut().rust_mut().auth.as_mut() {
            Some(auth) => auth.note_error(error),
            None => false,
        };
        if changed {
            self.publish_auth(None);
        }
    }

    /// Check the server, learning its auth mode first if that failed
    /// before, and preload the screens most visited. Never blocks.
    fn refresh(mut self: Pin<&mut Self>) {
        if self.rust().settings.kind == ConnectionKind::Local
            && self.rust().settings.books_path.is_empty()
            && self.rust().settings.socket_path.is_empty()
        {
            return;
        }
        if self.rust().refreshing {
            self.as_mut().rust_mut().refresh_again = true;
            return;
        }
        self.as_mut().set_refreshing(true);
        let this = self.rust();
        let needs_auth = this.auth.is_none();
        let (settings, store, client) = (
            this.settings.clone(),
            this.store.clone(),
            this.client.clone(),
        );
        let generation = this.sign_in_generation;
        let thread = self.qt_thread();
        runtime().spawn(async move {
            let auth = match needs_auth {
                true => match connect(&settings) {
                    (base, None) => connect_auth_async(&settings, &base, &store).await,
                    _ => None,
                },
                false => None,
            };
            let client = match &auth {
                Some(auth) => auth.client(connect(&settings).0),
                None => client,
            };
            // Before the views, so a change landing during them shows up
            // as a later `books` and `check` refreshes again.
            let books = client.version().await.ok().and_then(|v| v.books);
            let (a, b, c, d, e) = tokio::join!(
                views::load(&client, "overview", PRELOAD_ARGS),
                views::load(&client, "accounts", PRELOAD_ARGS),
                views::load(&client, "funds", PRELOAD_ARGS),
                views::load(&client, "cards", PRELOAD_ARGS),
                views::load(&client, "picker", PRELOAD_ARGS),
            );
            let results = [
                ("overview", a),
                ("accounts", b),
                ("funds", c),
                ("cards", d),
                ("picker", e),
            ];
            let _ = thread.queue(move |mut ledger: Pin<&mut qobject::Ledger>| {
                ledger.as_mut().set_refreshing(false);
                // Otherwise the server or sign-in changed meanwhile: this
                // answer is for the old one.
                if ledger.rust().sign_in_generation == generation {
                    if let Some(auth) = auth {
                        {
                            let mut this = ledger.as_mut().rust_mut();
                            this.auth = Some(auth);
                            this.rebuild_client();
                        }
                        ledger.as_mut().publish_auth(None);
                    }
                    ledger.as_mut().refreshed(results, books);
                }
                // A write, a pull or a server change asked meanwhile: its
                // answer must come from after it.
                if std::mem::take(&mut ledger.as_mut().rust_mut().refresh_again) {
                    ledger.refresh();
                }
            });
        });
    }

    /// `refresh`'s results. The server answered if any view did: those
    /// are cached and the revision moves on, and a view that failed shows
    /// its own problem when its page asks again. Only if none did is the
    /// server unreachable (`error`).
    fn refreshed(
        mut self: Pin<&mut Self>,
        results: [(&str, Result<serde_json::Value, model::ModelError>); 5],
        books: Option<String>,
    ) {
        if results.iter().all(|(_, r)| r.is_err()) {
            let e = results[0].1.as_ref().expect_err("all failed");
            self.as_mut().note_error(e);
            let signed_out = matches!(
                self.rust().auth.as_ref().map(|a| a.view().status),
                Some(AuthStatus::SignedOut | AuthStatus::Expired)
            );
            // The sign-in banner says it better than an error would.
            let message = if signed_out {
                String::new()
            } else {
                log::warn!("{e}");
                e.user_message(&self.rust().server())
            };
            let details = QString::from(e.to_string().as_str());
            self.as_mut().set_error_details(details);
            self.as_mut().set_error(QString::from(message.as_str()));
            return;
        }
        let next = self.rust().revision + 1;
        {
            let mut this = self.as_mut().rust_mut();
            this.books_seen = books;
            for (view, result) in results {
                if result.is_ok() {
                    let key = format!("{view}|{PRELOAD_ARGS}");
                    this.loads.preloaded(key, next, reply(result).to_string());
                }
            }
        }
        self.as_mut().set_error(QString::default());
        self.as_mut().set_revision(next);
    }

    fn check(self: Pin<&mut Self>) {
        if self.rust().refreshing {
            return;
        }
        let client = self.rust().client.clone();
        let thread = self.qt_thread();
        runtime().spawn(async move {
            // Unreachable: the offline banner's retry takes over.
            let Ok(version) = client.version().await else {
                return;
            };
            let _ = thread.queue(move |ledger: Pin<&mut qobject::Ledger>| {
                if version.books.is_some() && version.books != ledger.rust().books_seen {
                    ledger.refresh();
                }
            });
        });
    }

    /// Forget every reply (signed out, or another server) and have the
    /// pages reload.
    fn forget(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().loads.forget();
        let next = self.rust().revision + 1;
        self.set_revision(next);
    }

    fn load(mut self: Pin<&mut Self>, view: &QString, args: &QString) -> QString {
        let key = format!("{view}|{args}");
        let revision = self.rust().revision;
        let start = self.as_mut().rust_mut().loads.start(&key, revision);
        if let Some(epoch) = start.fetch {
            let client = self.rust().client.clone();
            let (view, args) = (view.to_string(), args.to_string());
            let thread = self.qt_thread();
            runtime().spawn(async move {
                let result = views::load(&client, &view, &args).await;
                let _ = thread.queue(move |ledger: Pin<&mut qobject::Ledger>| {
                    ledger.deliver(key, revision, epoch, result)
                });
            });
        }
        let reply = start.cached.unwrap_or_else(|| "null".to_string());
        let pending = start.pending;
        QString::from(format!(r#"{{"pending":{pending},"reply":{reply}}}"#).as_str())
    }

    fn deliver(
        mut self: Pin<&mut Self>,
        key: String,
        revision: i32,
        epoch: u64,
        result: Result<serde_json::Value, model::ModelError>,
    ) {
        let ok = result.is_ok();
        let current = self.rust().revision;
        let server = self.rust().server();
        let reply = reply(result.as_ref().map_err(|e| e.user_message(&server))).to_string();
        let shown = self
            .as_mut()
            .rust_mut()
            .loads
            .finish(&key, revision, epoch, current, ok, reply);
        if let Some(reply) = shown {
            if let Err(e) = &result {
                self.as_mut().note_error(e);
            }
            self.loaded(QString::from(key.as_str()), QString::from(reply.as_str()));
        }
    }

    fn edit_draft(&self, id: &QString) -> QString {
        if let Ok(tx) = runtime().block_on(model::find_transaction(
            &self.rust().client,
            &id.to_string(),
        )) {
            if tx.postings.len() != 2
                || tx
                    .postings
                    .windows(2)
                    .any(|p| p[0].amount.commodity != p[1].amount.commodity)
            {
                let postings: Vec<_> = tx
                    .postings
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        serde_json::json!({ "account": p.account, "amount": p.amount.exact,
                        "commodity": p.amount.commodity, "original": i })
                    })
                    .collect();
                return json(&serde_json::json!({ "date": tx.date, "payee": tx.payee,
                    "memo": tx.memo, "postings": postings }));
            }
        }
        match runtime().block_on(model::edit_draft(&self.rust().client, &id.to_string())) {
            Ok(d) => json(&serde_json::json!({
                "date": d.date, "payee": d.payee, "memo": d.memo,
                "from": d.from_account, "to": d.to_account, "amount": d.amount,
            })),
            Err(e) => json(&serde_json::json!({ "error": e.user_message(&self.rust().server()) })),
        }
    }

    fn check_amount(&self, text: &QString, places: i32, positive: bool) -> QString {
        let places = u32::try_from(places).unwrap_or(0);
        QString::from(writes::amount_problem(&text.to_string(), places, positive).as_str())
    }

    fn today(&self) -> QString {
        QString::from(chrono::Local::now().format("%Y-%m-%d").to_string().as_str())
    }

    fn settings_json(&self) -> QString {
        let mut value = serde_json::to_value(&self.rust().settings).unwrap_or_default();
        value["defaultSocket"] = settings::default_socket().display().to_string().into();
        json(&value)
    }

    fn about_json(&self) -> QString {
        if self.rust().settings.books_path.is_empty()
            && self.rust().settings.socket_path.is_empty()
            && self.rust().settings.kind == ConnectionKind::Local
        {
            return json(
                &serde_json::json!({"server_version":kabacus_service::VERSION,"source_url":"https://github.com/BrannenGH/kabacus"}),
            );
        }
        match runtime().block_on(self.rust().client.version()) {
            Ok(v) => json(&serde_json::json!({
                "server_version": v.server_version,
                "commit": v.commit,
                "source_url": v.source_url,
            })),
            Err(e) => json(&serde_json::json!({ "error": e.to_string() })),
        }
    }

    fn test_connection(
        &self,
        kind: &QString,
        socket_path: &QString,
        server_url: &QString,
        private_ca: &QString,
    ) -> QString {
        let candidate = settings_from(kind, socket_path, server_url, private_ca);
        match runtime().block_on(settings::test_connection(&candidate)) {
            Ok(report) => json(&report),
            Err(e) => json(&serde_json::json!({ "error": e.to_string() })),
        }
    }

    fn save_settings(
        mut self: Pin<&mut Self>,
        kind: &QString,
        socket_path: &QString,
        server_url: &QString,
        private_ca: &QString,
    ) -> QString {
        let new = settings_from(kind, socket_path, server_url, private_ca);
        if let Err(e) = new.connect() {
            return QString::from(e.to_string().as_str());
        }
        if let Err(e) = new.save(&settings::config_path()) {
            return QString::from(
                crate::i18nc!("%1 is the error", "Couldn't save the settings: %1", e).as_str(),
            );
        }
        let (base, _) = connect(&new);
        let auth = connect_auth(&new, &base, &self.rust().store);
        {
            let mut this = self.as_mut().rust_mut();
            this.settings = new;
            this.auth = auth;
            this.sign_in_generation += 1;
            if let Some(task) = this.sign_in_task.take() {
                task.abort();
            }
            this.rebuild_client();
        }
        self.as_mut().publish_auth(None);
        self.as_mut().forget();
        self.refresh();
        QString::default()
    }

    fn begin_sign_in(mut self: Pin<&mut Self>) -> QString {
        self.as_mut().ensure_auth();
        let pending = match &self.rust().auth {
            Some(auth) => runtime().block_on(auth.begin_sign_in()),
            None => Err(model::ModelError::Invalid(crate::i18n!(
                "Can't reach the server to find out how to sign in"
            ))),
        };
        let pending = match pending {
            Ok(pending) => pending,
            Err(e) => return json(&serde_json::json!({ "error": e.to_string() })),
        };
        let url = pending.authorize_url.clone();
        let thread = self.qt_thread();
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.sign_in_generation += 1;
            if let Some(task) = this.sign_in_task.take() {
                task.abort();
            }
            this.sign_in_generation
        };
        let task = runtime().spawn(async move {
            let result = pending.finish().await;
            let _ = thread.queue(move |mut ledger: Pin<&mut qobject::Ledger>| {
                if ledger.rust().sign_in_generation != generation {
                    return;
                }
                ledger.as_mut().rust_mut().sign_in_task = None;
                match result {
                    Ok(done) => {
                        {
                            let mut this = ledger.as_mut().rust_mut();
                            if let Some(auth) = this.auth.as_mut() {
                                auth.signed_in(done);
                            }
                            // Refreshes begun while the browser was open used the old session.
                            this.sign_in_generation += 1;
                            this.rebuild_client();
                            // A load from the expired session would only
                            // report it expired again.
                            this.loads.restart();
                        }
                        ledger.as_mut().publish_auth(None);
                        ledger.refresh();
                    }
                    Err(e) => ledger.publish_auth(Some(&e.to_string())),
                }
            });
        });
        self.as_mut().rust_mut().sign_in_task = Some(task.abort_handle());
        self.publish_auth(None);
        json(&serde_json::json!({ "url": url }))
    }

    fn cancel_sign_in(mut self: Pin<&mut Self>) {
        {
            let mut this = self.as_mut().rust_mut();
            this.sign_in_generation += 1;
            if let Some(task) = this.sign_in_task.take() {
                task.abort();
            }
        }
        self.publish_auth(None);
    }

    fn sign_out(mut self: Pin<&mut Self>) -> QString {
        self.as_mut().cancel_sign_in();
        let result = match self.as_mut().rust_mut().auth.as_mut() {
            Some(auth) => runtime().block_on(auth.sign_out()),
            None => Ok(()),
        };
        self.as_mut().rust_mut().rebuild_client();
        self.as_mut().publish_auth(None);
        self.as_mut().forget();
        match result {
            Ok(()) => QString::default(),
            Err(e) => QString::from(e.to_string().as_str()),
        }
    }
}

impl qobject::Ledger {
    fn start_preview(mut self: Pin<&mut Self>, draft: &QString) -> i32 {
        let ticket = self.rust().preview_ticket.wrapping_add(1);
        self.as_mut().rust_mut().preview_ticket = ticket;
        let (draft, client) = (draft.to_string(), self.rust().client.clone());
        let thread = self.qt_thread();
        runtime().spawn(async move {
            let reply = match parse_draft(&draft) {
                Ok(draft) => match writes::preview(&client, &draft).await {
                    Ok(p) => serde_json::json!({ "ok": p }),
                    Err(e) => write_error(e),
                },
                Err(e) => write_error(e),
            };
            let _ = thread.queue(move |ledger: Pin<&mut qobject::Ledger>| {
                ledger.previewed(ticket, json(&reply));
            });
        });
        ticket
    }

    fn start_commit(mut self: Pin<&mut Self>, draft: &QString, head: &QString) {
        if self.rust().saving {
            return;
        }
        self.as_mut().set_saving(true);
        let (draft, head) = (draft.to_string(), head.to_string());
        let client = self.rust().client.clone();
        let thread = self.qt_thread();
        runtime().spawn(async move {
            let saved = match parse_draft(&draft) {
                Ok(draft) => writes::commit(&client, &draft, &head).await,
                Err(e) => Err(e),
            };
            let _ = thread.queue(move |mut ledger: Pin<&mut qobject::Ledger>| {
                ledger.as_mut().set_saving(false);
                let reply = match saved {
                    Ok(saved) => {
                        ledger.as_mut().refresh();
                        serde_json::json!({ "ok": saved })
                    }
                    Err(e) => write_error(e),
                };
                ledger.committed(json(&reply));
            });
        });
    }
}

fn parse_draft(draft: &str) -> Result<writes::Draft, writes::WriteError> {
    serde_json::from_str(draft).map_err(|e| writes::WriteError::Other {
        message: e.to_string(),
    })
}

/// The args `refresh` preloads with; pages ask for these views with no
/// args, which `Fetch.qml` sends as this.
const PRELOAD_ARGS: &str = "{}";

/// `{"ok": value}` or `{"error": message}` for QML.
fn reply<T: serde::Serialize, E: std::fmt::Display>(result: Result<T, E>) -> QString {
    match result {
        Ok(v) => json(&serde_json::json!({ "ok": v })),
        Err(e) => json(&serde_json::json!({ "error": e.to_string() })),
    }
}

/// `{"error": {kind, ...}, "message": …}`: the form shows it next to the
/// field, as a conflict sheet, or as a failed balance check.
fn write_error(e: writes::WriteError) -> serde_json::Value {
    serde_json::json!({ "error": e, "message": e.to_string() })
}

fn settings_from(
    kind: &QString,
    socket_path: &QString,
    server_url: &QString,
    private_ca: &QString,
) -> Settings {
    Settings {
        kind: if kind.to_string() == "remote" {
            ConnectionKind::Remote
        } else {
            ConnectionKind::Local
        },
        socket_path: socket_path.to_string().trim().to_string(),
        books_path: String::new(),
        server_url: server_url.to_string().trim().to_string(),
        private_ca: private_ca.to_string().trim().to_string(),
    }
}

fn json<T: serde::Serialize>(value: &T) -> QString {
    QString::from(
        serde_json::to_string(value)
            .unwrap_or_else(|_| "[]".to_string())
            .as_str(),
    )
}

fn workspace_path() -> std::path::PathBuf {
    settings::config_path().with_file_name("kabacus-windows.json")
}
fn workspace_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}

impl qobject::Ledger {
    fn canonical_books_path(&self, path: &QString) -> QString {
        std::fs::canonicalize(path.to_string())
            .map(|p| QString::from(p.to_string_lossy().as_ref()))
            .unwrap_or_else(|_| path.clone())
    }
    fn local_books_info(&self) -> QString {
        let result = runtime().block_on(async {
            let Some(books) = self.rust().client.local_books().await? else {
                return Ok::<_, kabacus_client::ClientError>(serde_json::Value::Null);
            };
            Ok(serde_json::json!({"path":books.path,"name":books.name().await?}))
        });
        json(&result.unwrap_or(serde_json::Value::Null))
    }
    fn open_books(mut self: Pin<&mut Self>, path: &QString) -> QString {
        match runtime().block_on(kabacus_client::LocalBooks::open(
            path.to_string(),
            kabacus_service::local::LocalCaller::for_app(kabacus_client::books::computer_user()),
        )) {
            Ok(books) => {
                let path = books.path.clone().unwrap_or_default();
                {
                    let mut this = self.as_mut().rust_mut();
                    this.settings.books_path = path.to_string_lossy().into_owned();
                    this.settings.socket_path.clear();
                    this.settings.kind = ConnectionKind::Local;
                    this.client = KabacusClient::from_local(books);
                    this.auth = None;
                    this.sign_in_generation += 1;
                    this.loads = Loads::default();
                    this.books_seen = None;
                    this.control = None;
                }
                self.as_mut().set_error(QString::default());
                self.as_mut().set_auth_json(json(
                    &serde_json::json!({"status":"off","username":"","issuer":"","warning":""}),
                ));
                self.as_mut().refresh();
                QString::default()
            }
            Err(e) => QString::from(e.to_string()),
        }
    }
    fn new_books(
        mut self: Pin<&mut Self>,
        path: &QString,
        name: &QString,
        currency: &QString,
        account: &QString,
    ) -> QString {
        match runtime().block_on(kabacus_client::LocalBooks::create(
            path.to_string(),
            &name.to_string(),
            &currency.to_string(),
            &account.to_string(),
            kabacus_service::local::LocalCaller::for_app(kabacus_client::books::computer_user()),
        )) {
            Ok(books) => {
                drop(books);
                self.as_mut().open_books(path)
            }
            Err(e) => QString::from(e.to_string()),
        }
    }
    fn save_copy(&self, path: &QString) -> QString {
        let result = runtime().block_on(async {
            let client = &self.rust().client;
            if let Some(books) = client.local_books().await? {
                return books.save_copy(path.to_string()).await;
            }
            let version = client.version().await?;
            let response = client
                .request_books(kabacus_client::BooksRequest {
                    method: "GET".into(),
                    path: format!("/ledgers/{}/export", version.ledger),
                    head: None,
                    body: serde_json::Value::Null,
                })
                .await?;
            if response.status >= 400 {
                return Err(kabacus_client::ClientError::Request(
                    String::from_utf8_lossy(&response.body).into_owned(),
                ));
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.to_string())
                .map_err(|e| kabacus_client::ClientError::Request(e.to_string()))?;
            std::io::Write::write_all(&mut file, &response.body)
                .map_err(|e| kabacus_client::ClientError::Request(e.to_string()))
        });
        match result {
            Ok(()) => QString::default(),
            Err(e) => QString::from(e.to_string()),
        }
    }
    fn workspace_json(&self) -> QString {
        let _lock = workspace_lock().lock().unwrap_or_else(|p| p.into_inner());
        let mut workspace = kabacus_client::workspace::Workspace::load(&workspace_path());
        let window = self
            .rust()
            .window_id
            .get()
            .and_then(|id| workspace.windows.iter().find(|w| &w.id == id));
        let automation = window.is_some_and(|w| w.automation);
        let port = window.map_or(7979, |w| w.control_port);
        workspace.automation = automation;
        workspace.control_port = port;
        json(&workspace)
    }
    fn remember_window(&self, state: &QString) -> QString {
        let result = (|| -> anyhow::Result<()> {
            let window: kabacus_client::workspace::WindowState =
                serde_json::from_str(&state.to_string())?;
            let _ = self.rust().window_id.set(window.id.clone());
            let _lock = workspace_lock().lock().unwrap_or_else(|p| p.into_inner());
            let mut workspace = kabacus_client::workspace::Workspace::load(&workspace_path());
            workspace.remember(window);
            workspace.save(&workspace_path())?;
            Ok(())
        })();
        match result {
            Ok(()) => QString::default(),
            Err(e) => QString::from(e.to_string()),
        }
    }
    fn forget_window(&self, id: &QString) {
        let _lock = workspace_lock().lock().unwrap_or_else(|p| p.into_inner());
        let mut workspace = kabacus_client::workspace::Workspace::load(&workspace_path());
        workspace.close(&id.to_string());
        let _ = workspace.save(&workspace_path());
    }
    fn save_preferences(
        mut self: Pin<&mut Self>,
        reopen: bool,
        automation: bool,
        port: i32,
    ) -> QString {
        if automation {
            if self
                .rust()
                .control
                .as_ref()
                .is_none_or(|c| i32::from(c.port) != port)
            {
                let result = runtime().block_on(async {
                    kabacus_client::control::ControlChannel::for_client(
                        self.rust().client.clone(),
                        port.try_into().map_err(|_| {
                            kabacus_client::ClientError::Request(
                                "Choose a port from 1 to 65535".into(),
                            )
                        })?,
                    )
                    .await
                });
                match result {
                    Ok(channel) => self.as_mut().rust_mut().control = Some(channel),
                    Err(e) => return QString::from(e.to_string()),
                }
            }
        } else {
            self.as_mut().rust_mut().control = None;
        }
        let _lock = workspace_lock().lock().unwrap_or_else(|p| p.into_inner());
        let mut workspace = kabacus_client::workspace::Workspace::load(&workspace_path());
        workspace.reopen = reopen;
        if let Some(window) = workspace
            .windows
            .iter_mut()
            .find(|w| Some(&w.id) == self.rust().window_id.get())
        {
            window.automation = automation;
            window.control_port = port.try_into().unwrap_or(7979);
        }
        match workspace.save(&workspace_path()) {
            Ok(()) => QString::default(),
            Err(e) => QString::from(e.to_string()),
        }
    }
    fn agent_activity(&self) -> QString {
        json(&self.rust().control.as_ref().map(|c| c.activity()))
    }
    fn command_catalog(&self) -> QString {
        json(&kabacus_service::catalog::commands())
    }
}
