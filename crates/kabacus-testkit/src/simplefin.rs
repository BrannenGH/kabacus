// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{spawn_scratch, TestServer};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use kabacus_service::simplefin::{Reply, Request, Response, Transport};
use serde_json::json;
use std::sync::Arc;

const CLAIM_URL: &str = "https://bridge.simplefin.org/simplefin/claim/cedar-demo";

pub fn token() -> String {
    STANDARD.encode(CLAIM_URL)
}

pub async fn books() -> TestServer {
    spawn_scratch(
        "",
        |cfg| cfg.import.simplefin.enabled = true,
        Some(Arc::new(Bridge)),
    )
    .await
}

struct Bridge;

impl Transport for Bridge {
    fn send(&self, request: Request) -> Reply<'_> {
        Box::pin(async move {
            if request.claim {
                assert_eq!(request.url.as_str(), CLAIM_URL);
                return Ok(Response {
                    status: 200,
                    body: b"https://cedar-demo:harbor-birch@bridge.simplefin.org/simplefin"
                        .to_vec(),
                });
            }

            assert_eq!(request.url.path(), "/simplefin/accounts");
            assert!(request
                .url
                .query_pairs()
                .any(|(key, value)| key == "version" && value == "2"));
            let rows = if request
                .url
                .query_pairs()
                .any(|(key, _)| key == "start-date")
            {
                json!([{
                    "id": "HH 2386",
                    "posted": chrono::Utc::now().timestamp() - 86_400,
                    "amount": "-18.40",
                    "payee": "Harbor Street Cafe"
                }])
            } else {
                json!([])
            };
            Ok(Response {
                status: 200,
                body: serde_json::to_vec(&json!({
                    "connections": [{"conn_id": "Cedar Bank", "name": "Cedar Bank"}],
                    "accounts": [{
                        "id": "Cedar Everyday",
                        "conn_id": "Cedar Bank",
                        "name": "Cedar Everyday",
                        "currency": "USD",
                        "balance": "981.60",
                        "transactions": rows
                    }]
                }))
                .unwrap(),
            })
        })
    }
}
