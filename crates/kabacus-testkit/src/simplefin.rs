// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{spawn_scratch, TestServer};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use kabacus_server::config::Config;
use kabacus_service::simplefin::{Reply, Request, Response, Transport};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CLAIM_URL: &str = "https://bridge.simplefin.org/simplefin/claim/cedar-demo";

const HARBOR_CLAIM_URL: &str = "https://bridge.simplefin.org/simplefin/claim/harbor-demo";
const CEDAR_AUTH: &str = "cedar-demo:harbor-birch";
const HARBOR_AUTH: &str = "harbor-demo:cedar-maple";

pub fn harbor_token() -> String {
    STANDARD.encode(HARBOR_CLAIM_URL)
}

pub fn token() -> String {
    STANDARD.encode(CLAIM_URL)
}

pub async fn books_requiring_payment_on_first_fetch() -> TestServer {
    books_with_bridge(
        |_| {},
        Bridge {
            unpaid: AtomicBool::new(true),
        },
    )
    .await
}

pub async fn books_with(customize: impl FnOnce(&mut Config)) -> TestServer {
    books_with_bridge(customize, Bridge::default()).await
}

async fn books_with_bridge(customize: impl FnOnce(&mut Config), bridge: Bridge) -> TestServer {
    spawn_scratch(
        "account assets:bank:checking  ; ynab-account:Cedar Checking\n\
         account assets:bank:savings  ; ynab-account:Harbor Savings\n\
         account liabilities:credit-card:cedar  ; ynab-account:Cedar Rewards\n\
         account assets:bank:checking:card-cedar\n",
        |cfg| {
            cfg.import.simplefin.enabled = true;
            customize(cfg);
        },
        Some(Arc::new(bridge)),
    )
    .await
}

#[derive(Default)]
struct Bridge {
    unpaid: AtomicBool,
}

impl Transport for Bridge {
    fn send(&self, request: Request) -> Reply<'_> {
        Box::pin(async move {
            if request.claim {
                let credentials = match request.url.as_str() {
                    CLAIM_URL => CEDAR_AUTH,
                    HARBOR_CLAIM_URL => HARBOR_AUTH,
                    _ => panic!("Unexpected fixture claim"),
                };
                return Ok(Response {
                    status: 200,
                    body: format!("https://{credentials}@bridge.simplefin.org/simplefin")
                        .into_bytes(),
                });
            }

            assert_eq!(request.url.path(), "/simplefin/accounts");
            assert!(request
                .url
                .query_pairs()
                .any(|(key, value)| key == "version" && value == "2"));
            let (bank, account, payee) = match request.authorization.as_deref() {
                Some(auth) if auth == format!("Basic {}", STANDARD.encode(CEDAR_AUTH)) => {
                    ("Cedar Bank", "Cedar Everyday", "Harbor Street Cafe")
                }
                Some(auth) if auth == format!("Basic {}", STANDARD.encode(HARBOR_AUTH)) => {
                    ("Harbor Bank", "Harbor Everyday", "Red Maple Books")
                }
                _ => panic!("Unexpected fixture credentials"),
            };
            if self.unpaid.swap(false, Ordering::Relaxed) {
                return Ok(Response {
                    status: 402,
                    body: br#"{"errlist":[{"code":"gen.","msg":"Payment required."}],"accounts":[],"connections":[]}"#.to_vec(),
                });
            }
            let rows = if request
                .url
                .query_pairs()
                .any(|(key, _)| key == "start-date")
            {
                json!([{
                    "id": "HH 2386",
                    "posted": chrono::Utc::now().timestamp() - 86_400,
                    "amount": "-18.40",
                    "description": payee,
                    "payee": payee
                }])
            } else {
                json!([])
            };
            Ok(Response {
                status: 200,
                body: serde_json::to_vec(&json!({
                    "connections": [{"conn_id": bank, "name": bank}],
                    "errlist": [],
                    "accounts": [{
                        "id": account,
                        "conn_id": bank,
                        "name": account,
                        "currency": "USD",
                        "balance": "981.60",
                        "balance-date": chrono::Utc::now().timestamp(),
                        "transactions": rows
                    }]
                }))
                .unwrap(),
            })
        })
    }
}
