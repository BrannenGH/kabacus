// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Renaming and removing accounts, in every declaration, posting and tag
//! value. Both are rewrites.

use crate::books::WorkingCopy;
use crate::ops::{Applied, OpError, OpWarning};
use kabacus_core::{AccountKind, AccountName};
use std::collections::BTreeMap;

/// `name` under `from` moved under `to`, or `None` if it isn't under
/// `from`.
fn moved(name: &str, from: &str, to: &str) -> Option<String> {
    if name == from {
        Some(to.to_string())
    } else {
        name.strip_prefix(from)
            .filter(|rest| rest.starts_with(':'))
            .map(|rest| format!("{to}{rest}"))
    }
}

/// Tag values in a comment that name an account under `from`, moved, and
/// how many there were.
fn rename_in_comment(comment: &str, from: &str, to: &str) -> (String, usize) {
    let mut count = 0;
    let lines: Vec<String> = comment
        .split('\n')
        .map(|line| {
            line.split(',')
                .map(|part| {
                    let Some((name, value)) = part.split_once(':') else {
                        return part.to_string();
                    };
                    let trimmed = value.trim();
                    match moved(trimmed, from, to) {
                        Some(new) if name.split_whitespace().last().is_some() => {
                            count += 1;
                            let lead = &value[..value.len() - value.trim_start().len()];
                            let trail = &value[value.trim_end().len()..];
                            format!("{name}:{lead}{new}{trail}")
                        }
                        _ => part.to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    (lines.join("\n"), count)
}

/// Rename `from` (and every account under it) to `to`, in every
/// declaration, posting, balance assertion and tag value.
pub fn rename(
    wc: &mut WorkingCopy,
    from: &str,
    to: &str,
    merge: bool,
    allow_kind_change: bool,
) -> Result<Applied, OpError> {
    kabacus_core::render_account_declaration(to, &[])
        .map_err(|message| OpError::invalid("account", message))?;
    if from == to {
        return Err(OpError::invalid(
            "account",
            "from and to are the same account",
        ));
    }
    if AccountName::new(to).is_under(from) {
        return Err(OpError::invalid(
            "account",
            format!("{to} is under {from}; an account can't move into itself"),
        ));
    }
    let chart = wc.chart();
    let rules = wc.rules_chart();
    let affected: Vec<&kabacus_core::ClassifiedAccount> = chart
        .accounts
        .iter()
        .filter(|c| c.account.is_under(from))
        .collect();
    if affected.is_empty() {
        return Err(OpError::NotFound(from.to_string()));
    }
    let to_exists = chart.accounts.iter().any(|c| c.account.is_under(to));
    if to_exists && !merge {
        return Err(OpError::invalid(
            "account",
            format!("{to} already exists; pass merge: true to merge {from} into it"),
        ));
    }
    let from_kind = chart.kind_of(&AccountName::new(from));
    if to_exists {
        let to_kind = chart.kind_of(&AccountName::new(to));
        if to_kind != from_kind {
            return Err(OpError::invalid(
                "account",
                format!("{from} is {from_kind} but {to} is {to_kind}; only accounts of the same kind merge"),
            ));
        }
    }
    let mut kind_changes = Vec::new();
    for c in affected
        .iter()
        .filter(|c| !matches!(c.kind, AccountKind::Parent))
    {
        let new = AccountName::new(moved(c.account.as_str(), from, to).expect("under from"));
        let new_kind = rules.kind_of(&new);
        if new_kind != c.kind {
            kind_changes.push(serde_json::json!({
                "account": c.account.to_string(),
                "to": new.to_string(),
                "old_kind": c.kind,
                "new_kind": new_kind,
            }));
        }
    }
    if !kind_changes.is_empty() && !allow_kind_change {
        return Err(OpError::invalid(
            "kind_change",
            format!(
                "renaming {from} to {to} changes the kind of {} account(s) ({}); pass allow_kind_change: true to do it anyway",
                kind_changes.len(),
                kind_changes
                    .iter()
                    .map(|k| format!("{}: {} -> {}", k["account"].as_str().unwrap_or_default(), k["old_kind"].as_str().unwrap_or_default(), k["new_kind"].as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }

    let mut applied = Applied::new(None, "rewrite", String::new());
    let mut occurrences = 0;
    let mut samples: Vec<String> = Vec::new();

    // Declarations: moved, or dropped where `to` already declares one.
    let declared: Vec<String> = wc
        .book
        .accounts
        .keys()
        .filter(|a| moved(a, from, to).is_some())
        .cloned()
        .collect();
    for old in declared {
        let new = moved(&old, from, to).expect("under from");
        let tags = wc.book.accounts[&old].tags.clone();
        occurrences += 1;
        if let Some(change) = wc.undeclare(&old) {
            applied.changes.push(change);
        }
        if !wc.is_declared(&new) {
            let (tags, n) = rename_in_comment(&tags, from, to);
            occurrences += n;
            applied.changes.push(wc.declare(&new, &tags));
        }
    }
    // Transactions: postings, assertions and tag values.
    for i in 0..wc.book.transactions.len() {
        let before = wc.tx(i).clone();
        let mut tx = before.clone();
        let mut count = 0;
        let (comment, n) = rename_in_comment(&tx.comment, from, to);
        tx.comment = comment;
        tx.tags = kabacus_core::declared_tags(&tx.comment);
        count += n;
        for p in &mut tx.postings {
            if let Some(new) = moved(p.account.as_str(), from, to) {
                p.account = AccountName::new(new);
                count += 1;
            }
            let (comment, n) = rename_in_comment(&p.comment, from, to);
            p.comment = comment;
            p.tags = kabacus_core::declared_tags(&p.comment);
            count += n;
        }
        if count == 0 {
            continue;
        }
        occurrences += count;
        let change = wc.replace(i, tx)?;
        if samples.len() < 3 {
            samples.extend(change.after.clone());
        }
        applied.changes.push(change);
        applied.before.push(before);
        applied.after.push(wc.tx(i).clone());
    }

    applied.subject = format!("rename account: {from} -> {to} ({occurrences} occurrence(s))");
    applied.details = serde_json::json!({
        "from": from,
        "to": to,
        "merged": to_exists,
        "occurrences": occurrences,
        "transactions": applied.after.len(),
        "samples": samples,
        "kind_changes": kind_changes,
    });
    if !kind_changes.is_empty() {
        applied.warnings.push(OpWarning {
            code: "kind_change".to_string(),
            message: format!(
                "renaming {from} to {to} changes the kind of {} account(s)",
                kind_changes.len()
            ),
        });
    }
    // Renaming moves balances, not amounts.
    applied.before.clear();
    applied.after.clear();
    Ok(applied)
}

/// Remove `account`'s declaration, if nothing refers to it.
pub fn remove(wc: &mut WorkingCopy, account: &str) -> Result<Applied, OpError> {
    let name = AccountName::new(account);
    if !wc.is_declared(account) {
        return Err(OpError::NotFound(account.to_string()));
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for kind in ["postings", "assertions", "directives", "sub_accounts"] {
        counts.insert(kind.to_string(), 0);
    }
    for p in wc.book.transactions.iter().flat_map(|t| &t.postings) {
        if p.account == name {
            *counts.get_mut("postings").expect("listed") += 1;
            if p.balance_assertion.is_some() {
                *counts.get_mut("assertions").expect("listed") += 1;
            }
        }
    }
    let chart = wc.chart();
    counts.insert(
        "sub_accounts".to_string(),
        chart
            .accounts
            .iter()
            .filter(|c| c.account != name && c.account.is_under(account))
            .count(),
    );
    if counts.values().any(|n| *n > 0) {
        return Err(OpError::AccountInUse {
            account: account.to_string(),
            counts,
        });
    }
    let change = wc.undeclare(account).expect("declared");
    let mut applied = Applied::new(None, "rewrite", format!("remove account: {account}"));
    applied.details = serde_json::json!({
        "account": account,
        "removed": change.before.clone().into_iter().collect::<Vec<_>>(),
    });
    applied.changes.push(change);
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renames_tag_values_naming_the_account() {
        let (comment, n) = rename_in_comment(
            "fund:assets:bank:checking:card-sc, paid\nnote:assets:bank:checking:card-scx",
            "assets:bank:checking:card-sc",
            "assets:bank:checking:card-store-card",
        );
        assert_eq!(n, 1);
        assert_eq!(
            comment,
            "fund:assets:bank:checking:card-store-card, paid\nnote:assets:bank:checking:card-scx"
        );
    }

    #[test]
    fn moves_an_account_and_those_under_it_only() {
        assert_eq!(moved("a:b", "a:b", "x").as_deref(), Some("x"));
        assert_eq!(moved("a:b:c", "a:b", "x").as_deref(), Some("x:c"));
        assert_eq!(moved("a:bc", "a:b", "x"), None);
    }
}
