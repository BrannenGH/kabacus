// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared server state: the store, the ledger this server serves, auth,
//! and the books cache. Writes need no lock of their own: each commits
//! only against the revision it was read at.

use crate::auth::roles::Principal;
use crate::auth::AuthContext;
use crate::config::{Config, Database};
use kabacus_store::{AnyStore, BookStore, PostgresStore, SqliteStore, UserId};
use std::collections::HashMap;

pub use kabacus_service::state::{find_or_create_ledger, KindTable, DEFAULT_LEDGER, SERVER_USER};

pub struct AppState {
    pub config: Config,
    pub auth: AuthContext,
    pub tool_lists: std::sync::Arc<crate::mcp::ToolLists>,
    pub(crate) service: std::sync::Arc<kabacus_service::state::Service>,
    users: tokio::sync::Mutex<HashMap<String, UserId>>,
}

impl std::ops::Deref for AppState {
    type Target = kabacus_service::state::Service;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

/// Open the configured store.
pub async fn open_store(config: &Config) -> anyhow::Result<AnyStore> {
    Ok(match &config.store {
        Database::Sqlite(path) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            AnyStore::Sqlite(SqliteStore::open(path)?)
        }
        Database::Postgres(url) => {
            AnyStore::Postgres(PostgresStore::connect(url, config.database_connections).await?)
        }
    })
}

impl AppState {
    pub async fn new(config: Config) -> anyhow::Result<AppState> {
        let store = open_store(&config).await?;
        Self::with_store(config, store).await
    }

    pub async fn with_store(config: Config, store: AnyStore) -> anyhow::Result<AppState> {
        let ledger = find_or_create_ledger(&store, config.ledger.as_deref()).await?;
        let auth = AuthContext::init(config.auth.clone()).await?;
        let kinds = KindTable {
            rules: config.account_kinds.clone(),
            reserve_template: config.card_reserve_account.clone(),
        };
        Ok(AppState {
            service: std::sync::Arc::new(kabacus_service::state::Service::new(
                store,
                ledger,
                kabacus_service::state::Settings {
                    allow_rewrite: config.allow_rewrite,
                    import_max_bytes: config.import_max_bytes,
                    app_client_id: config.auth.clients.app.clone(),
                },
                kinds,
            )),
            tool_lists: std::sync::Arc::new(crate::mcp::ToolLists::new()),
            config,
            auth,
            users: Default::default(),
        })
    }

    /// The user the audit log records for `principal`: their identity at
    /// the issuer, or in single-user mode a user by their name.
    pub async fn user_of(&self, principal: &Principal) -> anyhow::Result<UserId> {
        let key = format!(
            "{}\u{1f}{}\u{1f}{}",
            principal.issuer, principal.subject, principal.username
        );
        let mut users = self.users.lock().await;
        if let Some(id) = users.get(&key) {
            return Ok(*id);
        }
        let id = if principal.auth_none {
            self.store.local_user(&principal.username).await?
        } else {
            self.store
                .user_for_identity(&principal.issuer, &principal.subject, &principal.username)
                .await?
        };
        users.insert(key, id);
        Ok(id)
    }

    /// Re-read the kind rules from the config file. A file that
    /// can't be read or has unusable rules leaves the current ones in place.
    pub fn reload_kinds(&self) -> anyhow::Result<()> {
        let Some(path) = &self.config.config_file else {
            anyhow::bail!(
                "the server was started without a config file; there is nothing to reload"
            );
        };
        let config = Config::load_from_file(path)?;
        config.validate_kinds()?;
        self.service.replace_kinds(KindTable {
            rules: config.account_kinds,
            reserve_template: config.card_reserve_account,
        });
        Ok(())
    }
}
