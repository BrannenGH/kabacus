// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP endpoint, served by kabacus-server itself at `/mcp`
//! (streamable HTTP) on the unix socket and, when configured, the remote
//! HTTPS listener.
//!
//! Every request is authenticated exactly as the API's are: the
//! same bearer tokens, validated by the same [`crate::auth::AuthContext`],
//! with roles from the same `role_map`. An unauthenticated request gets a
//! 401 pointing at this server's RFC 9728 protected resource metadata, which
//! names the configured issuer, so an MCP client can sign the user in itself.
//!
//! The tools mirror the API one for one ([`crate::api::ENDPOINTS`]) and call
//! the same [`crate::service`] functions, so they have the same role checks
//! and gates. The rewrite tools are listed only to a caller who can
//! use them: the server doesn't ban rewrites (`--no-rewrite`) and the
//! caller holds `ledger-rewrite`. Like every write tool they default to a
//! dry run.

pub mod skills;
pub mod tools;

use crate::auth::roles::Channel;
use crate::config::AuthMode;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;

pub use tools::{tool_list_hash, KabacusMcp, REWRITE_TOOLS};

pub const PATH: &str = "/mcp";
const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// Marks a request that arrived on the remote HTTPS listener, so the
/// resource URL this server advertises has the right scheme.
#[derive(Debug, Clone, Copy)]
pub struct ViaHttps;

/// `/mcp` and its protected resource metadata, for [`crate::api::router`].
pub fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let tool_state = state.clone();
    // Stateless: every call is its own authenticated POST, so a token that
    // expires or loses a role mid-conversation takes effect at once.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        // Requests reach this through the unix socket or the remote
        // listener under whatever name it's published as, and every one
        // carries a bearer token a browser wouldn't attach on its own, so
        // the Host allowlist adds nothing. Browsers are still turned away:
        // any request with an Origin header is refused.
        .disable_allowed_hosts()
        .enforce_origin_validation()
        // A batch may be 10 MB; leave room for the envelope.
        .with_max_request_body_bytes(crate::batch::MAX_BYTES + 1024 * 1024);
    let service = StreamableHttpService::new(
        move || Ok(KabacusMcp::new(tool_state.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service(PATH, service)
        // Only the route above: the metadata routes below stay open.
        .route_layer(axum::middleware::from_fn_with_state(state, authenticate))
        .route(METADATA_PATH, get(protected_resource_metadata))
        .route(
            &format!("{METADATA_PATH}{PATH}"),
            get(protected_resource_metadata),
        )
}

/// The URL clients reach `/mcp` at: the configured one, else built from
/// the host the request was sent to.
fn resource_url(state: &AppState, req: &Request, https: bool) -> String {
    if let Some(url) = &state.config.mcp.resource_url {
        return url.trim_end_matches('/').to_string();
    }
    let scheme = if https { "https" } else { "http" };
    format!("{scheme}://{}{PATH}", request_host(req))
}

/// The host (and port) a request was sent to: its `Host` header (HTTP/1.1)
/// or its URI's authority (HTTP/2 has no `Host`, only `:authority`).
fn request_host<B>(req: &axum::http::Request<B>) -> &str {
    req.headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri().authority().map(|a| a.as_str()))
        .unwrap_or("localhost")
}

/// Where the metadata for `resource` lives (RFC 9728 §3.1: the well-known
/// prefix inserted before the resource's path).
fn metadata_url(resource: &str) -> String {
    match resource.find("://").map(|i| i + 3) {
        Some(start) => {
            let path_at = resource[start..]
                .find('/')
                .map_or(resource.len(), |i| start + i);
            format!(
                "{}{METADATA_PATH}{}",
                &resource[..path_at],
                &resource[path_at..]
            )
        }
        None => format!("{METADATA_PATH}{PATH}"),
    }
}

/// Authenticate the request as the API would, and hand the
/// principal, marked as arriving through MCP, to the tools.
async fn authenticate(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let headers = req.headers();
    let authz = headers.get("authorization").and_then(|v| v.to_str().ok());
    // In `--auth=none` mode the client names itself; MCP clients don't
    // send the app's hint header, so default to "mcp".
    let hint = headers
        .get("kabacus-client-hint")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("mcp");
    match state.auth.authenticate(authz, Some(hint)).await {
        Ok(mut principal) => {
            principal.channel = Channel::Mcp;
            req.extensions_mut().insert(principal);
            next.run(req).await
        }
        Err(e) => {
            tracing::debug!("mcp: refused a request: {e}");
            let https = req.extensions().get::<ViaHttps>().is_some();
            let resource = resource_url(&state, &req, https);
            let mut challenge = format!("Bearer resource_metadata=\"{}\"", metadata_url(&resource));
            if authz.is_some() {
                challenge.push_str(", error=\"invalid_token\"");
            }
            (
                StatusCode::UNAUTHORIZED,
                [(axum::http::header::WWW_AUTHENTICATE, challenge)],
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response()
        }
    }
}

/// RFC 9728 protected resource metadata: which authorization server issues
/// tokens for `/mcp`. Only in OAuth mode.
async fn protected_resource_metadata(State(state): State<Arc<AppState>>, req: Request) -> Response {
    if *state.auth.mode() == AuthMode::None {
        return StatusCode::NOT_FOUND.into_response();
    }
    let https = req.extensions().get::<ViaHttps>().is_some();
    let mut metadata = serde_json::json!({
        "resource": resource_url(&state, &req, https),
        "authorization_servers": [state.auth.issuer().unwrap_or_default()],
        "bearer_methods_supported": ["header"],
        "resource_name": "Kabacus",
    });
    if !state.config.mcp.scopes.is_empty() {
        metadata["scopes_supported"] = serde_json::json!(state.config.mcp.scopes);
    }
    Json(metadata).into_response()
}

/// The tool list each client (user and client id) last listed, kept in
/// memory, so a client that comes back after the tools changed is told at
/// once; and a signal to every open `subscriptions/listen` when they
/// change.
#[derive(Default)]
pub struct ToolLists {
    seen: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    changed: std::sync::OnceLock<tokio::sync::watch::Sender<u64>>,
}

impl ToolLists {
    pub fn new() -> ToolLists {
        ToolLists::default()
    }

    fn changed(&self) -> &tokio::sync::watch::Sender<u64> {
        self.changed
            .get_or_init(|| tokio::sync::watch::channel(0).0)
    }

    fn key(principal: &crate::auth::roles::Principal) -> String {
        format!("{}\u{1f}{}", principal.username, principal.client)
    }

    /// The hash of the tool list `principal`'s client last listed.
    pub fn seen(&self, principal: &crate::auth::roles::Principal) -> Option<String> {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&Self::key(principal))
            .cloned()
    }

    pub fn set_seen(&self, principal: &crate::auth::roles::Principal, hash: &str) {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(Self::key(principal), hash.to_string());
    }

    /// Tell every open subscription the tools changed.
    pub fn notify_all(&self) {
        self.changed().send_modify(|n| *n += 1);
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed().subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTP/2 clients send no `Host`: the port must come from the URI, or
    /// the advertised resource (and its metadata) points at the wrong port.
    #[test]
    fn the_host_comes_from_the_host_header_or_the_http2_authority() {
        let http1 = axum::http::Request::get("/mcp")
            .header("host", "books.lan:8443")
            .body(())
            .unwrap();
        assert_eq!(request_host(&http1), "books.lan:8443");
        let http2 = axum::http::Request::get("https://books.lan:8443/mcp")
            .version(axum::http::Version::HTTP_2)
            .body(())
            .unwrap();
        assert_eq!(request_host(&http2), "books.lan:8443");
        let neither = axum::http::Request::get("/mcp").body(()).unwrap();
        assert_eq!(request_host(&neither), "localhost");
    }
    #[test]
    fn metadata_url_inserts_the_well_known_prefix_before_the_path() {
        assert_eq!(
            metadata_url("https://books.example.lan:8443/mcp"),
            "https://books.example.lan:8443/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            metadata_url("https://books.example.lan"),
            "https://books.example.lan/.well-known/oauth-protected-resource"
        );
    }
}
