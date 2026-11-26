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
    unpaid: bool,
    recent: Option<(i64, bool)>,
    cedar_currency: Option<&'static str>,
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
            if self.unpaid {
                return Ok(Response {
                    status: 402,
                    body: br#"{"errlist":[{"code":"gen.","msg":"Payment required."}],"accounts":[],"connections":[]}"#.to_vec(),
                });
            }
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
            let rows = if syncing {
                if let Some((posted, extra)) = self.recent {
                    let mut recent = vec![
                        json!({"id":"HH 2388","posted":posted,"amount":"-18.40","description":"Harbor Street Cafe"}),
                    ];
                    if extra {
                        recent.push(json!({"id":"HH 2389","posted":posted + 86400,"amount":"-12.10","description":"Oak Bakery"}));
                    }
                    json!(recent)
                } else {
                    rows
                }
            } else {
                rows
            };
            let body = json!({ "connections": [{"conn_id":"Cedar Bank","name":"Cedar Bank"},{"conn_id":"Harbor Bank","name":"Harbor Bank"}],
                "errlist": [{"code":"gen.api","msg": format!("Window warning: {ACCESS}")},{"code":"act.missingdata","msg":"Listing is incomplete","account_id":"Everyday Account"}],
                "accounts": [
                {"id":"Everyday Account","conn_id":"Cedar Bank","name":"Checking","currency":self.cedar_currency.unwrap_or("USD"),"balance":"-18.40","balance-date":1792022400,"transactions": rows, "holdings":[]},
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
    scratch_with_history(per_member, "").await
}

async fn scratch_with_history(
    per_member: bool,
    history: &str,
) -> (Service, LocalCaller, LocalCaller) {
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
        format!("commodity USD1.00\naccount {TARGET}\naccount {SECOND}\n{history}"),
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
async fn payment_error_keeps_claim_for_refresh_and_rejects_unconfirmed_replacement() {
    let (state, dana, _) = scratch(false).await;
    let mut bridge = Bridge {
        unpaid: true,
        ..Default::default()
    };
    let setup = || ConnectBody {
        token: STANDARD.encode(CLAIM),
        replace: false,
        dry_run: false,
    };
    let failure = connect(&state, &dana, &bridge, setup()).await.unwrap_err();
    assert_eq!(failure.code(), "simplefin_payment");
    assert_eq!(failure.to_string(), "Sorry, SimpleFIN is showing that your account with them is not paid. A paid SimpleFIN account is required");
    assert_eq!(connection(&state, &dana).await.unwrap()["connected"], true);
    assert_eq!(
        connect(&state, &dana, &bridge, setup())
            .await
            .unwrap_err()
            .code(),
        "simplefin_connected"
    );
    bridge.unpaid = false;
    let result = list_accounts(&state, &dana, &bridge, true).await.unwrap();
    assert_eq!(result["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(bridge.claimed.load(Ordering::Relaxed), 1);
    assert_eq!(bridge.fetched.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn creates_and_links_a_card_in_one_revision() {
    let (state, dana, _) = scratch(false).await;
    let bridge = Bridge::default();
    connect(
        &state,
        &dana,
        &bridge,
        ConnectBody {
            token: STANDARD.encode(CLAIM),
            replace: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let head = state.store.revision(state.ledger).await.unwrap();
    let link_body = || {
        serde_json::from_value(json!({
            "create": "revolving-debt", "remote": {"conn_id":"Cedar Bank", "id":"Everyday Account"}
        }))
        .unwrap()
    };
    let target = "liabilities:credit-card:Cedar Rewards";
    assert!(link(
        &state,
        &dana,
        &bridge,
        TARGET,
        &head.to_string(),
        link_body()
    )
    .await
    .is_err());
    assert!(link(
        &state,
        &dana,
        &bridge,
        "liabilities:credit-card:Invalid; Name",
        &head.to_string(),
        link_body()
    )
    .await
    .is_err());
    assert_eq!(state.store.revision(state.ledger).await.unwrap(), head);
    let result = link(
        &state,
        &dana,
        &bridge,
        target,
        &head.to_string(),
        link_body(),
    )
    .await
    .unwrap();
    assert_eq!(result["head"], (head + 1).to_string());
    assert_eq!(result["linked"], true);
    assert_eq!(
        list_accounts(&state, &dana, &bridge, false).await.unwrap()["accounts"][0]["linked"],
        target
    );
    let review = sync(
        &state,
        &dana,
        &bridge,
        result["head"].as_str().unwrap(),
        body(json!({"since": "2026-10-01", "end_date": "2026-11-01"})),
    )
    .await
    .unwrap();
    assert_eq!(review["counts"]["new"], 1);
}

#[tokio::test]
async fn simplefin_imports_in_the_feed_currency_within_one_revision() {
    let (state, dana, _) = scratch_with_history(false, &format!(
        "commodity $1.00\naccount equity:opening\n\n2026-08-01 Opening\n    {SECOND}  $25.00\n    equity:opening\n"
    )).await;
    let bridge = Bridge {
        cedar_currency: Some("cad"),
        ..Bridge::default()
    };
    connect(&state, &dana, &bridge, setup_request())
        .await
        .unwrap();
    let mut head = state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string();
    for (target, bank) in [(TARGET, "Cedar Bank"), (SECOND, "Harbor Bank")] {
        let linked = link(&state, &dana, &bridge, target, &head, mapping(bank, None))
            .await
            .unwrap();
        head = linked["head"].as_str().unwrap().into();
    }

    let preview = sync(
        &state,
        &dana,
        &bridge,
        &head,
        body(json!({"since": "2026-10-01", "end_date": "2026-11-01"})),
    )
    .await
    .unwrap();
    assert_eq!(preview["counts"]["new"], 2);
    assert_eq!(preview["accounts"][0]["source"]["currency"], "CAD");
    assert_eq!(
        preview["accounts"][0]["balance_check"]["difference"],
        "0.00"
    );
    assert_eq!(preview["accounts"][1]["source"]["currency"], "USD");
    assert_eq!(preview["accounts"][1]["balance_check"]["books"], "6.60");
    let stored = state.books_snapshot().await.unwrap();
    assert_eq!(stored.key, head);
    assert!(!stored.book.commodities.contains_key("CAD"));

    let committed = sync(
        &state,
        &dana,
        &bridge,
        &head,
        body(json!({
            "since": "2026-10-01", "end_date": "2026-11-01",
            "snapshot": preview["snapshot"], "dry_run": false
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        committed["head"],
        (head.parse::<u64>().unwrap() + 1).to_string()
    );
    let stored = state.books_snapshot().await.unwrap();
    assert!(stored.book.commodities.contains_key("CAD"));
    for (target, currency) in [(TARGET, "CAD"), (SECOND, "$")] {
        let tx = stored
            .book
            .transactions
            .iter()
            .find(|tx| tx.postings[0].account.as_str() == target && tx.tag("import-id").is_some())
            .unwrap();
        assert!(tx.postings.iter().all(|p| p.amount.commodity == currency));
        assert_eq!(tx.postings[0].amount.money.to_string(), "-18.40");
    }
}

#[tokio::test]
async fn simplefin_custom_currency_cannot_be_linked() {
    let (state, dana, _) = scratch(false).await;
    let bridge = Bridge {
        cedar_currency: Some("https://cedar.example/currency/rewards"),
        ..Bridge::default()
    };
    connect(
        &state,
        &dana,
        &bridge,
        ConnectBody {
            token: STANDARD.encode(CLAIM),
            replace: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let head = state.store.revision(state.ledger).await.unwrap();
    let error = link(
        &state,
        &dana,
        &bridge,
        TARGET,
        &head.to_string(),
        serde_json::from_value(json!({
            "remote": {"conn_id": "Cedar Bank", "id": "Everyday Account"}
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "unsupported_currency");
    assert_eq!(state.store.revision(state.ledger).await.unwrap(), head);
}

#[tokio::test]
async fn automatic_sync_waits_for_review_and_observes_the_shared_request_interval() {
    let (state, dana, _) = scratch(false).await;
    let first = Utc::now().date_naive() - Days::new(2);
    let posted = first.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let mut bridge = Bridge {
        recent: Some((posted, false)),
        ..Default::default()
    };
    connect(
        &state,
        &dana,
        &bridge,
        ConnectBody {
            token: STANDARD.encode(CLAIM),
            replace: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let head = state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string();
    let linked = link(
        &state,
        &dana,
        &bridge,
        TARGET,
        &head,
        LinkBody {
            create: None,
            remote: Some(RemoteAccount {
                conn_id: "Cedar Bank".into(),
                id: "Everyday Account".into(),
            }),
        },
    )
    .await
    .unwrap();
    let head = linked["head"].as_str().unwrap();
    let automatic = || body(json!({"automatic": true}));
    assert_eq!(
        sync(&state, &dana, &bridge, head, automatic())
            .await
            .unwrap()["skipped"],
        "needs_review"
    );
    assert_eq!(bridge.fetched.load(Ordering::Relaxed), 1);

    let mut manual = json!({"since":first, "end_date":first.succ_opt().unwrap()});
    manual["snapshot"] = sync(&state, &dana, &bridge, head, body(manual.clone()))
        .await
        .unwrap()["snapshot"]
        .clone();
    manual["dry_run"] = json!(false);
    let committed = sync(&state, &dana, &bridge, head, body(manual))
        .await
        .unwrap();
    let head = committed["head"].as_str().unwrap();
    assert_eq!(
        sync(&state, &dana, &bridge, head, automatic())
            .await
            .unwrap()["skipped"],
        "recent_request"
    );
    assert_eq!(bridge.fetched.load(Ordering::Relaxed), 2);
    state
        .store
        .set_simplefin_access(
            state.ledger,
            Owner::Local,
            &Access::new(ACCESS.into(), Utc::now().timestamp() - 86400),
        )
        .await
        .unwrap();
    bridge.recent = Some((posted, true));
    let result = sync(&state, &dana, &bridge, head, automatic())
        .await
        .unwrap();
    assert_eq!(result["counts"]["new"], 1);
    assert_eq!(result["head"], head);
    assert_eq!(current_head(&state).await, head);
    state.bank_connections.clear(Owner::Local);
    assert_eq!(connection(&state, &dana).await.unwrap()["pending_count"], 1);
    assert_eq!(bridge.fetched.load(Ordering::Relaxed), 3);
    assert_eq!(
        sync(&state, &dana, &bridge, head, automatic())
            .await
            .unwrap()["skipped"],
        "recent_request"
    );
    let preview = sync(&state, &dana, &bridge, head, body(json!({"pending": true})))
        .await
        .unwrap();
    assert_eq!(preview["counts"]["new"], 1);
    assert_eq!(
        preview["accounts"][0]["rows"][0]["description"],
        "Oak Bakery"
    );
    let saved = sync(
        &state,
        &dana,
        &bridge,
        head,
        body(json!({"pending": true, "dry_run": false, "snapshot": preview["snapshot"]})),
    )
    .await
    .unwrap();
    assert_eq!(saved["counts"]["new"], 1);
    assert_ne!(saved["head"], head);
    assert_eq!(connection(&state, &dana).await.unwrap()["pending_count"], 0);
    assert_eq!(bridge.fetched.load(Ordering::Relaxed), 3);
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
                create: None,
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
    let mut unknown_account = request.clone();
    unknown_account["decisions"] = json!({
        "assets:bank:other": [{
            "import_id": review["accounts"][0]["rows"][0]["import_id"],
            "action": "skip"
        }]
    });
    assert_eq!(
        sync(&state, &dana, &mock, &head, body(unknown_account))
            .await
            .unwrap_err()
            .code(),
        "statement_import"
    );
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

#[tokio::test]
async fn a_books_writer_can_connect_on_first_visit() {
    let (state, dana, _) = scratch(true).await;
    let writer = LocalCaller::new("Drew Evans", "app");
    let user = state.user_of(&writer).await.unwrap();
    assert!(state.store.memberships(user).await.unwrap().is_empty());
    assert_eq!(
        connection(&state, &writer).await.unwrap(),
        json!({"connected":false})
    );
    let request = |dry_run| ConnectBody {
        token: STANDARD.encode(CLAIM),
        replace: false,
        dry_run,
    };
    let bridge = Bridge::default();
    connect(&state, &writer, &bridge, request(true))
        .await
        .unwrap();
    assert!(state.store.memberships(user).await.unwrap().is_empty());
    connect(&state, &writer, &bridge, request(false))
        .await
        .unwrap();
    assert_eq!(
        connection(&state, &writer).await.unwrap()["connected"],
        true
    );
    assert_eq!(
        connection(&state, &dana).await.unwrap(),
        json!({"connected":false})
    );
    state
        .store
        .set_membership(state.ledger, user, None)
        .await
        .unwrap();
    assert_eq!(
        connection(&state, &writer).await.unwrap(),
        json!({"connected":false})
    );
}

const CARD: &str = "liabilities:credit-card:Cedar Rewards";

fn setup_request() -> ConnectBody {
    ConnectBody {
        token: STANDARD.encode(CLAIM),
        replace: false,
        dry_run: false,
    }
}

fn mapping(bank: &str, create: Option<NewAccountKind>) -> LinkBody {
    LinkBody {
        create,
        remote: Some(RemoteAccount {
            conn_id: bank.into(),
            id: "Everyday Account".into(),
        }),
    }
}

async fn current_head(state: &Service) -> String {
    state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string()
}

async fn import_october(
    state: &Service,
    caller: &LocalCaller,
    bridge: &Bridge,
    head: &str,
) -> Value {
    let mut request = json!({"since":"2026-10-01", "end_date":"2026-11-01"});
    request["snapshot"] = sync(state, caller, bridge, head, body(request.clone()))
        .await
        .unwrap()["snapshot"]
        .clone();
    request["dry_run"] = json!(false);
    sync(state, caller, bridge, head, body(request))
        .await
        .unwrap()
}

#[tokio::test]
async fn switching_a_card_feed_starts_a_fresh_review_and_keeps_ids_feed_scoped() {
    let (state, dana, _) = scratch(false).await;
    let bridge = Bridge::default();
    connect(&state, &dana, &bridge, setup_request())
        .await
        .unwrap();
    link(
        &state,
        &dana,
        &bridge,
        CARD,
        &current_head(&state).await,
        mapping("Cedar Bank", Some(NewAccountKind::RevolvingDebt)),
    )
    .await
    .unwrap();
    let imported = import_october(&state, &dana, &bridge, &current_head(&state).await).await;
    assert_eq!(imported["counts"]["new"], 1);

    let linked = link(
        &state,
        &dana,
        &bridge,
        CARD,
        imported["head"].as_str().unwrap(),
        mapping("Harbor Bank", None),
    )
    .await
    .unwrap();
    let head = linked["head"].as_str().unwrap();
    let listings = list_accounts(&state, &dana, &bridge, false).await.unwrap();
    assert_eq!(listings["accounts"][0]["linked"], Value::Null);
    assert_eq!(listings["accounts"][1]["linked"], CARD);
    assert_eq!(
        sync(
            &state,
            &dana,
            &bridge,
            head,
            body(json!({"automatic":true}))
        )
        .await
        .unwrap()["skipped"],
        "needs_review"
    );

    let review = sync(
        &state,
        &dana,
        &bridge,
        head,
        body(json!({"end_date":"2026-11-01"})),
    )
    .await
    .unwrap();
    assert_eq!(review["accounts"][0]["since"], "2026-10-01");
    assert_eq!(review["counts"]["new"], 1);
    let saved = import_october(&state, &dana, &bridge, head).await;
    assert_eq!(saved["counts"]["new"], 1);

    let linked = link(
        &state,
        &dana,
        &bridge,
        CARD,
        saved["head"].as_str().unwrap(),
        mapping("Cedar Bank", None),
    )
    .await
    .unwrap();
    let back = import_october(&state, &dana, &bridge, linked["head"].as_str().unwrap()).await;
    assert_eq!(back["accounts"][0]["counts"]["duplicate"], 1);
    assert_eq!(back["head"], linked["head"]);
}

#[tokio::test]
async fn disconnecting_keeps_the_card_history_and_reconnect_requires_a_new_preview() {
    let (state, dana, _) = scratch(false).await;
    let bridge = Bridge::default();
    let before = current_head(&state).await;
    connect(&state, &dana, &bridge, setup_request())
        .await
        .unwrap();
    assert_eq!(current_head(&state).await, before);
    link(
        &state,
        &dana,
        &bridge,
        CARD,
        &before,
        mapping("Cedar Bank", Some(NewAccountKind::RevolvingDebt)),
    )
    .await
    .unwrap();
    let saved = import_october(&state, &dana, &bridge, &current_head(&state).await).await;
    let head = saved["head"].as_str().unwrap();
    let transactions = state
        .books_snapshot()
        .await
        .unwrap()
        .book
        .transactions
        .clone();

    assert_eq!(disconnect(&state, &dana).await.unwrap()["connected"], false);
    assert_eq!(disconnect(&state, &dana).await.unwrap()["connected"], false);
    assert_eq!(current_head(&state).await, head);
    assert_eq!(connection(&state, &dana).await.unwrap()["connected"], false);
    assert!(sync(
        &state,
        &dana,
        &bridge,
        head,
        body(json!({"automatic":true}))
    )
    .await
    .is_err());

    let mut renewed = Bridge::default();
    let listed = connect(&state, &dana, &renewed, setup_request())
        .await
        .unwrap();
    assert_eq!(listed["accounts"][0]["linked"], CARD);
    assert_eq!(
        sync(
            &state,
            &dana,
            &renewed,
            head,
            body(json!({"dry_run":false, "snapshot":saved["snapshot"]}))
        )
        .await
        .unwrap_err()
        .code(),
        "snapshot_expired"
    );
    assert_eq!(
        state.books_snapshot().await.unwrap().book.transactions,
        transactions
    );
    let already_linked = link(
        &state,
        &dana,
        &renewed,
        SECOND,
        head,
        mapping("Cedar Bank", None),
    )
    .await
    .unwrap_err();
    assert_eq!(already_linked.code(), "simplefin_linked");
    assert_eq!(current_head(&state).await, head);

    let unlinked = link(
        &state,
        &dana,
        &renewed,
        CARD,
        head,
        LinkBody {
            remote: None,
            create: None,
        },
    )
    .await
    .unwrap();
    let head = unlinked["head"].as_str().unwrap();
    link(
        &state,
        &dana,
        &renewed,
        SECOND,
        head,
        mapping("Cedar Bank", None),
    )
    .await
    .unwrap();
    let listed = list_accounts(&state, &dana, &renewed, false).await.unwrap();
    assert_eq!(listed["accounts"][0]["linked"], SECOND);
    assert_eq!(
        state.books_snapshot().await.unwrap().book.transactions,
        transactions
    );
    renewed.recent = Some((1792022400 + 86400, false));
    let saved = import_october(&state, &dana, &renewed, &current_head(&state).await).await;
    assert_eq!(saved["counts"]["new"], 1);
    assert_eq!(saved["accounts"][0]["account"], SECOND);
    let books = state.books_snapshot().await.unwrap();
    assert_eq!(books.book.transactions[0], transactions[0]);
    assert!(books.book.transactions[1]
        .postings
        .iter()
        .any(|posting| posting.account.as_str() == SECOND));
}

#[tokio::test]
async fn unlink_preserves_the_feed_identity_of_older_imports() {
    let id = "simplefin:HH%202386";
    let target = kabacus_core::bank_import::encode_id(CARD);
    let journal = format!(
        "account {CARD}  ; simplefin-account:Cedar%20Bank/Everyday%20Account, simplefin-server:bridge.simplefin.org
account {TARGET}:card-Cedar Rewards
account expenses:dining

2026-10-15 * Harbor Street Cafe  ; import-id:{id}, import-target:{target}/{id}
    expenses:dining  18.40 USD
    {CARD}  -18.40 USD
    {TARGET}  -18.40 USD
    {TARGET}:card-Cedar Rewards  18.40 USD
"
    );
    let (state, dana, _) = scratch_with_history(false, &journal).await;
    let bridge = Bridge::default();
    connect(&state, &dana, &bridge, setup_request())
        .await
        .unwrap();
    let head = current_head(&state).await;
    let replay = import_october(&state, &dana, &bridge, &head).await;
    assert_eq!(replay["accounts"][0]["counts"]["duplicate"], 1);
    assert_eq!(replay["head"], head);
    link(
        &state,
        &dana,
        &bridge,
        CARD,
        &head,
        LinkBody {
            remote: None,
            create: None,
        },
    )
    .await
    .unwrap();
    link(
        &state,
        &dana,
        &bridge,
        CARD,
        &current_head(&state).await,
        mapping("Harbor Bank", None),
    )
    .await
    .unwrap();
    let saved = import_october(&state, &dana, &bridge, &current_head(&state).await).await;
    assert_eq!(saved["counts"]["new"], 1);
    link(
        &state,
        &dana,
        &bridge,
        CARD,
        saved["head"].as_str().unwrap(),
        mapping("Cedar Bank", None),
    )
    .await
    .unwrap();
    let replay = import_october(&state, &dana, &bridge, &current_head(&state).await).await;
    assert_eq!(replay["accounts"][0]["counts"]["duplicate"], 1);
}
