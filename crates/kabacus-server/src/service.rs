// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server metadata and authorization adapters around shared book operations.

use crate::api::ApiError;
use crate::auth::roles::{Principal, Role};
use crate::state::AppState;
pub use kabacus_service::service::*;
use serde::Serialize;

pub fn require(principal: &Principal, role: Role) -> Result<(), ApiError> {
    if principal.has_role(role) {
        Ok(())
    } else {
        Err(ApiError::MissingRole(role))
    }
}

pub fn invalid(rule: &str, message: String) -> ApiError {
    kabacus_service::service::invalid(rule, message).into()
}

// --- version (unauthenticated) ------------------------------------

#[derive(Debug, Serialize)]
pub struct VersionDto {
    pub ledger: i64,
    pub server_version: String,
    /// The exact commit this server was built from (AGPL §13: the source
    /// of what is running); empty when the build had no git history.
    pub commit: String,
    /// Where that commit's source is: its tree in the repository, or the
    /// repository itself when the commit is unknown.
    pub source_url: String,
    /// A stable hash of every tool's name and schema, so a client can
    /// tell its tool list is out of date without listing the tools.
    pub tool_list_hash: String,
    /// The books' revision.
    pub head: Option<String>,
    /// Changes whenever the books do; the app refreshes when it moves.
    pub books: Option<String>,
    /// The ledger engine's version.
    pub engine_version: String,
    /// The store's schema version, and which store it is.
    pub schema_version: usize,
    pub store: String,
    pub auth: AuthInfoDto,
}

#[derive(Debug, Serialize)]
pub struct AuthInfoDto {
    pub mode: String,
    pub issuer: Option<String>,
    pub app_client_id: Option<String>,
}

const COMMIT: &str = env!("KABACUS_COMMIT");
const REPOSITORY: &str = env!("KABACUS_SOURCE_URL");

fn source_url() -> String {
    if COMMIT.is_empty() {
        REPOSITORY.to_string()
    } else {
        format!("{REPOSITORY}/tree/{COMMIT}")
    }
}

pub async fn version(state: &AppState) -> VersionDto {
    use kabacus_store::BookStore;
    let revision = state
        .store
        .revision(state.ledger)
        .await
        .ok()
        .map(|r| r.to_string());
    let mode = match state.auth.mode() {
        crate::config::AuthMode::Oauth => "oauth",
        crate::config::AuthMode::None => "none",
    };
    VersionDto {
        ledger: state.ledger,
        server_version: kabacus_ledger::VERSION.to_string(),
        commit: COMMIT.to_string(),
        source_url: source_url(),
        tool_list_hash: crate::mcp::tool_list_hash(),
        head: revision.clone(),
        books: revision,
        engine_version: kabacus_ledger::VERSION.to_string(),
        schema_version: kabacus_store::sqlite::schema::latest_version(),
        store: match &state.store {
            kabacus_store::AnyStore::Sqlite(_) => "sqlite",
            kabacus_store::AnyStore::Postgres(_) => "postgres",
        }
        .to_string(),
        auth: AuthInfoDto {
            mode: mode.to_string(),
            issuer: state.auth.issuer().map(str::to_string),
            app_client_id: state.config.auth.clients.app.clone(),
        },
    }
}
