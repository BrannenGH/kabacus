// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One document's durable working copy and conditional snapshot saves.
//! Only this server opens the working database. Clients share its API and MCP.

pub mod host;

use crate::config::{Config, Database};
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use host::{Host, Snapshot};
use kabacus_store::{BookStore, LedgerId, Revision};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Session {
    pub state: Arc<AppState>,
    host: Host,
    base: Snapshot,
    saved: Vec<(LedgerId, Revision)>,
    recovery: PathBuf,
    blocked: bool,
}

pub fn recovery_root() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(state).join("kabacus/recovery");
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".local/state/kabacus/recovery"),
        None => std::env::temp_dir().join("kabacus-recovery"),
    }
}

impl Session {
    pub async fn open(
        mut config: Config,
        host: Host,
        recovery_root: &Path,
    ) -> anyhow::Result<Self> {
        let base = host.read(config.import_max_bytes).await?;
        std::fs::create_dir_all(recovery_root)?;
        let recovery = tempfile::Builder::new()
            .prefix("document-")
            .tempdir_in(recovery_root)?
            .keep();
        let base_path = recovery.join("base.kabacus");
        std::fs::write(&base_path, &base.bytes)?;
        // Read only through the import validator: schema, audit replay and strict books checks.
        let dumps = kabacus_store::file::read(&base_path)?;
        let working = recovery.join("working.kabacus");
        kabacus_store::file::write(&dumps, &working)?;
        write_metadata(&host, &base, &recovery)?;
        config.database = Some(working.clone());
        config.database_url = None;
        config.store = Database::Sqlite(working);
        config.validate()?;
        let state = Arc::new(AppState::new(config).await?);
        let saved = revisions(&state).await?;
        Ok(Self {
            state,
            host,
            base,
            saved,
            recovery,
            blocked: false,
        })
    }

    pub fn recovery_path(&self) -> &Path {
        &self.recovery
    }

    pub async fn flush(&mut self) -> anyhow::Result<()> {
        let current = revisions(&self.state).await?;
        if current == self.saved {
            return Ok(());
        }
        let ids: Vec<_> = current.iter().map(|(id, _)| *id).collect();
        let snapshot = self.recovery.join("pending.kabacus");
        self.state.store.export_file(&ids, &snapshot).await?;
        let bytes = std::fs::read(&snapshot)?;
        // Pending is a self-contained recovery file, unlike a live SQLite database with a WAL.
        self.base = self
            .host
            .save(&self.base, bytes, self.state.config.import_max_bytes)
            .await?;
        self.saved = current;
        std::fs::write(self.recovery.join("base.kabacus"), &self.base.bytes)?;
        write_metadata(&self.host, &self.base, &self.recovery)?;
        Ok(())
    }
}

fn write_metadata(host: &Host, base: &Snapshot, recovery: &Path) -> anyhow::Result<()> {
    std::fs::write(
        recovery.join("session.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source": host.location(),
            "storage_version": base.version,
            "working_file": "working.kabacus",
            "base_file": "base.kabacus",
        }))?,
    )?;
    Ok(())
}

async fn revisions(state: &AppState) -> anyhow::Result<Vec<(LedgerId, Revision)>> {
    Ok(state
        .store
        .ledgers()
        .await?
        .into_iter()
        .map(|l| (l.id, l.revision))
        .collect())
}

pub fn router(session: Session) -> Router {
    let state = session.state.clone();
    crate::api::router(state).layer(axum::middleware::from_fn_with_state(
        Arc::new(Mutex::new(session)),
        checkpoint,
    ))
}

async fn checkpoint(
    State(session): State<Arc<Mutex<Session>>>,
    request: Request,
    next: Next,
) -> Response {
    // Includes MCP and imports: even a partially committed non-atomic batch is checkpointed.
    let mut session = session.lock().await;
    if let Err(error) = crate::api::authenticate(&session.state, request.headers()).await {
        // Metadata and web assets retain their existing unauthenticated GET behavior.
        if request.method() == axum::http::Method::GET
            || request.method() == axum::http::Method::HEAD
        {
            return next.run(request).await;
        }
        return error.into_response();
    }
    let may_write = !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    );
    if may_write {
        if session.blocked {
            return refused(
                &session,
                false,
                "The document session needs recovery before further writes.",
            );
        }
        if let Err(error) = session
            .host
            .unchanged(&session.base, session.state.config.import_max_bytes)
            .await
        {
            session.blocked = true;
            return refused(&session, false, &error.to_string());
        }
    }
    let response = next.run(request).await;
    if !session.blocked {
        if let Err(error) = session.flush().await {
            session.blocked = true;
            return refused(&session, true, &error.to_string());
        }
    }
    response
}

fn refused(session: &Session, committed_locally: bool, message: &str) -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        Json(serde_json::json!({
            "code": "document_conflict",
            "message": message,
            "details": {
                "committed_locally": committed_locally,
                "recovery_directory": session.recovery,
                "retry_write": false,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
