// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Driving `/mcp` the way an MCP client does, for the e2e tests.

#![allow(dead_code)]

use kabacus_store::BookStore;
use kabacus_testkit::TestServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

pub type Client = RunningService<RoleClient, ()>;

pub async fn connect(server: &TestServer) -> Client {
    let config = StreamableHttpClientTransportConfig::with_uri("http://localhost/mcp");
    let transport = StreamableHttpClientTransport::from_unix_socket_with_config(
        server.socket_path.to_str().unwrap(),
        config,
    );
    ().serve(transport).await.unwrap()
}

pub fn object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => panic!("expected a JSON object"),
    }
}

/// A tool's structured result, or its error's data (`{code, message, ...}`).
pub async fn try_call(client: &Client, name: &str, args: Value) -> Result<Value, Value> {
    match client
        .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(object(args)))
        .await
    {
        Ok(r) => Ok(r.structured_content.expect("structured content")),
        Err(rmcp::service::ServiceError::McpError(e)) => {
            Err(e.data.unwrap_or_else(|| json!({"message": e.message})))
        }
        Err(other) => panic!("{name}: {other}"),
    }
}

pub async fn call(client: &Client, name: &str, args: Value) -> Value {
    try_call(client, name, args)
        .await
        .unwrap_or_else(|e| panic!("{name} failed: {e}"))
}

/// The error data of a call that must fail.
pub async fn fail(client: &Client, name: &str, args: Value) -> Value {
    match try_call(client, name, args).await {
        Ok(v) => panic!("{name} should have failed, got {v}"),
        Err(e) => e,
    }
}

/// Call a write tool with `dry_run: false`, returning its result and the
/// new head.
pub async fn commit(client: &Client, name: &str, mut args: Value) -> (Value, String) {
    args["dry_run"] = json!(false);
    let result = call(client, name, args).await;
    let head = result["head"].as_str().unwrap().to_string();
    (result, head)
}

/// Commit a rewrite in one call, returning its result.
pub async fn rewrite(client: &Client, name: &str, mut args: Value) -> Value {
    args["dry_run"] = json!(false);
    call(client, name, args).await
}

pub async fn head(client: &Client) -> String {
    call(client, "get_version", json!({})).await["head"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn exact(value: &Value) -> &str {
    value["exact"]
        .as_str()
        .unwrap_or_else(|| panic!("no exact in {value}"))
}

/// A server whose rewrite gates are open for the local principal.
pub fn rewrites(cfg: &mut kabacus_server::config::Config) {
    cfg.auth.no_auth_roles = vec![
        "ledger-read".to_string(),
        "ledger-write".to_string(),
        "ledger-rewrite".to_string(),
    ];
}

/// The books as the store holds them now.
pub async fn book(server: &TestServer) -> kabacus_ledger::Book {
    server
        .state
        .store
        .load(server.state.ledger, None)
        .await
        .unwrap()
        .book
}

/// The books rendered as a journal.
pub async fn journal(server: &TestServer) -> String {
    kabacus_ledger::render_book(&book(server).await)
}

/// The books pass the strict check.
pub async fn assert_checks(server: &TestServer) {
    let failures = kabacus_ledger::check(&book(server).await);
    assert!(
        failures.is_empty(),
        "{:?}",
        failures.iter().map(|f| f.message()).collect::<Vec<_>>()
    );
}

/// What the audit log records for the write that made `revision`.
pub async fn change(server: &TestServer, revision: &str) -> kabacus_store::ChangeInfo {
    server
        .state
        .store
        .change(server.state.ledger, revision.parse().unwrap())
        .await
        .unwrap()
        .info
}

/// The ledger's revision: how many writes it has had.
pub async fn revision(server: &TestServer) -> i64 {
    server
        .state
        .store
        .revision(server.state.ledger)
        .await
        .unwrap()
}

/// The ledger's revision as the `head` writes are made against.
pub async fn store_head(server: &TestServer) -> String {
    revision(server).await.to_string()
}

/// `$KABACUS_HLEDGER` or `hledger` on the `PATH`, for comparing reports
/// with it; `None` skips the comparison unless `KABACUS_REQUIRE_HLEDGER`
/// is set.
pub fn hledger() -> Option<String> {
    let binary = std::env::var("KABACUS_HLEDGER").unwrap_or_else(|_| "hledger".to_string());
    let found = std::process::Command::new(&binary)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if found {
        return Some(binary);
    }
    if std::env::var_os("KABACUS_REQUIRE_HLEDGER").is_some() {
        panic!("hledger ({binary}) is required but can't be run");
    }
    eprintln!("hledger not found: skipping the comparison with it");
    None
}
