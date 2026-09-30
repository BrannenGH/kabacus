// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Public UI assets only; ledger access stays on the authenticated API.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/app", get(|| async { Redirect::permanent("/app/") }))
        .route("/app/", get(index))
        .route("/app/{*path}", get(asset))
}

async fn index(State(state): State<Arc<AppState>>) -> Response {
    serve(&state, "index.html").await
}

async fn asset(State(state): State<Arc<AppState>>, Path(path): Path<String>) -> Response {
    serve(&state, &path).await
}

async fn serve(state: &AppState, path: &str) -> Response {
    let Some(root) = state.config.web_dir.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Serve the build's entry point and assets, never arbitrary files in web_dir.
    if path != "index.html" && !path.starts_with("assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part.starts_with('.') || part.contains('\\'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mime = match path.rsplit('.').next() {
        Some("html") if path == "index.html" => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(file) = tokio::fs::canonicalize(root.join(path)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !file.starts_with(&root) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(file).await {
        Ok(bytes) => ([
            ("content-type", mime),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' https:; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"),
        ], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
