// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `/mcp` is authenticated exactly like the API. Bearer tokens
//! from the configured provider (here kabacus-testkit's stand-in), roles
//! from the same `role_map`, and a 401 that points MCP clients at the
//! provider through RFC 9728 protected resource metadata.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kabacus_server::config::{AuthConfig, AuthMode};
use kabacus_server::mcp::REWRITE_TOOLS;
use kabacus_store::BookStore;
use kabacus_testkit::provider::{FakeProvider, ProviderOptions};
use kabacus_testkit::{spawn_oauth_server, spawn_remote_server, spawn_server, TestServer};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use tower::ServiceExt as _;

type Client = RunningService<RoleClient, ()>;

async fn store_head(server: &TestServer) -> String {
    server
        .state
        .store
        .revision(server.state.ledger)
        .await
        .unwrap()
        .to_string()
}

async fn connect(server: &TestServer, token: &str) -> Result<Client, String> {
    let config =
        StreamableHttpClientTransportConfig::with_uri("http://localhost/mcp").auth_header(token);
    let transport = StreamableHttpClientTransport::from_unix_socket_with_config(
        server.socket_path.to_str().unwrap(),
        config,
    );
    ().serve(transport).await.map_err(|e| e.to_string())
}

async fn call(client: &Client, name: &'static str, args: Value) -> Result<Value, String> {
    let Value::Object(args) = args else {
        panic!("expected an object")
    };
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(args))
        .await
        .map(|r| r.structured_content.expect("structured content"))
        .map_err(|e| e.to_string())
}

/// A request straight to the server's router, as `Host: books.example.lan`.
async fn send(server: &TestServer, request: Request<Body>) -> (StatusCode, String, Value) {
    let response = kabacus_server::api::router(server.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let challenge = response
        .headers()
        .get("www-authenticate")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        challenge,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

fn initialize(authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::post("/mcp")
        .header("host", "books.example.lan")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    builder
        .body(Body::from(
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "test", "version": "0"}}
            })
            .to_string(),
        ))
        .unwrap()
}
#[tokio::test]
async fn an_unauthenticated_client_is_pointed_at_the_issuer() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;

    let (status, challenge, _) = send(&server, initialize(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        challenge,
        "Bearer resource_metadata=\"http://books.example.lan/.well-known/oauth-protected-resource/mcp\""
    );

    let (status, challenge, _) = send(&server, initialize(Some("Bearer not-a-jwt"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        challenge.ends_with(", error=\"invalid_token\""),
        "{challenge}"
    );

    let metadata = Request::get("/.well-known/oauth-protected-resource/mcp")
        .header("host", "books.example.lan")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&server, metadata).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resource"], "http://books.example.lan/mcp");
    assert_eq!(body["authorization_servers"], json!([provider.issuer]));
    assert_eq!(body["bearer_methods_supported"], json!(["header"]));

    // A valid token gets through the same request.
    let token = provider.mint("alex", "zed-editor");
    let (status, _, _) = send(&server, initialize(Some(&format!("Bearer {token}")))).await;
    assert_eq!(status, StatusCode::OK);

    // A browser page can't use one: any request with an Origin is refused.
    let mut from_browser = initialize(Some(&format!("Bearer {token}")));
    from_browser
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let (status, _, _) = send(&server, from_browser).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn an_expired_token_is_refused() {
    let provider = FakeProvider::start(ProviderOptions {
        token_lifetime_secs: -600,
        ..Default::default()
    })
    .await;
    let server = spawn_oauth_server(&provider).await;
    let Err(err) = connect(&server, &provider.mint("alex", "zed-editor")).await else {
        panic!("an expired token must not connect");
    };
    assert!(err.contains("Auth required"), "{err}");
}
#[tokio::test]
async fn writes_are_attributed_to_the_signed_in_user_via_mcp() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let client = connect(&server, &provider.mint("alex", "zed-editor"))
        .await
        .unwrap();

    let head = store_head(&server).await;
    call(
        &client,
        "add_transaction",
        json!({
            "date": "2026-02-01",
            "description": "Grocery Store",
            "postings": [
                {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                {"account": "assets:checking"}
            ],
            "head": head,
            "dry_run": false,
        }),
    )
    .await
    .unwrap();

    let last = server
        .state
        .store
        .history(server.state.ledger, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(last.actor, "mcp");
    assert_eq!(last.user, "alex");
    assert_eq!(last.details["client"], "zed-editor");
    assert_eq!(last.details["issuer"], provider.issuer.as_str());
    client.cancel().await.unwrap();
}

/// A server with every role mapped and rewrites allowed, against
/// `provider`.
async fn spawn_permissive_server(provider: &FakeProvider) -> TestServer {
    let issuer = provider.issuer.clone();
    spawn_server(move |cfg| {
        cfg.auth = AuthConfig {
            mode: AuthMode::Oauth,
            issuer: Some(issuer),
            audience: Some("kabacus-server".into()),
            role_map: ["ledger-read", "ledger-write", "ledger-rewrite"]
                .map(|r| (r.to_string(), r.to_string()))
                .into(),
            ..Default::default()
        };
    })
    .await
}

/// The token's roles decide, as for the API: read-only can't write, and
/// the rewrite tools appear only for ledger-rewrite.
#[tokio::test]
async fn the_tokens_roles_decide_what_mcp_may_do() {
    let reader = FakeProvider::start(ProviderOptions {
        roles: vec!["ledger-read".into()],
        ..Default::default()
    })
    .await;
    let server = spawn_permissive_server(&reader).await;
    let client = connect(&server, &reader.mint("sam", "zed-editor"))
        .await
        .unwrap();
    assert!(call(&client, "list_accounts", json!({})).await.is_ok());
    let head = store_head(&server).await;
    let err = call(
        &client,
        "set_cleared",
        json!({"id": 1, "account": "assets:checking", "status": "cleared", "head": head}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("ledger-write"), "{err}");
    let tools = client.list_all_tools().await.unwrap();
    assert!(!tools
        .iter()
        .any(|t| REWRITE_TOOLS.contains(&t.name.as_ref())));
    client.cancel().await.unwrap();

    let owner = FakeProvider::start(ProviderOptions {
        roles: vec![
            "ledger-read".into(),
            "ledger-write".into(),
            "ledger-rewrite".into(),
        ],
        ..Default::default()
    })
    .await;
    let server = spawn_permissive_server(&owner).await;
    let client = connect(&server, &owner.mint("alex", "zed-editor"))
        .await
        .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    for tool in REWRITE_TOOLS {
        assert!(tools.iter().any(|t| t.name == *tool), "{tool} missing");
    }
    client.cancel().await.unwrap();
}

/// No OAuth, so no metadata to discover.
#[tokio::test]
async fn no_auth_mode_publishes_no_resource_metadata() {
    let server = spawn_server(|_| {}).await;
    let metadata = Request::get("/.well-known/oauth-protected-resource/mcp")
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&server, metadata).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `/mcp` is on the remote listener too, advertising an https:// resource.
#[tokio::test]
async fn the_remote_listener_serves_mcp_over_https() {
    let remote = spawn_remote_server().await;
    let http = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(remote.pki.ca_pem.as_bytes()).unwrap())
        .build()
        .unwrap();

    let refused = http
        .post(format!("{}/mcp", remote.url))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    let challenge = refused.headers()["www-authenticate"].to_str().unwrap();
    let expected = format!("{}/.well-known/oauth-protected-resource/mcp", remote.url);
    assert!(challenge.contains(&expected), "{challenge}");

    let metadata: Value = http
        .get(&expected)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(metadata["resource"], format!("{}/mcp", remote.url));
    assert_eq!(
        metadata["authorization_servers"],
        json!([remote.provider.issuer])
    );
}
