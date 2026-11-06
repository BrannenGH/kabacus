// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Store-backed book operations, independent of server configuration.

use crate::caller::Caller;
use kabacus_core::AccountName;
use kabacus_store::{AnyStore, BookStore, LedgerId, UserId};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

pub struct Service {
    pub store: AnyStore,
    pub ledger: LedgerId,
    pub settings: Settings,
    pub books_cache: crate::cache::BooksCache,
    kinds: RwLock<KindTable>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub allow_rewrite: bool,
    pub import_max_bytes: usize,
    pub app_client_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            allow_rewrite: true,
            import_max_bytes: 256 * 1024 * 1024,
            app_client_id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KindTable {
    pub rules: Vec<kabacus_core::KindRule>,
    pub reserve_template: String,
}

/// The name the ledger a server makes for itself is given.
pub const DEFAULT_LEDGER: &str = "Books";

/// The user who owns what a server makes for itself.
pub const SERVER_USER: &str = "Kabacus";

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

impl Service {
    pub fn new(store: AnyStore, ledger: LedgerId, settings: Settings, kinds: KindTable) -> Self {
        Self {
            store,
            ledger,
            settings,
            kinds: RwLock::new(kinds),
            books_cache: Default::default(),
        }
    }

    pub async fn user_of(&self, caller: &dyn Caller) -> anyhow::Result<UserId> {
        caller.user(&self.store).await
    }

    pub fn replace_kinds(&self, kinds: KindTable) {
        *self.kinds.write().unwrap_or_else(|p| p.into_inner()) = kinds;
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
