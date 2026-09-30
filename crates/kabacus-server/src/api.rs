// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The HTTP API. One `axum::Router` wired over [`crate::state::AppState`].
//! The handlers here only translate HTTP to and from [`crate::service`],
//! which the MCP endpoint ([`crate::mcp`]) calls too.

use crate::auth::oauth::AuthError;
use crate::auth::roles::{Principal, Role};
use crate::ops::OpError;
use crate::service::{self, invalid, RewriteBody};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

/// One route of the API and the MCP tools that do the same (the
/// MCP endpoint has every function the API has).
pub struct Endpoint {
    pub method: &'static str,
    pub path: &'static str,
    pub mcp_tools: &'static [&'static str],
}

/// Ledger API routes, besides MCP and UI assets. Each route added to
/// [`router`] is added here too, with the tools that mirror it.
pub const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        method: "POST",
        path: "/splits",
        mcp_tools: &["record_shared_expense"],
    },
    Endpoint {
        method: "GET",
        path: "/version",
        mcp_tools: &["get_version"],
    },
    Endpoint {
        method: "GET",
        path: "/commodity",
        mcp_tools: &["get_commodity"],
    },
    Endpoint {
        method: "GET",
        path: "/transactions",
        mcp_tools: &["search_transactions"],
    },
    Endpoint {
        method: "POST",
        path: "/transactions",
        mcp_tools: &["add_transaction"],
    },
    Endpoint {
        method: "GET",
        path: "/transactions/{id}",
        mcp_tools: &["get_transaction"],
    },
    Endpoint {
        method: "PATCH",
        path: "/transactions/{id}",
        mcp_tools: &["edit_transaction"],
    },
    Endpoint {
        method: "POST",
        path: "/batch",
        mcp_tools: &["batch"],
    },
    Endpoint {
        method: "PUT",
        path: "/transactions/{id}",
        mcp_tools: &["rewrite_transaction"],
    },
    Endpoint {
        method: "DELETE",
        path: "/transactions/{id}",
        mcp_tools: &["delete_transaction"],
    },
    Endpoint {
        method: "POST",
        path: "/transactions/{id}/corrections",
        mcp_tools: &["correct_transaction", "void_transaction"],
    },
    Endpoint {
        method: "PATCH",
        path: "/transactions/{id}/status",
        mcp_tools: &["set_cleared"],
    },
    Endpoint {
        method: "PUT",
        path: "/assertions",
        mcp_tools: &["set_balance_assertion"],
    },
    Endpoint {
        method: "POST",
        path: "/assertions/inclusive",
        mcp_tools: &["convert_money_assertions"],
    },
    Endpoint {
        method: "GET",
        path: "/history",
        mcp_tools: &["get_history"],
    },
    Endpoint {
        method: "GET",
        path: "/history/{rev}",
        mcp_tools: &["get_history_diff"],
    },
    Endpoint {
        method: "GET",
        path: "/history/{rev}/transactions",
        mcp_tools: &["get_transactions_at"],
    },
    Endpoint {
        method: "GET",
        path: "/cash-on-hand",
        mcp_tools: &["cash_on_hand"],
    },
    Endpoint {
        method: "POST",
        path: "/allocations",
        mcp_tools: &["allocate"],
    },
    Endpoint {
        method: "GET",
        path: "/cards/reserves",
        mcp_tools: &["card_reserves"],
    },
    Endpoint {
        method: "POST",
        path: "/cards/purchases",
        mcp_tools: &["record_card_purchase"],
    },
    Endpoint {
        method: "POST",
        path: "/cards/payments",
        mcp_tools: &["pay_card"],
    },
    Endpoint {
        method: "GET",
        path: "/counterparties",
        mcp_tools: &["list_counterparties"],
    },
    Endpoint {
        method: "GET",
        path: "/counterparties/{name}/statement",
        mcp_tools: &["counterparty_statement", "export_statement"],
    },
    Endpoint {
        method: "GET",
        path: "/accounts",
        mcp_tools: &["list_accounts"],
    },
    Endpoint {
        method: "POST",
        path: "/accounts",
        mcp_tools: &["declare_account"],
    },
    Endpoint {
        method: "POST",
        path: "/accounts/rename",
        mcp_tools: &["rename_account"],
    },
    Endpoint {
        method: "PUT",
        path: "/accounts/{account}/type",
        mcp_tools: &["set_account_type"],
    },
    Endpoint {
        method: "DELETE",
        path: "/accounts/{account}",
        mcp_tools: &["remove_account"],
    },
    Endpoint {
        method: "GET",
        path: "/accounts/kinds",
        mcp_tools: &["account_kinds"],
    },
    Endpoint {
        method: "POST",
        path: "/reconcile",
        mcp_tools: &["reconcile"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/income-statement",
        mcp_tools: &["income_statement"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/balance-sheet",
        mcp_tools: &["balance_sheet"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/categories",
        mcp_tools: &["category_breakdown"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/cash-on-hand-history",
        mcp_tools: &["cash_on_hand_history"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/cash-flow",
        mcp_tools: &["cash_flow"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/receivables",
        mcp_tools: &["receivables_activity"],
    },
    Endpoint {
        method: "GET",
        path: "/payees",
        mcp_tools: &["list_payees"],
    },
    Endpoint {
        method: "GET",
        path: "/register/{account}",
        mcp_tools: &["get_register"],
    },
    Endpoint {
        method: "POST",
        path: "/import",
        mcp_tools: &["import_journal"],
    },
    // `.kabacus` files are binary: API only, no MCP tools.
    Endpoint {
        method: "GET",
        path: "/ledgers/{id}/export",
        mcp_tools: &[],
    },
    Endpoint {
        method: "POST",
        path: "/ledgers/import",
        mcp_tools: &[],
    },
];

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/splits", post(record_shared_expense))
        .route("/version", get(get_version))
        .route("/commodity", get(get_commodity))
        .route(
            "/transactions",
            get(list_transactions).post(add_transaction),
        )
        .route(
            "/transactions/{id}",
            get(get_transaction)
                .put(put_rewrite)
                .delete(delete_rewrite)
                .patch(patch_edit),
        )
        .route(
            "/batch",
            post(run_batch).layer(axum::extract::DefaultBodyLimit::max(
                crate::batch::MAX_BYTES + 1024 * 1024,
            )),
        )
        .route("/transactions/{id}/corrections", post(add_correction))
        .route("/transactions/{id}/status", patch(patch_status))
        .route("/assertions", axum::routing::put(put_assertion))
        .route("/assertions/inclusive", post(convert_money_assertions))
        .route("/history", get(get_history))
        .route("/history/{rev}", get(get_history_rev))
        .route("/history/{rev}/transactions", get(get_history_transactions))
        .route("/cash-on-hand", get(get_cash_on_hand))
        .route("/allocations", post(allocate))
        .route("/cards/reserves", get(get_card_reserves))
        .route("/cards/purchases", post(record_card_purchase))
        .route("/cards/payments", post(pay_card))
        .route("/counterparties", get(list_counterparties))
        .route("/counterparties/{name}/statement", get(get_statement))
        .route("/accounts", get(list_accounts).post(declare_account))
        .route("/accounts/kinds", get(get_account_kinds))
        .route("/accounts/rename", post(rename_account))
        .route(
            "/accounts/{account}/type",
            axum::routing::put(set_account_type),
        )
        .route("/accounts/{account}", axum::routing::delete(remove_account))
        .route("/reconcile", post(reconcile))
        .route("/reports/income-statement", get(income_statement))
        .route("/reports/balance-sheet", get(balance_sheet))
        .route("/reports/categories", get(category_breakdown))
        .route("/reports/cash-on-hand-history", get(cash_on_hand_history))
        .route("/reports/cash-flow", get(cash_flow))
        .route("/reports/receivables", get(receivables_activity))
        .route("/payees", get(list_payees))
        .route("/register/{account}", get(get_register))
        .route(
            "/import",
            post(import_journal).layer(axum::extract::DefaultBodyLimit::max(
                state.config.import_max_bytes,
            )),
        )
        .route("/ledgers/{id}/export", get(export_ledger))
        .route(
            "/ledgers/import",
            post(import_ledgers).layer(axum::extract::DefaultBodyLimit::max(
                state.config.import_max_bytes + 1024,
            )),
        )
        .merge(crate::web::routes())
        .merge(crate::mcp::routes(state.clone()))
        .layer(axum::middleware::from_fn(log_request))
        .with_state(state)
}

/// A request slower than this is logged as a warning: the app gives up
/// on reads after 30 s, so a slow one is worth seeing before it fails.
pub(crate) const SLOW_REQUEST: std::time::Duration = std::time::Duration::from_secs(5);

/// One log line per request: method, path, status and time taken. The
/// query is left out; it can hold account names and amounts.
async fn log_request(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    let elapsed = started.elapsed();
    let ms = elapsed.as_millis();
    if status >= 500 {
        tracing::error!("{method} {path} -> {status} in {ms} ms");
    } else if elapsed >= SLOW_REQUEST {
        tracing::warn!("{method} {path} -> {status} in {ms} ms (slow)");
    } else {
        tracing::info!("{method} {path} -> {status} in {ms} ms");
    }
    response
}

// --- auth and headers -------------------------------------------------------

pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Principal, ApiError> {
    let authz = headers.get("authorization").and_then(|v| v.to_str().ok());
    let client_hint = headers
        .get("kabacus-client-hint")
        .and_then(|v| v.to_str().ok());
    state
        .auth
        .authenticate(authz, client_hint)
        .await
        .map_err(ApiError::Auth)
}

/// The head an If-Match names. Entity tags are quoted in HTTP (RFC 9110
/// §8.8.3), so a client echoing the ETag sends `"<head>"`; the app sends
/// the bare head. A weak tag (`W/"..."`) is left as it is: If-Match
/// compares strongly, so it never matches.
fn if_match_header(headers: &HeaderMap) -> Result<String, ApiError> {
    let value = headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| invalid("if-match", "If-Match header is required".to_string()))?
        .trim();
    Ok(value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
        .to_string())
}

/// The ETag for a head: quoted, as HTTP entity tags are.
fn etag(head: &str) -> String {
    format!("\"{head}\"")
}

// --- errors ------------------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    Auth(AuthError),
    MissingRole(Role),
    NotFound(String),
    Conflict {
        expected: String,
        current: String,
    },
    Invalid {
        rule: String,
        message: String,
        /// The strict check's failures, when it refused.
        failures: Vec<kabacus_ledger::CheckFailure>,
    },
    Internal(anyhow::Error),
    /// The first rewrite gate: the server bans rewrites (`--no-rewrite`).
    RewriteDisabled,
    /// An amount that isn't a decimal string or an exact object.
    AmountFormat {
        path: String,
        message: String,
    },
    /// The account is still referred to.
    AccountInUse {
        account: String,
        counts: std::collections::BTreeMap<String, usize>,
    },
    /// Over the batch limits.
    BatchTooLarge(String),
    /// `assert` refused because the balances differ.
    ReconcileMismatch(serde_json::Value),
    /// A batch refused in atomic mode, naming the operation.
    BatchFailed {
        index: Option<usize>,
        client_ref: Option<String>,
        error: Box<ApiError>,
    },
}

/// The machine-readable error shape, `{code, message, path?, details?}`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    /// The stable error code clients match on.
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::Auth(_) => "unauthorized",
            ApiError::MissingRole(_) => "role_required",
            ApiError::NotFound(_) => "not_found",
            ApiError::Conflict { .. } => "head_conflict",
            ApiError::Invalid { .. } => "validation_failed",
            ApiError::Internal(_) => "internal",
            ApiError::RewriteDisabled => "rewrite_disabled",
            ApiError::AmountFormat { .. } => "amount_format",
            ApiError::AccountInUse { .. } => "account_in_use",
            ApiError::BatchTooLarge(_) => "batch_too_large",
            ApiError::ReconcileMismatch(_) => "reconcile_mismatch",
            ApiError::BatchFailed { error, .. } => error.code(),
        }
    }

    pub fn body(&self) -> ErrorBody {
        let (message, path, details) = match self {
            ApiError::Auth(e) => (e.to_string(), None, None),
            ApiError::MissingRole(role) => (
                format!("your sign-in doesn't have the {} role", role.name()),
                None,
                Some(serde_json::json!({ "role": role.name() })),
            ),
            ApiError::NotFound(id) => (
                format!("{id} was not found"),
                None,
                Some(serde_json::json!({ "id": id })),
            ),
            ApiError::Conflict { expected, current } => (
                format!("the books changed since head {expected} (now {current}); re-read and retry with the new head"),
                None,
                Some(serde_json::json!({ "expected": expected, "current_head": current })),
            ),
            ApiError::Invalid { rule, message, failures } => {
                let mut details = serde_json::json!({ "rule": rule });
                if !failures.is_empty() {
                    details["failures"] = failures_json(failures);
                }
                if let Some(a) = failures
                    .iter()
                    .find(|f| f.kind == kabacus_ledger::FailureKind::Assertion)
                {
                    if let kabacus_ledger::Position::Transaction { id, posting } = &a.position {
                        details["transaction"] = serde_json::json!(id);
                        details["posting"] = serde_json::json!(posting);
                    }
                    details["fix_tool"] = serde_json::json!("set_balance_assertion");
                }
                (format!("{message} (rule: {rule})"), None, Some(details))
            }
            ApiError::Internal(e) => (e.to_string(), None, None),
            ApiError::RewriteDisabled => (
                "rewrites are disabled: the server was started with --no-rewrite or allow_rewrite = false".to_string(),
                None,
                None,
            ),
            ApiError::AmountFormat { path, message } => (message.clone(), Some(path.clone()), None),
            ApiError::AccountInUse { account, counts } => (
                format!(
                    "{account} is still in use ({}); move or remove those first",
                    counts
                        .iter()
                        .filter(|(_, n)| **n > 0)
                        .map(|(k, n)| format!("{n} {k}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None,
                Some(serde_json::json!({ "account": account, "counts": counts })),
            ),
            ApiError::BatchTooLarge(message) => (message.clone(), None, None),
            ApiError::ReconcileMismatch(result) => (
                "the ledger balance doesn't match the statement; nothing was written".to_string(),
                None,
                Some(result.clone()),
            ),
            ApiError::BatchFailed { index, client_ref, error } => {
                let inner = error.body();
                let mut details = inner.details.unwrap_or_else(|| serde_json::json!({}));
                details["index"] = serde_json::json!(index);
                details["client_ref"] = serde_json::json!(client_ref);
                let at = match (index, client_ref) {
                    (Some(i), Some(r)) => format!("operation {i} ({r}): "),
                    (Some(i), None) => format!("operation {i}: "),
                    _ => String::new(),
                };
                (
                    format!("{at}{}; nothing was written", inner.message),
                    Some(match (index, inner.path) {
                        (Some(i), Some(p)) => format!("ops[{i}].args.{p}"),
                        (Some(i), None) => format!("ops[{i}]"),
                        (None, p) => p.unwrap_or_default(),
                    }),
                    Some(details),
                )
            }
        };
        ErrorBody {
            code: self.code().to_string(),
            message,
            path,
            details,
        }
    }
}

/// The strict check's failures, as callers read them.
pub fn failures_json(failures: &[kabacus_ledger::CheckFailure]) -> serde_json::Value {
    serde_json::Value::Array(
        failures
            .iter()
            .map(|f| {
                let mut v = serde_json::json!({
                    "kind": f.kind.as_str(),
                    "message": f.message(),
                    "sentence": f.sentence,
                    "details": f.details,
                });
                match &f.position {
                    kabacus_ledger::Position::Transaction { id, posting } => {
                        v["transaction"] = serde_json::json!(id);
                        v["posting"] = serde_json::json!(posting);
                    }
                    kabacus_ledger::Position::Line { file, line } => {
                        v["file"] = serde_json::json!(file);
                        v["line"] = serde_json::json!(line);
                    }
                }
                v
            })
            .collect(),
    )
}

/// The rule a failure of `kind` breaks.
fn failure_rule(kind: kabacus_ledger::FailureKind) -> &'static str {
    match kind {
        kabacus_ledger::FailureKind::Readable => "readable",
        kabacus_ledger::FailureKind::Balanced => "balanced",
        kabacus_ledger::FailureKind::Assertion => "assertion",
        kabacus_ledger::FailureKind::Commodity => "commodity",
        kabacus_ledger::FailureKind::Account => "account",
    }
}

impl From<OpError> for ApiError {
    fn from(e: OpError) -> Self {
        match e {
            OpError::Validation(v) => invalid("validation", v.to_string()),
            OpError::ValidationMany(errs) => {
                let message = errs
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                invalid("validation", message)
            }
            OpError::NotFound(id) => ApiError::NotFound(id),
            OpError::Conflict { expected, current } => ApiError::Conflict { expected, current },
            OpError::Invalid { rule, message } => invalid(&rule, message),
            OpError::Check(failures) => ApiError::Invalid {
                rule: failures
                    .first()
                    .map_or("check", |f| failure_rule(f.kind))
                    .to_string(),
                message: failures
                    .iter()
                    .map(|f| f.message())
                    .collect::<Vec<_>>()
                    .join("; "),
                failures,
            },
            OpError::AccountInUse { account, counts } => ApiError::AccountInUse { account, counts },
            other => ApiError::Internal(anyhow::anyhow!(other.to_string())),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = self.code();
        let body = self.body();
        let mut json = match &self {
            ApiError::Auth(AuthError::MissingToken)
            | ApiError::Auth(AuthError::Inactive)
            | ApiError::Auth(AuthError::InvalidToken(_))
            | ApiError::Auth(AuthError::UnknownKid(_)) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    [("WWW-Authenticate", "Bearer")],
                    Json(serde_json::json!({"error": "unauthorized", "code": code})),
                )
                    .into_response()
            }
            ApiError::Auth(other) => serde_json::json!({"error": other.to_string()}),
            ApiError::MissingRole(role) => {
                serde_json::json!({"error": "missing role", "role": role.name()})
            }
            ApiError::NotFound(id) => serde_json::json!({"error": "not found", "id": id}),
            ApiError::Conflict { expected, current } => {
                serde_json::json!({"error": "conflict", "expected": expected, "current": current})
            }
            ApiError::Invalid {
                rule,
                message,
                failures,
            } => {
                serde_json::json!({"rule": rule, "message": message, "failures": failures_json(failures)})
            }
            ApiError::Internal(e) => serde_json::json!({"error": e.to_string()}),
            _ => {
                let mut json = serde_json::to_value(&body).unwrap_or_default();
                json["error"] = serde_json::json!(body.message);
                json
            }
        };
        json["code"] = serde_json::json!(code);
        if json.get("message").is_none() {
            json["message"] = serde_json::json!(body.message);
        }
        if let (Some(details), None) = (&body.details, json.get("details")) {
            json["details"] = details.clone();
        }
        let status = match &self {
            ApiError::Auth(_) => StatusCode::UNAUTHORIZED,
            ApiError::MissingRole(_) | ApiError::RewriteDisabled => StatusCode::FORBIDDEN,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict { .. } => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::BatchTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::AccountInUse { .. } | ApiError::ReconcileMismatch(_) => StatusCode::CONFLICT,
            ApiError::BatchFailed { error, .. } => match error.code() {
                "head_conflict" => StatusCode::CONFLICT,
                "not_found" => StatusCode::NOT_FOUND,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            },
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        (status, Json(json)).into_response()
    }
}

fn created_unless_dry_run(dry_run: bool) -> StatusCode {
    if dry_run {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    }
}

// --- handlers -----------------------------------------------------------------

/// Unauthenticated.
async fn get_version(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(service::version(&state).await)
}

/// The app's plain list, unless the caller asks for a page.
async fn list_transactions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<crate::search::SearchQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let head = etag(
        &state
            .books_snapshot()
            .await
            .map(|b| b.key.clone())
            .unwrap_or_default(),
    );
    let paged =
        q.limit.is_some() || q.cursor.is_some() || q.count_only.is_some() || q.fields.is_some();
    if paged {
        let page = crate::search::search(&state, &principal, &q).await?;
        return Ok((StatusCode::OK, [("ETag", head)], Json(page)).into_response());
    }
    let dtos = service::list_transactions(&state, &principal, &q).await?;
    Ok((StatusCode::OK, [("ETag", head)], Json(dtos)).into_response())
}

#[derive(Deserialize)]
struct GetTransactionQuery {
    #[serde(default)]
    include_history: bool,
}

async fn get_transaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<GetTransactionQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let tx = crate::search::get_transaction(&state, &principal, &id, q.include_history).await?;
    Ok(Json(tx).into_response())
}

/// A batch. Its head is in the body, like MCP's.
async fn run_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<crate::batch::BatchBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(crate::batch::batch(&state, &principal, body).await?).into_response())
}

async fn reconcile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<crate::reports::ReconcileParams>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    if body.head.is_none() {
        body.head = headers
            .get("if-match")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
    }
    Ok(Json(crate::reports::reconcile(&state, &principal, &body).await?).into_response())
}

macro_rules! report_route {
    ($name:ident, $params:ty) => {
        async fn $name(
            State(state): State<Arc<AppState>>,
            headers: HeaderMap,
            Query(q): Query<$params>,
        ) -> Result<Response, ApiError> {
            let principal = authenticate(&state, &headers).await?;
            Ok(Json(crate::reports::$name(&state, &principal, &q).await?).into_response())
        }
    };
}

report_route!(income_statement, crate::reports::IncomeStatementParams);
report_route!(balance_sheet, crate::reports::BalanceSheetParams);
report_route!(category_breakdown, crate::reports::CategoryBreakdownParams);
report_route!(cash_on_hand_history, crate::reports::CashHistoryParams);
report_route!(cash_flow, crate::reports::CashFlowParams);
report_route!(receivables_activity, crate::reports::ReceivablesParams);

async fn add_transaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::AddTransactionBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = service::add_transaction(&state, &principal, body, &if_match).await?;
    Ok((status, Json(result)).into_response())
}

async fn add_correction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<service::CorrectionBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = service::add_correction(&state, &principal, &id, body, &if_match).await?;
    Ok((status, Json(result)).into_response())
}

async fn patch_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<service::StatusBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let result = service::set_status(&state, &principal, &id, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn patch_edit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RewriteBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let result = service::edit_transaction(&state, &principal, &id, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn get_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::history(&state, &principal).await?).into_response())
}

async fn get_history_rev(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(rev): Path<String>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let diff = service::history_diff(&state, &principal, &rev).await?;
    Ok((StatusCode::OK, [("content-type", "text/plain")], diff).into_response())
}

async fn get_history_transactions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(rev): Path<String>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let txs = service::history_transactions(&state, &principal, &rev).await?;
    Ok(Json(txs).into_response())
}

async fn get_cash_on_hand(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<service::CashOnHandQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::cash_on_hand(&state, &principal, &q).await?).into_response())
}

async fn get_commodity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::books_commodity_info(&state, &principal).await?).into_response())
}

async fn get_account_kinds(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::account_kinds(&state, &principal).await?).into_response())
}

#[derive(Deserialize)]
struct CardReservesQuery {
    #[serde(default)]
    card: Option<String>,
}

async fn get_card_reserves(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<CardReservesQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::card_reserves(&state, &principal, q.card.as_deref()).await?).into_response())
}

async fn allocate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::AllocateBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = service::allocate(&state, &principal, body, &if_match).await?;
    Ok((status, Json(result)).into_response())
}

async fn record_card_purchase(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::CardPurchaseBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = service::record_card_purchase(&state, &principal, body, &if_match).await?;
    Ok((status, Json(result)).into_response())
}

async fn pay_card(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::CardPaymentBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let if_match = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = service::pay_card(&state, &principal, body, &if_match).await?;
    Ok((status, Json(result)).into_response())
}

async fn list_counterparties(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::counterparties(&state, &principal).await?).into_response())
}

async fn get_statement(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<service::StatementQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let stmt = service::statement(&state, &principal, &name, &q).await?;
    if matches!(q.format.as_deref(), None | Some("json")) {
        return Ok(Json(service::statement_dto(stmt)).into_response());
    }
    let file = service::statement_file(&stmt, &q)?;
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                file.content_type.to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", file.filename),
            ),
        ],
        file.body,
    )
        .into_response())
}

async fn list_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<service::AccountsQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::accounts_filtered(&state, &principal, &q).await?).into_response())
}

#[derive(Deserialize)]
struct PayeesQuery {
    #[serde(default)]
    q: Option<String>,
}

async fn list_payees(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<PayeesQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(service::payees(&state, &principal, q.q.as_deref()).await?).into_response())
}

#[derive(Deserialize)]
struct RegisterQuery {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    /// A page of lines instead of all of them.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

async fn get_register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
    Query(q): Query<RegisterQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    if q.limit.is_some() || q.cursor.is_some() {
        let page = crate::search::register_page(
            &state,
            &principal,
            &account,
            q.from.as_deref(),
            q.to.as_deref(),
            q.cursor.as_deref(),
            q.limit,
        )
        .await?;
        return Ok(Json(page).into_response());
    }
    let lines = service::register(
        &state,
        &principal,
        &account,
        q.from.as_deref(),
        q.to.as_deref(),
    )
    .await?;
    Ok(Json(lines).into_response())
}

// --- rewrites (hidden): behind the rewrite gates ------------------------

/// The rewrite gates, then If-Match.
async fn rewrite_gate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(Principal, String), ApiError> {
    let principal = authenticate(state, headers).await?;
    service::check_rewrite_gates(state, &principal)?;
    let if_match = if_match_header(headers)?;
    Ok((principal, if_match))
}

async fn put_rewrite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RewriteBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result = service::rewrite_transaction(&state, &principal, &id, body, &if_match).await?;
    Ok(Json(result).into_response())
}

#[derive(Deserialize, Default)]
struct DryRunBody {
    #[serde(default)]
    dry_run: bool,
}

async fn delete_rewrite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<DryRunBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result =
        service::delete_transaction(&state, &principal, &id, &if_match, body.dry_run).await?;
    Ok(Json(result).into_response())
}

async fn declare_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::DeclareAccountBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result = service::declare_account(&state, &principal, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn set_account_type(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
    Json(body): Json<service::SetAccountTypeBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result = service::set_account_type(&state, &principal, account, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn put_assertion(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::BalanceAssertionBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result = service::set_balance_assertion(&state, &principal, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn convert_money_assertions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DryRunBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result =
        service::convert_money_assertions(&state, &principal, &if_match, body.dry_run).await?;
    Ok(Json(result).into_response())
}

async fn rename_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<service::RenameAccountBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result = service::rename_account(&state, &principal, body, &if_match).await?;
    Ok(Json(result).into_response())
}

async fn remove_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
    Json(body): Json<DryRunBody>,
) -> Result<Response, ApiError> {
    let (principal, if_match) = rewrite_gate(&state, &headers).await?;
    let result =
        service::remove_account(&state, &principal, &account, &if_match, body.dry_run).await?;
    Ok(Json(result).into_response())
}

// --- import and export -------------------------------------------------------

async fn import_journal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<crate::import::JournalImportBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let status = created_unless_dry_run(body.dry_run);
    let result = crate::import::import_journal(&state, &principal, body).await?;
    Ok((status, Json(result)).into_response())
}

async fn export_ledger(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let (filename, bytes) = crate::import::export_ledger(&state, &principal, id).await?;
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/x-kabacus".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn import_ledgers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let result = crate::import::import_file(&state, &principal, &body).await?;
    Ok((StatusCode::CREATED, Json(result)).into_response())
}

async fn record_shared_expense(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<crate::split::SplitBody>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let head = if_match_header(&headers)?;
    let status = created_unless_dry_run(body.dry_run);
    let result = crate::split::record(&state, &principal, body, &head).await?;
    Ok((status, Json(result)).into_response())
}
