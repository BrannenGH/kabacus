// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use kabacus_client::{control::ControlChannel, LocalBooks};
use kabacus_service::local::LocalCaller;
use kabacus_store::BookStore;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[tokio::test]
async fn attached_agent_uses_the_open_books_and_records_its_real_name() {
    let folder = tempfile::tempdir().unwrap();
    let books = LocalBooks::create(
        folder.path().join("Household.kabacus"),
        "Cedar Household",
        "USD",
        "assets:checking",
        LocalCaller::new("Dana Okafor", "app"),
    )
    .await
    .unwrap();
    let channel = ControlChannel::start(books.clone(), 0).await.unwrap();
    let stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, channel.port))
        .await
        .unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read).lines();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"Cedar bookkeeping assistant","version":"1.0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_transaction","arguments":{"head":"2","dry_run":false,"date":"2026-11-05","description":"Cedar Design payroll","postings":[{"account":"assets:checking","amount":"3200.00"},{"account":"income:earnings","amount":"-3200.00"}]}}}),
    ];
    for request in messages {
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        if request.get("id").is_none() {
            continue;
        }
        loop {
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            let response: Value = serde_json::from_str(&response).unwrap();
            if response["id"] == request["id"] {
                assert!(response.get("error").is_none(), "{response}");
                assert_ne!(response["result"]["isError"], true, "{response}");
                break;
            }
        }
    }
    let history = books
        .service
        .store
        .history(books.service.ledger, 1)
        .await
        .unwrap();
    assert_eq!(history[0].actor, "mcp");
    assert_eq!(
        history[0].user,
        "Dana Okafor via Cedar bookkeeping assistant (agent)"
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while channel.activity().changed.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let activity = channel.activity();
    assert_eq!(
        activity.agent.as_deref(),
        Some("Cedar bookkeeping assistant")
    );
    assert_eq!(activity.changed.len(), 1);
    assert!(activity.expires_at > 0);
    let port = channel.port;
    drop(channel);
    tokio::task::yield_now().await;
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_err()
    );
}
