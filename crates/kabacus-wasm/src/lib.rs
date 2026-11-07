// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The browser calls the same store and service directly in its page.

use kabacus_service::{
    local::LocalCaller,
    state::{KindTable, Settings},
    Service,
};
use kabacus_store::{AnyStore, BookStore, SqliteStore, Who};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BrowserBooks {
    service: Arc<Service>,
    caller: LocalCaller,
}
fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
impl BrowserBooks {
    async fn with_store(store: SqliteStore, username: String) -> Result<Self, JsValue> {
        let store = AnyStore::Sqlite(store);
        let ledgers = store.ledgers().await.map_err(error)?;
        let ledger = ledgers
            .first()
            .ok_or_else(|| error("The file holds no ledger"))?
            .id;
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
            caller: LocalCaller::for_app(username),
        })
    }
}
#[wasm_bindgen]
impl BrowserBooks {
    pub async fn open(bytes: Vec<u8>, username: String) -> Result<BrowserBooks, JsValue> {
        Self::with_store(SqliteStore::from_bytes(&bytes).map_err(error)?, username).await
    }
    pub async fn create(
        name: String,
        currency: String,
        account: String,
        username: String,
    ) -> Result<BrowserBooks, JsValue> {
        let book = kabacus_service::setup::initial_book(&currency, &account).map_err(error)?;
        let store = SqliteStore::in_memory().map_err(error)?;
        let user = store.local_user(&username).await.map_err(error)?;
        let ledger = store.create_ledger(&name, user).await.map_err(error)?;
        let who = Who {
            user,
            actor: "app".into(),
            kind: "new".into(),
            summary: format!("New books: {name}"),
            details: serde_json::json!({"user":username}),
        };
        store.import(ledger, &book, &who).await.map_err(error)?;
        Self::with_store(store, username).await
    }
    pub async fn request(&self, request: String) -> Result<String, JsValue> {
        let request = serde_json::from_str(&request).map_err(error)?;
        let response = kabacus_service::dispatch::dispatch(&self.service, &self.caller, request)
            .await
            .unwrap_or_else(kabacus_service::dispatch::Response::error);
        serde_json::to_string(&response).map_err(error)
    }
    pub async fn bytes(&self) -> Result<Vec<u8>, JsValue> {
        let ids = self
            .service
            .store
            .ledgers()
            .await
            .map_err(error)?
            .iter()
            .map(|l| l.id)
            .collect::<Vec<_>>();
        self.service.store.export_bytes(&ids).await.map_err(error)
    }
    pub fn allow_rewrites(&mut self, enabled: bool) {
        self.caller.allow_rewrites = enabled;
    }
}
