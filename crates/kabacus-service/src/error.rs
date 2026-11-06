// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::ops::OpError;
use crate::service::invalid;

#[derive(Debug)]
pub enum Error {
    Denied {
        permission: crate::caller::Permission,
        body: ErrorBody,
    },
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
        error: Box<Error>,
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

impl Error {
    /// The stable error code clients match on.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Denied { .. } => "role_required",
            Error::NotFound(_) => "not_found",
            Error::Conflict { .. } => "head_conflict",
            Error::Invalid { .. } => "validation_failed",
            Error::Internal(_) => "internal",
            Error::RewriteDisabled => "rewrite_disabled",
            Error::AmountFormat { .. } => "amount_format",
            Error::AccountInUse { .. } => "account_in_use",
            Error::BatchTooLarge(_) => "batch_too_large",
            Error::ReconcileMismatch(_) => "reconcile_mismatch",
            Error::BatchFailed { error, .. } => error.code(),
        }
    }

    pub fn body(&self) -> ErrorBody {
        let (message, path, details) = match self {
            Error::Denied { body, .. } => return body.clone(),
            Error::NotFound(id) => (
                format!("{id} was not found"),
                None,
                Some(serde_json::json!({ "id": id })),
            ),
            Error::Conflict { expected, current } => (
                format!("the books changed since head {expected} (now {current}); re-read and retry with the new head"),
                None,
                Some(serde_json::json!({ "expected": expected, "current_head": current })),
            ),
            Error::Invalid { rule, message, failures } => {
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
            Error::Internal(e) => (e.to_string(), None, None),
            Error::RewriteDisabled => (
                "rewrites are disabled: the server was started with --no-rewrite or allow_rewrite = false".to_string(),
                None,
                None,
            ),
            Error::AmountFormat { path, message } => (message.clone(), Some(path.clone()), None),
            Error::AccountInUse { account, counts } => (
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
            Error::BatchTooLarge(message) => (message.clone(), None, None),
            Error::ReconcileMismatch(result) => (
                "the ledger balance doesn't match the statement; nothing was written".to_string(),
                None,
                Some(result.clone()),
            ),
            Error::BatchFailed { index, client_ref, error } => {
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

impl From<OpError> for Error {
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
            OpError::NotFound(id) => Error::NotFound(id),
            OpError::Conflict { expected, current } => Error::Conflict { expected, current },
            OpError::Invalid { rule, message } => invalid(&rule, message),
            OpError::Check(failures) => Error::Invalid {
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
            OpError::AccountInUse { account, counts } => Error::AccountInUse { account, counts },
            other => Error::Internal(anyhow::anyhow!(other.to_string())),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.body().message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Internal(error) => Some(error.as_ref()),
            Error::BatchFailed { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}
