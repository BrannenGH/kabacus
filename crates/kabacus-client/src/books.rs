// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! One request contract for direct library calls and remote books.

use crate::{ClientError, KabacusClient};
use kabacus_service::{
    dispatch::{Request, Response},
    local::LocalCaller,
    state::{KindTable, Settings},
    Service,
};
use kabacus_store::{AnyStore, BookStore, SqliteStore};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

pub trait Books: Send + Sync {
    fn request(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<Response, ClientError>> + Send;
}

#[derive(Clone)]
pub struct LocalBooks {
    pub service: Arc<Service>,
    pub caller: LocalCaller,
    pub path: Option<PathBuf>,
}

pub fn computer_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "local".into())
}

fn failure(e: impl std::fmt::Display) -> ClientError {
    ClientError::Server(e.to_string())
}

impl LocalBooks {
    pub async fn from_store(store: AnyStore, caller: LocalCaller) -> Result<Self, ClientError> {
        let ledger = match store.ledgers().await.map_err(failure)?.first() {
            Some(ledger) => ledger.id,
            None => {
                let owner = store.local_user(&caller.username).await.map_err(failure)?;
                store.create_ledger("Books", owner).await.map_err(failure)?
            }
        };
        Ok(Self {
            service: Arc::new(Service::new(
                store,
                ledger,
                Settings::default(),
                KindTable {
                    rules: kabacus_core::default_kind_rules(),
                    reserve_template: kabacus_core::DEFAULT_RESERVE_TEMPLATE.into(),
                },
            )),
            caller,
            path: None,
        })
    }

    pub async fn open(path: impl AsRef<Path>, caller: LocalCaller) -> Result<Self, ClientError> {
        let path = std::fs::canonicalize(path).map_err(failure)?;
        if !path.is_file() {
            return Err(ClientError::Request("Choose a .kabacus file".into()));
        }
        let validate_path = path.clone();
        tokio::task::spawn_blocking(move || kabacus_store::file::read(&validate_path))
            .await
            .map_err(failure)?
            .map_err(failure)?;
        let store = SqliteStore::open(&path).map_err(failure)?;
        let mut books = Self::from_store(AnyStore::Sqlite(store), caller).await?;
        books.path = Some(path);
        Ok(books)
    }

    pub async fn create(
        path: impl AsRef<Path>,
        name: &str,
        currency: &str,
        account: &str,
        caller: LocalCaller,
    ) -> Result<Self, ClientError> {
        let path = path.as_ref();
        let initial = kabacus_service::setup::initial_book(currency, account).map_err(failure)?;
        // Reserve the name before SQLite opens it; New Books never overwrites.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(failure)?;
        let operation = async {
            let store = SqliteStore::open(path).map_err(failure)?;
            let owner = store.local_user(&caller.username).await.map_err(failure)?;
            let ledger = store.create_ledger(name, owner).await.map_err(failure)?;
            let who = kabacus_store::Who {
                user: owner,
                actor: caller.actor.clone(),
                kind: "new".into(),
                summary: format!("New books: {name}"),
                details: serde_json::json!({"user": caller.username}),
            };
            store
                .import(ledger, &initial, &who)
                .await
                .map_err(failure)?;
            let mut books = Self::from_store(AnyStore::Sqlite(store), caller).await?;
            books.path = Some(std::fs::canonicalize(path).map_err(failure)?);
            Ok(books)
        }
        .await;
        if operation.is_err() {
            let _ = std::fs::remove_file(path);
        }
        operation
    }

    pub async fn name(&self) -> Result<String, ClientError> {
        self.service
            .store
            .ledgers()
            .await
            .map_err(failure)?
            .into_iter()
            .find(|l| l.id == self.service.ledger)
            .map(|l| l.name)
            .ok_or_else(|| ClientError::NotFound("Books".into()))
    }
    pub async fn save_copy(&self, path: impl AsRef<Path>) -> Result<(), ClientError> {
        self.service
            .store
            .export_file(&[self.service.ledger], path.as_ref())
            .await
            .map_err(failure)
    }
}

impl Books for LocalBooks {
    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        Ok(
            kabacus_service::dispatch::dispatch(&self.service, &self.caller, request)
                .await
                .unwrap_or_else(Response::error),
        )
    }
}
impl Books for KabacusClient {
    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let method = request.method.parse().map_err(failure)?;
        let body = if request.body.is_null() {
            None
        } else {
            Some(serde_json::to_vec(&request.body)?)
        };
        let (status, body, headers) = self
            .request(method, &request.path, request.head.as_deref(), body)
            .await?;
        Ok(Response {
            status,
            body,
            content_type: headers
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("application/json")
                .into(),
            filename: None,
        })
    }
}

pub(crate) struct LocalFile {
    pub path: PathBuf,
    pub caller: LocalCaller,
    pub ready: tokio::sync::OnceCell<LocalBooks>,
}
impl LocalFile {
    pub async fn books(&self) -> Result<&LocalBooks, ClientError> {
        self.ready
            .get_or_try_init(|| LocalBooks::open(&self.path, self.caller.clone()))
            .await
    }
}
