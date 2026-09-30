// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared server state: the store, the ledger this server serves, auth,
//! and the books cache. Writes need no lock of their own: each commits
//! only against the revision it was read at.

use crate::auth::roles::Principal;
use crate::auth::AuthContext;
use crate::config::{Config, Database};
use kabacus_core::AccountName;
use kabacus_store::{AnyStore, BookStore, LedgerId, PostgresStore, SqliteStore, UserId};
use std::collections::{HashMap, HashSet};

pub struct AppState {
    pub config: Config,
    pub store: AnyStore,
    /// The ledger this server serves.
    pub ledger: LedgerId,
    pub auth: AuthContext,
    /// The account kind rules, replaced on reload.
    kinds: std::sync::RwLock<KindTable>,
    /// Which tool list each MCP client last saw.
    pub tool_lists: crate::mcp::ToolLists,
    /// The books as recently read, by revision.
    pub books_cache: crate::cache::BooksCache,
    /// Each signed-in caller's user, once looked up.
    users: tokio::sync::Mutex<HashMap<String, UserId>>,
}

/// The rule table and card reserve naming in use.
#[derive(Debug, Clone, Default)]
pub struct KindTable {
    pub rules: Vec<kabacus_core::KindRule>,
    pub reserve_template: String,
}

/// The name the ledger a server makes for itself is given.
pub const DEFAULT_LEDGER: &str = "Books";

/// The user who owns what a server makes for itself.
pub const SERVER_USER: &str = "Kabacus";

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

/// The ledger named `name`, or when none is named the only (or first)
/// one; made empty if there is none.
pub async fn find_or_create_ledger(
    store: &AnyStore,
    name: Option<&str>,
) -> anyhow::Result<LedgerId> {
    let ledgers = store.ledgers().await?;
    let found = match name {
        Some(name) => ledgers.iter().find(|l| l.name == name),
        None => ledgers.first(),
    };
    if let Some(ledger) = found {
        return Ok(ledger.id);
    }
    let owner = store.local_user(SERVER_USER).await?;
    Ok(store
        .create_ledger(name.unwrap_or(DEFAULT_LEDGER), owner)
        .await?)
}

impl AppState {
    pub async fn new(config: Config) -> anyhow::Result<AppState> {
        let store = open_store(&config).await?;
        Self::with_store(config, store).await
    }

    pub async fn with_store(config: Config, store: AnyStore) -> anyhow::Result<AppState> {
        let ledger = find_or_create_ledger(&store, config.ledger.as_deref()).await?;
        let auth = AuthContext::init(config.auth.clone()).await?;
        let kinds = std::sync::RwLock::new(KindTable {
            rules: config.account_kinds.clone(),
            reserve_template: config.card_reserve_account.clone(),
        });
        Ok(AppState {
            kinds,
            tool_lists: crate::mcp::ToolLists::new(),
            books_cache: Default::default(),
            config,
            store,
            ledger,
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

    /// Every account currently declared.
    pub async fn declared_accounts(&self) -> anyhow::Result<HashSet<AccountName>> {
        Ok(self.books_snapshot().await?.declared.clone())
    }

    /// The kind rules in use now.
    pub fn kind_table(&self) -> KindTable {
        self.kinds
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// A chart that only knows the rules: enough to classify the accounts
    /// of one transaction.
    pub fn kind_rules_chart(&self) -> kabacus_core::Chart {
        let table = self.kind_table();
        kabacus_core::Chart::from_rules(table.rules, table.reserve_template)
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
        *self
            .kinds
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = KindTable {
            rules: config.account_kinds,
            reserve_template: config.card_reserve_account,
        };
        Ok(())
    }

    pub async fn commodities(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.books_snapshot().await?.commodities.clone())
    }

    /// Account path -> friendly name, from the `ynab-account` /
    /// `ynab-envelope` tags on account declarations.
    pub async fn friendly_account_names(&self) -> anyhow::Result<HashMap<String, String>> {
        Ok(friendly_names(&self.books_snapshot().await?.book))
    }

    pub async fn account_types(&self) -> anyhow::Result<HashMap<String, bool>> {
        Ok(account_types(&self.books_snapshot().await?.book))
    }
}

pub fn account_types(book: &kabacus_ledger::Book) -> HashMap<String, bool> {
    book.accounts
        .values()
        .filter_map(|a| {
            let declaration = kabacus_core::AccountDeclaration {
                name: a.name.clone(),
                tags: kabacus_core::declared_tags(&a.tags),
            };
            // Invalid imported metadata must never hide a physical account.
            declaration
                .synthetic_override()
                .unwrap_or(Some(false))
                .map(|synthetic| (a.name.to_string(), synthetic))
        })
        .collect()
}

/// Account path -> friendly name, from the tags of `book`'s declarations.
pub fn friendly_names(book: &kabacus_ledger::Book) -> HashMap<String, String> {
    book.accounts
        .values()
        .filter_map(|a| {
            let declaration = kabacus_core::AccountDeclaration {
                name: a.name.clone(),
                tags: kabacus_core::declared_tags(&a.tags),
            };
            declaration
                .friendly_name()
                .map(|f| (a.name.as_str().to_string(), f.to_string()))
        })
        .collect()
}
