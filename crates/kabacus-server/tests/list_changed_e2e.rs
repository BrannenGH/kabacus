// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: the server declares tools.listChanged, tells a
//! client whose tool list is out of date (after a restart on a new
//! version, or a change while it is connected), and reports a stable
//! tool_list_hash.

mod support;

use kabacus_server::auth::none::no_auth_principal;
use kabacus_testkit::{spawn_server, TestServer};
use rmcp::model::{
    ClientCapabilities, ClientConfig, Implementation, ProtocolVersion, ServerNotification,
    SubscriptionFilter,
};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, NotificationContext};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::json;
use std::time::Duration;
use support::*;

/// What a client last saw is, as far as the server knows, an older build's
/// tool list.
fn pretend_an_older_build_was_listed(server: &TestServer) {
    let principal = no_auth_principal(&[], Some("mcp"));
    server.state.tool_lists.set_seen(&principal, "older-build");
}
#[tokio::test]
async fn the_server_declares_list_changed_and_its_version() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let info = client.peer_info().expect("server info");
    let tools = info.capabilities.tools.clone().expect("tools capability");
    assert_eq!(tools.list_changed, Some(true));
    let version = call(&client, "get_version", json!({})).await;
    let server_info = info.server_info.clone().expect("serverInfo");
    assert_eq!(server_info.name, "kabacus-server");
    assert_eq!(json!(server_info.version), version["server_version"]);
    client.cancel().await.unwrap();
}

/// A client on the 2026-07-28 protocol: it starts with `server/discover`
/// and listens for notifications with `subscriptions/listen`.
async fn connect_current(server: &TestServer) -> rmcp::service::RunningService<RoleClient, ()> {
    let transport = StreamableHttpClientTransport::from_unix_socket_with_config(
        server.socket_path.to_str().unwrap(),
        StreamableHttpClientTransportConfig::with_uri("http://localhost/mcp"),
    );
    ().serve_with_lifecycle(
        transport,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await
    .unwrap()
}

/// A client that comes back while its tool list is out of date hears so as
/// soon as it listens; one that just listed tools doesn't; a change while
/// it listens is sent to it.
#[tokio::test]
async fn a_returning_client_hears_its_tools_changed() {
    let server = spawn_server(|_| {}).await;
    let first = connect_current(&server).await;
    first.list_all_tools().await.unwrap();
    first.cancel().await.unwrap();

    pretend_an_older_build_was_listed(&server);
    let client = connect_current(&server).await;
    let mut subscription = client
        .peer()
        .listen(SubscriptionFilter::builder().tools_list_changed().build())
        .await
        .unwrap();
    let notification = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a notification within 5 seconds")
        .unwrap();
    assert!(
        matches!(
            notification,
            Some(ServerNotification::ToolListChangedNotification(_))
        ),
        "{notification:?}"
    );

    // Listing the tools brings it up to date: no notification now...
    client.list_all_tools().await.unwrap();
    let mut quiet = client
        .peer()
        .listen(SubscriptionFilter::builder().tools_list_changed().build())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), quiet.next())
            .await
            .is_err()
    );
    // ...until the tools change while it listens.
    server.state.tool_lists.notify_all();
    let notification = tokio::time::timeout(Duration::from_secs(5), quiet.next())
        .await
        .expect("a notification within 5 seconds")
        .unwrap();
    assert!(matches!(
        notification,
        Some(ServerNotification::ToolListChangedNotification(_))
    ));
    client.cancel().await.unwrap();
}

/// Records tools/list_changed, as a client on an older protocol would.
struct Recorder(tokio::sync::mpsc::UnboundedSender<()>);

impl ClientHandler for Recorder {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("recorder", "1"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        let _ = self.0.send(());
    }
}

/// A client on a protocol without subscriptions hears it with its next
/// call.
#[tokio::test]
async fn a_client_on_an_older_protocol_hears_it_with_its_next_call() {
    let server = spawn_server(|_| {}).await;
    pretend_an_older_build_was_listed(&server);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let transport = StreamableHttpClientTransport::from_unix_socket_with_config(
        server.socket_path.to_str().unwrap(),
        StreamableHttpClientTransportConfig::with_uri("http://localhost/mcp"),
    );
    let client = Recorder(tx).serve(transport).await.unwrap();
    client
        .call_tool(rmcp::model::CallToolRequestParams::new("get_version"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a notification within 5 seconds");
    client.cancel().await.unwrap();
}

/// tool_list_hash is the same for the same build, across restarts.
#[tokio::test]
async fn the_tool_list_hash_is_stable_across_restarts() {
    let a = spawn_server(|_| {}).await;
    let b = spawn_server(|_| {}).await;
    let ca = connect(&a).await;
    let cb = connect(&b).await;
    let ha = call(&ca, "get_version", json!({})).await["tool_list_hash"].clone();
    let hb = call(&cb, "get_version", json!({})).await["tool_list_hash"].clone();
    assert!(ha.as_str().is_some_and(|h| h.len() == 16), "{ha}");
    assert_eq!(ha, hb);
    assert_eq!(ha, json!(kabacus_server::mcp::tool_list_hash()));
    ca.cancel().await.unwrap();
    cb.cancel().await.unwrap();
}
