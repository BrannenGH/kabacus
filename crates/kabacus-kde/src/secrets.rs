// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the app keeps its refresh token: the platform secret
//! store, never KConfig or a plain file. On Plasma that's KWallet, through
//! the freedesktop Secret Service API it provides; elsewhere, any Secret
//! Service (GNOME Keyring, KeePassXC, …).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// Whose secret: one refresh token per server and provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretAccount {
    /// The server's address, or `unix:<socket>` for the local one.
    pub server: String,
    pub issuer: String,
}

pub trait SecretStore: Send + Sync + 'static {
    fn load<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, Option<String>>;
    fn save<'a>(&'a self, account: &'a SecretAccount, secret: &'a str) -> StoreFuture<'a, ()>;
    fn clear<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, ()>;
}

/// The Secret Service over D-Bus. In a sandbox this needs
/// `--talk-name=org.freedesktop.secrets`.
pub struct SecretServiceStore;

fn attributes(account: &SecretAccount) -> HashMap<&str, &str> {
    HashMap::from([
        ("application", "kabacus"),
        ("kind", "refresh-token"),
        ("server", account.server.as_str()),
        ("issuer", account.issuer.as_str()),
    ])
}

async fn service() -> Result<secret_service::SecretService<'static>, String> {
    secret_service::SecretService::connect(secret_service::EncryptionType::Dh)
        .await
        .map_err(|e| {
            // Lower case: it's the %1 of the messages that report it.
            crate::i18nc!("%1 is the error", "can't reach the secret store: %1", e)
        })
}

/// Where to keep a new secret: the `default` collection (KWallet's wallet,
/// GNOME's login keyring), else `login`, else any collection but the
/// in-memory `session` one, which would forget it at logout. With none of
/// those, ask the store to make a default one; it prompts the user.
async fn persistent_collection<'a>(
    service: &'a secret_service::SecretService<'a>,
) -> Result<secret_service::Collection<'a>, String> {
    if let Ok(c) = service.get_default_collection().await {
        return Ok(c);
    }
    if let Ok(c) = service.get_collection_by_alias("login").await {
        return Ok(c);
    }
    let session = service
        .get_collection_by_alias("session")
        .await
        .ok()
        .map(|c| c.collection_path.clone());
    let collections = service
        .get_all_collections()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(c) = collections
        .into_iter()
        .find(|c| Some(&c.collection_path) != session.as_ref())
    {
        return Ok(c);
    }
    service
        .create_collection("Login", "default")
        .await
        .map_err(|e| {
            crate::i18nc!(
                "%1 is the error",
                "the secret store has no wallet or keyring to keep it in (%1)",
                e
            )
        })
}

impl SecretStore for SecretServiceStore {
    fn load<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, Option<String>> {
        Box::pin(async move {
            let service = service().await?;
            let found = service
                .search_items(attributes(account))
                .await
                .map_err(|e| e.to_string())?;
            let Some(item) = found.unlocked.first().or(found.locked.first()) else {
                return Ok(None);
            };
            // Unlocking may prompt (KWallet's password dialog).
            item.ensure_unlocked().await.map_err(|e| e.to_string())?;
            let secret = item.get_secret().await.map_err(|e| e.to_string())?;
            Ok(Some(String::from_utf8(secret).map_err(|e| e.to_string())?))
        })
    }

    fn save<'a>(&'a self, account: &'a SecretAccount, secret: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let service = service().await?;
            let collection = persistent_collection(&service).await?;
            collection
                .ensure_unlocked()
                .await
                .map_err(|e| e.to_string())?;
            collection
                .create_item(
                    // The item's label, as KWallet or Seahorse lists it.
                    &crate::i18nc!(
                        "%1 is the server's address",
                        "Kabacus sign-in for %1",
                        account.server
                    ),
                    attributes(account),
                    secret.as_bytes(),
                    true,
                    "text/plain",
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn clear<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let service = service().await?;
            let found = service
                .search_items(attributes(account))
                .await
                .map_err(|e| e.to_string())?;
            for item in found.unlocked.iter().chain(found.locked.iter()) {
                item.delete().await.map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    }
}

/// A store in memory, for tests. `unavailable` makes every call fail, as
/// when no Secret Service is running.
#[derive(Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<SecretAccount, String>>,
    pub unavailable: bool,
}

impl MemoryStore {
    /// A store that fails every call.
    pub fn unavailable() -> MemoryStore {
        MemoryStore {
            unavailable: true,
            ..Default::default()
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.unavailable {
            return Err("no secret store is running".into());
        }
        Ok(())
    }
}

impl MemoryStore {
    pub fn get(&self, account: &SecretAccount) -> Option<String> {
        self.secrets.lock().unwrap().get(account).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.lock().unwrap().is_empty()
    }
}

impl SecretStore for MemoryStore {
    fn load<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, Option<String>> {
        Box::pin(async move {
            self.check()?;
            Ok(self.get(account))
        })
    }

    fn save<'a>(&'a self, account: &'a SecretAccount, secret: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check()?;
            self.secrets
                .lock()
                .unwrap()
                .insert(account.clone(), secret.to_string());
            Ok(())
        })
    }

    fn clear<'a>(&'a self, account: &'a SecretAccount) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check()?;
            self.secrets.lock().unwrap().remove(account);
            Ok(())
        })
    }
}
