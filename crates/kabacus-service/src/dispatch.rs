// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! In-process request adapter used by local books, native windows and WebAssembly.

use crate::{caller::Caller, error::Error, reports, search, service as s, Service};
use kabacus_store::BookStore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
    pub filename: Option<String>,
}

impl Response {
    pub fn json(value: impl Serialize) -> Result<Self, Error> {
        Ok(Self {
            status: 200,
            body: serde_json::to_vec(&value).map_err(|e| Error::Internal(e.into()))?,
            content_type: "application/json".into(),
            filename: None,
        })
    }

    pub fn error(error: Error) -> Self {
        let status = match error.code() {
            "role_required" | "rewrite_disabled" => 403,
            "not_found" => 404,
            "head_conflict" | "account_in_use" | "reconcile_mismatch" => 409,
            "batch_too_large" => 413,
            "internal" => 500,
            _ => 422,
        };
        Self {
            status,
            body: serde_json::to_vec(&error.body()).unwrap_or_default(),
            content_type: "application/json".into(),
            filename: None,
        }
    }
}

fn parse<T: DeserializeOwned>(body: &Value) -> Result<T, Error> {
    serde_json::from_value(body.clone()).map_err(|e| s::invalid("request", e.to_string()))
}
fn query<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_urlencoded::from_str(text).map_err(|e| s::invalid("query", e.to_string()))
}

pub async fn version(state: &Service) -> Result<Value, Error> {
    let head = state
        .store
        .revision(state.ledger)
        .await
        .map_err(|e| Error::Internal(e.into()))?
        .to_string();
    Ok(
        json!({"server_version": kabacus_ledger::VERSION, "engine_version": kabacus_ledger::VERSION,
        "head": head, "books": head, "schema_version": kabacus_store::sqlite::schema::latest_version(), "ledger":state.ledger, "store": "sqlite", "commit": "",
        "source_url": env!("KABACUS_LIBRARY_SOURCE"), "tool_list_hash": "",
        "auth": {"mode": "none", "issuer": null, "app_client_id": null}}),
    )
}

pub async fn dispatch(
    state: &Service,
    caller: &dyn Caller,
    request: Request,
) -> Result<Response, Error> {
    let (path, qs) = request.path.split_once('?').unwrap_or((&request.path, ""));
    let segments: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .map(|s| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map(|s| s.into_owned())
                .map_err(|e| s::invalid("path", e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
    let body = &request.body;
    let head = request.head.as_deref().unwrap_or("").trim_matches('"');
    let dry = body["dry_run"].as_bool().unwrap_or(false);
    macro_rules! result {
        ($value:expr) => {
            Response::json($value.await?)
        };
    }
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["version"]) => Response::json(version(state).await?),
        ("GET", ["commands"]) => {
            s::require(caller, crate::caller::Permission::Read)?;
            Response::json(crate::catalog::commands())
        }
        ("GET", ["commodity"]) => result!(s::books_commodity_info(state, caller)),
        ("GET", ["transactions"]) => {
            let q: search::SearchQuery = query(qs)?;
            if q.limit.is_some()
                || q.cursor.is_some()
                || q.count_only.is_some()
                || q.fields.is_some()
            {
                result!(search::search(state, caller, &q))
            } else {
                result!(s::list_transactions(state, caller, &q))
            }
        }
        ("GET", ["transactions", id]) => {
            #[derive(Deserialize)]
            struct Q {
                #[serde(default)]
                include_history: bool,
            }
            let q: Q = query(qs)?;
            result!(search::get_transaction(
                state,
                caller,
                id,
                q.include_history
            ))
        }
        ("POST", ["transactions"]) => {
            result!(s::add_transaction(state, caller, parse(body)?, head))
        }
        ("PATCH", ["transactions", id]) => {
            result!(s::edit_transaction(state, caller, id, parse(body)?, head))
        }
        ("PUT", ["transactions", id]) => result!(s::rewrite_transaction(
            state,
            caller,
            id,
            parse(body)?,
            head
        )),
        ("DELETE", ["transactions", id]) => {
            result!(s::delete_transaction(state, caller, id, head, dry))
        }
        ("POST", ["transactions", id, "corrections"]) => {
            result!(s::add_correction(state, caller, id, parse(body)?, head))
        }
        ("PATCH", ["transactions", id, "status"]) => {
            result!(s::set_status(state, caller, id, parse(body)?, head))
        }
        ("POST", ["allocations"]) => result!(s::allocate(state, caller, parse(body)?, head)),
        ("POST", ["cards", "purchases"]) => {
            result!(s::record_card_purchase(state, caller, parse(body)?, head))
        }
        ("POST", ["cards", "payments"]) => result!(s::pay_card(state, caller, parse(body)?, head)),
        ("POST", ["splits"]) => result!(crate::split::record(state, caller, parse(body)?, head)),
        ("POST", ["batch"]) => result!(crate::batch::batch(state, caller, parse(body)?)),
        ("POST", ["reconcile"]) => {
            let mut q: reports::ReconcileParams = parse(body)?;
            if q.head.is_none() && !head.is_empty() {
                q.head = Some(head.into());
            }
            result!(reports::reconcile(state, caller, &q))
        }
        ("GET", ["history"]) => result!(s::history(state, caller)),
        ("GET", ["history", rev]) => Ok(Response {
            status: 200,
            body: s::history_diff(state, caller, rev).await?.into_bytes(),
            content_type: "text/plain".into(),
            filename: None,
        }),
        ("GET", ["history", rev, "transactions"]) => {
            result!(s::history_transactions(state, caller, rev))
        }
        ("GET", ["cash-on-hand"]) => result!(s::cash_on_hand(state, caller, &query(qs)?)),
        ("GET", ["cards", "reserves"]) => {
            #[derive(Deserialize)]
            struct Q {
                card: Option<String>,
            }
            let q: Q = query(qs)?;
            result!(s::card_reserves(state, caller, q.card.as_deref()))
        }
        ("GET", ["counterparties"]) => result!(s::counterparties(state, caller)),
        ("GET", ["counterparties", name, "statement"]) => {
            let q: s::StatementQuery = query(qs)?;
            let statement = s::statement(state, caller, name, &q).await?;
            if matches!(q.format.as_deref(), None | Some("json")) {
                Response::json(s::statement_dto(statement))
            } else {
                let f = s::statement_file(&statement, &q)?;
                Ok(Response {
                    status: 200,
                    body: f.body,
                    content_type: f.content_type.into(),
                    filename: Some(f.filename),
                })
            }
        }
        ("GET", ["accounts"]) => result!(s::accounts_filtered(state, caller, &query(qs)?)),
        ("GET", ["accounts", "kinds"]) => result!(s::account_kinds(state, caller)),
        ("POST", ["accounts"]) => result!(s::declare_account(state, caller, parse(body)?, head)),
        ("POST", ["accounts", "rename"]) => {
            result!(s::rename_account(state, caller, parse(body)?, head))
        }
        ("DELETE", ["accounts", account]) => {
            result!(s::remove_account(state, caller, account, head, dry))
        }
        ("PUT", ["accounts", account, "type"]) => result!(s::set_account_type(
            state,
            caller,
            (*account).to_string(),
            parse(body)?,
            head
        )),
        ("PUT", ["assertions"]) => {
            result!(s::set_balance_assertion(state, caller, parse(body)?, head))
        }
        ("POST", ["assertions", "inclusive"]) => {
            result!(s::convert_money_assertions(state, caller, head, dry))
        }
        ("GET", ["payees"]) => {
            #[derive(Deserialize)]
            struct Q {
                q: Option<String>,
            }
            let q: Q = query(qs)?;
            result!(s::payees(state, caller, q.q.as_deref()))
        }
        ("GET", ["register", account]) => {
            #[derive(Deserialize)]
            struct Q {
                from: Option<String>,
                to: Option<String>,
                limit: Option<usize>,
                cursor: Option<String>,
            }
            let q: Q = query(qs)?;
            if q.limit.is_some() || q.cursor.is_some() {
                result!(search::register_page(
                    state,
                    caller,
                    account,
                    q.from.as_deref(),
                    q.to.as_deref(),
                    q.cursor.as_deref(),
                    q.limit
                ))
            } else {
                result!(s::register(
                    state,
                    caller,
                    account,
                    q.from.as_deref(),
                    q.to.as_deref()
                ))
            }
        }
        ("GET", ["reports", "income-statement"]) => {
            result!(reports::income_statement(state, caller, &query(qs)?))
        }
        ("GET", ["reports", "balance-sheet"]) => {
            result!(reports::balance_sheet(state, caller, &query(qs)?))
        }
        ("GET", ["reports", "categories"]) => {
            result!(reports::category_breakdown(state, caller, &query(qs)?))
        }
        ("GET", ["reports", "cash-on-hand-history"]) => {
            result!(reports::cash_on_hand_history(state, caller, &query(qs)?))
        }
        ("GET", ["reports", "cash-flow"]) => {
            result!(reports::cash_flow(state, caller, &query(qs)?))
        }
        ("GET", ["reports", "receivables"]) => {
            result!(reports::receivables_activity(state, caller, &query(qs)?))
        }
        ("POST", ["import"]) => result!(crate::import::import_journal(state, caller, parse(body)?)),
        ("GET", ["ledgers", id, "export"]) => {
            let id = id
                .parse()
                .map_err(|_| s::invalid("ledger", "Invalid ledger id".into()))?;
            let (name, bytes) = crate::import::export_ledger(state, caller, id).await?;
            Ok(Response {
                status: 200,
                body: bytes,
                content_type: "application/x-kabacus".into(),
                filename: Some(name),
            })
        }
        ("POST", ["ledgers", "import"]) => {
            let bytes: Vec<u8> = parse(body)?;
            result!(crate::import::import_file(state, caller, &bytes))
        }
        _ => Err(Error::NotFound(format!("{} {path}", request.method))),
    }
}
