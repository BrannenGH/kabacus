// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::local::LocalCaller;
use crate::state::{KindTable, Settings};
use kabacus_store::{AnyStore, SqliteStore, Who};
use std::sync::atomic::{AtomicUsize, Ordering};

const TARGET: &str = "assets:bank:checking";
const SECOND: &str = "assets:bank:savings";
const CLAIM: &str = "https://bridge.simplefin.org/simplefin/claim/cedar-test";
const ACCESS: &str = "https://cedar-user:cedar-private@bridge.simplefin.org/simplefin";

#[derive(Default)]
struct Bridge {
    claimed: AtomicUsize,
    fetched: AtomicUsize,
    synced: AtomicUsize,
}

impl Transport for Bridge {
    fn send(&self, request: Request) -> Reply<'_> {
        Box::pin(async move {
            assert!(request.url.username().is_empty());
            if request.claim {
                assert_eq!(request.url.as_str(), CLAIM);
                assert!(request.authorization.is_none());
                return Ok(if self.claimed.fetch_add(1, Ordering::Relaxed) == 0 {
                    Response {
                        status: 200,
                        body: format!("  {ACCESS}\n").into_bytes(),
                    }
                } else {
                    Response {
                        status: 403,
                        body: b"Already claimed".to_vec(),
                    }
                });
            }
            assert_eq!(request.url.path(), "/simplefin/accounts");
            assert_eq!(
                request.authorization.as_deref(),
                Some(format!("Basic {}", STANDARD.encode("cedar-user:cedar-private")).as_str())
            );
            let query: BTreeMap<_, _> = request.url.query_pairs().into_owned().collect();
            assert_eq!(query.get("version").map(String::as_str), Some("2"));
            self.fetched.fetch_add(1, Ordering::Relaxed);
            let syncing = query.contains_key("start-date");
            if syncing {
                self.synced.fetch_add(1, Ordering::Relaxed);
                assert!(query.contains_key("end-date"));
                assert!(!query.contains_key("account") && !query.contains_key("pending"));
            } else {
                assert_eq!(query.get("balances-only").map(String::as_str), Some("1"));
            }
            let rows = if syncing {
                json!([
                    {"id":"HH 2386","posted":1792022400,"amount":"-18.40","payee":"Harbor Street Cafe","description":"Lunch order 1864","memo":"BANK MEMO", "mcc":"5812"},
                    {"id":"HH 2387","posted":0,"amount":"-12.10","pending":true,"description":"Oak Bakery"}
                ])
            } else {
                json!([])
            };
            let body = json!({ "connections": [{"conn_id":"Cedar Bank","name":"Cedar Bank"},{"conn_id":"Harbor Bank","name":"Harbor Bank"}],
                "errlist": [{"code":"gen.api","msg": format!("Window warning: {ACCESS}")},{"code":"act.missingdata","msg":"Listing is incomplete","account_id":"Everyday Account"}],
                "accounts": [
                {"id":"Everyday Account","conn_id":"Cedar Bank","name":"Checking","currency":"USD","balance":"-18.40","balance-date":1792022400,"transactions": rows, "holdings":[]},
                {"id":"Everyday Account","conn_id":"Harbor Bank","name":"Savings","currency":"USD","balance":"-18.40","balance-date":1792022400,"transactions": rows}
            ]});
            Ok(Response {
                status: 200,
                body: serde_json::to_vec(&body).unwrap(),
            })
        })
    }
}

async fn scratch(per_member: bool) -> (Service, LocalCaller, LocalCaller) {
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    let owner = store.local_user("Dana Okafor").await.unwrap();
    let other = store.local_user("Riley Chen").await.unwrap();
    let ledger = store.create_ledger("Cedar household", owner).await.unwrap();
    store
        .set_membership(ledger, other, Some(kabacus_store::Role::Write))
        .await
        .unwrap();
    let source = "/scratch/household.journal".into();
    let journal = kabacus_ledger::MemorySource(BTreeMap::from([(
        source,
        format!("commodity USD1.00\naccount {TARGET}\naccount {SECOND}\n"),
    )]));
    let book = kabacus_ledger::import(&journal, std::path::Path::new("/scratch/household.journal"))
        .unwrap()
        .book;
    store
        .import(
            ledger,
            &book,
            &Who {
                user: owner,
                actor: "cli".into(),
                kind: "new".into(),
                summary: "New household books".into(),
                details: json!({}),
            },
        )
        .await
        .unwrap();
    let state = Service::new(
        store,
        ledger,
        Settings {
            simplefin_per_member: per_member,
            ..Default::default()
        },
        KindTable {
            rules: kabacus_core::default_kind_rules(),
            reserve_template: kabacus_core::DEFAULT_RESERVE_TEMPLATE.into(),
        },
    );
    (
        state,
        LocalCaller::new("Dana Okafor", "cli"),
        LocalCaller::new("Riley Chen", "cli"),
    )
}

fn body(value: Value) -> SyncBody {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn linked_simplefin_accounts_commit_one_snapshot_without_a_second_fetch() {
    let (state, dana, _) = scratch(false).await;
    let mock = Bridge::default();
    let connect_body = || ConnectBody {
        token: STANDARD.encode(CLAIM),
        replace: true,
        dry_run: false,
    };
    let accounts = connect(&state, &dana, &mock, connect_body()).await.unwrap();
    assert_eq!(accounts["accounts"].as_array().unwrap().len(), 2);
    assert!(!accounts.to_string().contains(ACCESS));
    assert_eq!(
        connect(&state, &dana, &mock, connect_body())
            .await
            .unwrap_err()
            .code(),
        "simplefin_token_used"
    );
    let mut head = state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string();
    for (target, conn) in [(TARGET, "Cedar Bank"), (SECOND, "Harbor Bank")] {
        let result = link(
            &state,
            &dana,
            &mock,
            target,
            &head,
            LinkBody {
                remote: Some(RemoteAccount {
                    conn_id: conn.into(),
                    id: "Everyday Account".into(),
                }),
            },
        )
        .await
        .unwrap();
        head = result["head"].as_str().unwrap().to_string();
    }
    let mut request = json!({"since": "2026-10-01", "end_date":"2026-11-01"});
    let review = sync(&state, &dana, &mock, &head, body(request.clone()))
        .await
        .unwrap();
    assert_eq!(
        review["accounts"][0]["rows"][0]["description"],
        "Harbor Street Cafe | Lunch order 1864"
    );
    assert_eq!(review["accounts"][0]["pending"], 1);
    assert_eq!(review["counts"]["new"], 2);
    request["snapshot"] = review["snapshot"].clone();
    request["dry_run"] = json!(false);
    let saved = sync(&state, &dana, &mock, &head, body(request.clone()))
        .await
        .unwrap();
    assert_eq!(
        saved["head"].as_str().unwrap().parse::<u64>().unwrap(),
        head.parse::<u64>().unwrap() + 1
    );
    assert_eq!(mock.fetched.load(Ordering::Relaxed), 2);
    assert_eq!(mock.synced.load(Ordering::Relaxed), 1);
    let repeated = sync(
        &state,
        &dana,
        &mock,
        saved["head"].as_str().unwrap(),
        body(request),
    )
    .await
    .unwrap();
    assert_eq!(repeated["head"], saved["head"]);
    assert!(repeated["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["counts"]["duplicate"] == 1));
    assert!(!saved.to_string().contains(ACCESS));
    assert!(!state
        .store
        .history(state.ledger, 20)
        .await
        .unwrap()
        .iter()
        .any(|row| row.details.to_string().contains(ACCESS)));
}

#[tokio::test]
async fn private_claim_addresses_and_other_members_snapshots_are_refused() {
    let (state, dana, riley) = scratch(true).await;
    let mock = Bridge::default();
    let connect_body = |url| ConnectBody {
        token: STANDARD.encode(url),
        replace: false,
        dry_run: false,
    };
    for url in [
        "http://bridge.simplefin.org/simplefin/claim/cedar",
        "https://127.0.0.1/simplefin/claim/cedar",
        "https://[fc00::1]/simplefin/claim/cedar",
    ] {
        assert_eq!(
            connect(&state, &dana, &mock, connect_body(url))
                .await
                .unwrap_err()
                .code(),
            "simplefin_url"
        );
    }
    assert_eq!(mock.claimed.load(Ordering::Relaxed), 0);
    connect(&state, &dana, &mock, connect_body(CLAIM))
        .await
        .unwrap();
    assert_eq!(
        connection(&state, &riley).await.unwrap(),
        json!({"connected":false})
    );
    let riley_id = state.user_of(&riley).await.unwrap();
    state
        .store
        .set_simplefin_access(
            state.ledger,
            Owner::Member(riley_id),
            &Access::new(
                ACCESS.replace("cedar-user", "riley-user"),
                Utc::now().timestamp(),
            ),
        )
        .await
        .unwrap();
    let head = state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string();
    let reviewed = sync(
        &state,
        &dana,
        &mock,
        &head,
        body(json!({"since":"2026-10-01", "end_date":"2026-11-01"})),
    )
    .await
    .unwrap();
    let invalid = sync(
        &state,
        &riley,
        &mock,
        &head,
        body(json!({"dry_run":false, "snapshot":reviewed["snapshot"]})),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code(), "snapshot_expired");
    let before = mock.fetched.load(Ordering::Relaxed);
    let long = sync(
        &state,
        &dana,
        &mock,
        &head,
        body(json!({"since":"2026-06-01", "end_date":"2026-11-01"})),
    )
    .await
    .unwrap_err();
    assert_eq!(long.code(), "simplefin_window");
    assert_eq!(mock.fetched.load(Ordering::Relaxed), before);
}
