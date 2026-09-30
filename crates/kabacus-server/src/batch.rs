// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `batch`: many write operations applied in order to one working copy
//! of the books, checked once, and committed as one write (or one per
//! operation). Operation *n* sees what the operations before it did, and
//! can refer to an earlier result with `{"$ref": "<client_ref>.id"}`.

use crate::api::ApiError;
use crate::auth::roles::{Principal, Role};
use crate::books::WorkingCopy;
use crate::ops::{self, OpError};
use crate::service::{self, balance_change_dtos, check_rewrite_gates, invalid, require};
use crate::state::AppState;
use kabacus_store::{BookStore, ChangeSet};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// A batch's limits: operations and request size.
pub const MAX_OPS: usize = 100;
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
pub const TIME_LIMIT: Duration = Duration::from_secs(120);

/// The ordinary tools a batch may run.
pub const ORDINARY_TOOLS: &[&str] = &[
    "add_transaction",
    "edit_transaction",
    "record_card_purchase",
    "pay_card",
    "allocate",
    "set_cleared",
    "void_transaction",
    "correct_transaction",
];

/// The rewrite-class tools a batch may run: any of them puts the whole
/// batch behind the rewrite gates.
pub const REWRITE_TOOLS: &[&str] = &[
    "declare_account",
    "rename_account",
    "remove_account",
    "set_balance_assertion",
    "rewrite_transaction",
    "delete_transaction",
    "convert_money_assertions",
];

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct BatchOp {
    /// One of the write tools, e.g. `"edit_transaction"`.
    pub tool: String,
    /// That tool's arguments, without `head` and `dry_run`. A value
    /// `{"$ref": "<client_ref>.id"}` is replaced by that earlier
    /// operation's id.
    #[serde(default)]
    pub args: Value,
    /// A name for this operation, unique in the batch, to refer to its
    /// result and to find it in the result.
    #[serde(default)]
    pub client_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BatchBody {
    /// The books' current head; checked once for the whole batch.
    pub head: String,
    /// Defaults to true: check everything, write nothing.
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// `"single"` (default): one write for the batch; `"per_op"`: one
    /// write (revision) per operation.
    #[serde(default)]
    pub commit: Option<String>,
    /// `"atomic"` (default): any failure writes nothing; or
    /// `"continue_on_error"` (dry runs only) to list every failure.
    #[serde(default)]
    pub mode: Option<String>,
    /// Recorded with the write. Committing the same id again returns
    /// the first result and writes nothing.
    #[serde(default)]
    pub batch_id: Option<String>,
    /// At most 100 operations.
    pub ops: Vec<BatchOp>,
}

/// One operation's outcome in a batch's result.
#[derive(Debug, Clone, Serialize)]
pub struct OpResult {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ref: Option<String>,
    /// `ok` or `error`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub warnings: Vec<ops::OpWarning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::api::ErrorBody>,
}

/// Replace every `{"$ref": "<client_ref>.id"}` in `value` with that
/// operation's id.
fn resolve_refs(value: &mut Value, ids: &HashMap<String, u64>) -> Result<(), ApiError> {
    match value {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(Value::String(reference)) = map.get("$ref") {
                    let (name, field) = reference.rsplit_once('.').unwrap_or((reference, "id"));
                    if field != "id" {
                        return Err(invalid(
                            "$ref",
                            format!("{reference:?}: only <client_ref>.id can be referred to"),
                        ));
                    }
                    let id = ids.get(name).ok_or_else(|| {
                        invalid(
                            "$ref",
                            format!("{reference:?} names no earlier operation with an id"),
                        )
                    })?;
                    *value = json!(id);
                    return Ok(());
                }
            }
            for v in map.values_mut() {
                resolve_refs(v, ids)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter_mut().try_for_each(|v| resolve_refs(v, ids)),
        _ => Ok(()),
    }
}

/// A stable 64-bit FNV-1a hash.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// How far back a replayed batch is looked for.
const REPLAY_WINDOW: usize = 1000;

/// A committed batch's result, if `batch_id` was committed before: the
/// audit log records it with the write.
async fn recorded(state: &AppState, batch_id: &str) -> Option<Value> {
    let history = state
        .store
        .history(state.ledger, REPLAY_WINDOW)
        .await
        .ok()?;
    let first = history
        .iter()
        .rfind(|c| c.details.get("batch_id").and_then(Value::as_str) == Some(batch_id))?;
    if let Some(result) = first.details.get("result") {
        return Some(result.clone());
    }
    let head = state.store.revision(state.ledger).await.ok()?;
    Some(json!({
        "head": head.to_string(),
        "dry_run": false,
        "batch_id": batch_id,
        "commit": first.revision.to_string(),
    }))
}

fn op_error(index: usize, client_ref: &Option<String>, error: ApiError) -> ApiError {
    ApiError::BatchFailed {
        index: Some(index),
        client_ref: client_ref.clone(),
        error: Box::new(error),
    }
}

/// Which operation wrote the transaction a failure is in, if any.
fn blame(
    failures: &[kabacus_ledger::CheckFailure],
    written_by: &HashMap<u64, usize>,
) -> Option<usize> {
    failures.iter().find_map(|f| match &f.position {
        kabacus_ledger::Position::Transaction { id, .. } => written_by.get(id).copied(),
        _ => None,
    })
}

/// Run a batch.
pub async fn batch(
    state: &AppState,
    principal: &Principal,
    body: BatchBody,
) -> Result<Value, ApiError> {
    match tokio::time::timeout(TIME_LIMIT, run(state, principal, body)).await {
        Ok(result) => result,
        Err(_) => Err(ApiError::BatchTooLarge(format!(
            "the batch took longer than {} seconds; split it into smaller batches",
            TIME_LIMIT.as_secs()
        ))),
    }
}

async fn run(state: &AppState, principal: &Principal, body: BatchBody) -> Result<Value, ApiError> {
    if body.ops.len() > MAX_OPS {
        return Err(ApiError::BatchTooLarge(format!(
            "{} operations; a batch takes at most {MAX_OPS}",
            body.ops.len()
        )));
    }
    let size = serde_json::to_vec(&body.ops).map(|v| v.len()).unwrap_or(0);
    if size > MAX_BYTES {
        return Err(ApiError::BatchTooLarge(format!(
            "{size} bytes of operations; a batch takes at most {MAX_BYTES}"
        )));
    }
    let dry_run = body.dry_run.unwrap_or(true);
    let per_op = match body.commit.as_deref() {
        None | Some("single") => false,
        Some("per_op") => true,
        Some(other) => {
            return Err(invalid(
                "commit",
                format!("{other:?}: use single or per_op"),
            ))
        }
    };
    let atomic = match body.mode.as_deref() {
        None | Some("atomic") => true,
        Some("continue_on_error") => false,
        Some(other) => {
            return Err(invalid(
                "mode",
                format!("{other:?}: use atomic or continue_on_error"),
            ))
        }
    };
    if !atomic && !dry_run {
        return Err(invalid(
            "mode",
            "continue_on_error lists every failure in a dry run; a batch is committed only atomically".to_string(),
        ));
    }
    let mut seen = HashSet::new();
    for op in &body.ops {
        if let Some(r) = &op.client_ref {
            if !seen.insert(r.clone()) {
                return Err(invalid(
                    "client_ref",
                    format!("client_ref {r:?} is used twice in this batch"),
                ));
            }
        }
    }
    let mut rewrite = false;
    for (i, op) in body.ops.iter().enumerate() {
        if REWRITE_TOOLS.contains(&op.tool.as_str()) {
            rewrite = true;
        } else if !ORDINARY_TOOLS.contains(&op.tool.as_str()) {
            return Err(op_error(
                i,
                &op.client_ref,
                invalid(
                    "tool",
                    format!(
                        "{:?} can't run in a batch; these can: {}, {}",
                        op.tool,
                        ORDINARY_TOOLS.join(", "),
                        REWRITE_TOOLS.join(", ")
                    ),
                ),
            ));
        }
    }
    // The highest role among the operations.
    if rewrite {
        check_rewrite_gates(state, principal)?;
    } else {
        require(principal, Role::Write)?;
    }
    if let (Some(id), false) = (&body.batch_id, dry_run) {
        if let Some(mut first) = recorded(state, id).await {
            first["replayed"] = json!(true);
            return Ok(first);
        }
    }

    let commodity = service::books_commodity(state).await?;
    let mut wc = WorkingCopy::load(state, &body.head)
        .await
        .map_err(ApiError::from)?;

    let mut ids: HashMap<String, u64> = HashMap::new();
    let mut written_by: HashMap<u64, usize> = HashMap::new();
    let mut results: Vec<OpResult> = Vec::new();
    let mut steps: Vec<(ChangeSet, String, &'static str)> = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (index, op) in body.ops.into_iter().enumerate() {
        let attempt = (|| -> Result<ops::Applied, ApiError> {
            let mut args = op.args.clone();
            if args.is_null() {
                args = json!({});
            }
            resolve_refs(&mut args, &ids)?;
            let write_op = crate::mcp::tools::op_from_args(&op.tool, args, &commodity)?;
            ops::apply(&mut wc, write_op).map_err(ApiError::from)
        })();
        match attempt {
            Ok(applied) => {
                if let (Some(r), Some(id)) = (&op.client_ref, applied.id) {
                    ids.insert(r.clone(), id);
                }
                for tx in &applied.after {
                    written_by.insert(tx.id, index);
                }
                if per_op {
                    let changes = wc.checkpoint();
                    if !changes.is_empty() {
                        steps.push((changes, applied.subject.clone(), applied.kind));
                    }
                }
                before.extend(applied.before);
                after.extend(applied.after);
                results.push(OpResult {
                    index,
                    client_ref: op.client_ref,
                    status: "ok",
                    id: applied.id,
                    warnings: applied.warnings,
                    error: None,
                });
            }
            Err(error) if atomic => return Err(op_error(index, &op.client_ref, error)),
            Err(error) => results.push(OpResult {
                index,
                client_ref: op.client_ref,
                status: "error",
                id: None,
                warnings: Vec::new(),
                error: Some(error.body()),
            }),
        }
    }

    // One check of the final state.
    let ok = results.iter().filter(|r| r.status == "ok").count();
    let mut batch_error = None;
    if ok > 0 {
        if let Err(e) = ops::check(&wc) {
            let index = match &e {
                OpError::Check(failures) => blame(failures, &written_by),
                _ => None,
            };
            let client_ref = index.and_then(|i| results[i].client_ref.clone());
            let error = ApiError::from(e);
            if atomic {
                return Err(ApiError::BatchFailed {
                    index,
                    client_ref,
                    error: Box::new(error),
                });
            }
            batch_error = Some(json!({
                "index": index,
                "client_ref": client_ref,
                "error": error.body(),
            }));
        }
    }

    // Flags for card transactions still missing their reserve lines.
    let txs = wc.transactions();
    for (i, missing) in kabacus_core::unreserved_flags(&wc.rules_chart(), &txs) {
        let id = txs[i].id;
        if let Some(&op) = written_by.get(&id) {
            if let Some(r) = results.iter_mut().find(|r| r.index == op) {
                for m in missing {
                    r.warnings.push(ops::OpWarning {
                        code: "unreserved".to_string(),
                        message: format!(
                            "#{id} is missing its reserve line: {} still needs {}",
                            m.reserve,
                            kabacus_core::format_money(&m.amount)
                        ),
                    });
                }
            }
        }
    }

    let (changes, truncated) = ops::balance_changes(&wc, &before, &after);
    let errors = results.len() - ok;
    let mut result = json!({
        "head": body.head,
        "dry_run": true,
        "ops": results,
        "balance_changes": balance_change_dtos(&changes),
        "cash_on_hand": crate::service::CashPreviewDto::from(&ops::cash_change(&wc)),
        "counts": {"ok": ok, "error": errors},
    });
    if truncated {
        result["truncated"] = json!(true);
    }
    if let Some(e) = batch_error {
        result["batch_error"] = e;
    }
    if let Some(id) = &body.batch_id {
        result["batch_id"] = json!(id);
    }
    if dry_run {
        return Ok(result);
    }

    let subject = format!("batch: {ok} operation(s)");
    if !per_op {
        let changes = wc.changes();
        steps = if changes.is_empty() {
            Vec::new()
        } else {
            vec![(changes, subject, "batch")]
        };
    }
    let mut revision = wc.revision;
    let last = steps.len();
    for (n, (changes, subject, kind)) in steps.into_iter().enumerate() {
        let mut who = ops::who(state, principal, kind, &subject)
            .await
            .map_err(ApiError::from)?;
        if let Some(id) = &body.batch_id {
            who.details["batch_id"] = json!(id);
            if n + 1 == last {
                let mut recorded = result.clone();
                recorded["head"] = json!((revision + 1).to_string());
                recorded["dry_run"] = json!(false);
                who.details["result"] = recorded;
            }
        }
        revision = ops::commit(state, changes, &who, revision)
            .await
            .map_err(ApiError::from)?;
    }
    if revision != wc.revision {
        ops::remember(state, &wc, revision).await;
    }
    result["head"] = json!(revision.to_string());
    result["dry_run"] = json!(false);
    Ok(result)
}
