// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{expired, feed, links, matches_link, store_error, warning_values, Snapshot, Window};
use crate::bank_import::{plan_statement, ImportOptions};
use crate::books::WorkingCopy;
use crate::error::Error;
use crate::Service;
use chrono::NaiveDate;
use kabacus_core::bank_import::simplefin::{utc_date, Account, AccountSet};
use kabacus_core::bank_import::{currency_code, ImportError, Outcome, SourceKind};
use kabacus_store::simplefin::{Access, Owner};
use kabacus_store::BookStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub(super) struct Pending {
    pub(super) id: String,
    end: NaiveDate,
    pub(super) warnings: Vec<Value>,
    accounts: BTreeMap<String, Account>,
}

impl Pending {
    pub(super) fn load(access: &Access) -> Result<Option<Self>, Error> {
        access
            .pending
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| store_error(kabacus_store::StoreError::Invalid(e.to_string())))
    }

    pub(super) fn since(&self) -> NaiveDate {
        self.accounts
            .values()
            .flat_map(|account| &account.transactions)
            .filter_map(|row| utc_date(row.posted))
            .min()
            .unwrap_or(self.end)
    }

    pub(super) fn snapshot(&self, access: &Access) -> Snapshot {
        Snapshot {
            id: self.id.clone(),
            access: access.clone(),
            at: 0,
            window: Some(Window {
                since: self.since(),
                end: self.end,
            }),
            data: Arc::new(AccountSet {
                accounts: self.accounts.values().cloned().collect(),
                connections: Vec::new(),
                errlist: Vec::new(),
                errors: Vec::new(),
            }),
        }
    }

    pub(super) fn account(
        &self,
        target: &str,
        key: &str,
        host: &Option<String>,
        domain: &str,
    ) -> Option<&Account> {
        self.accounts
            .get(target)
            .filter(|account| matches_link(key, host, account, domain))
    }

    pub(super) fn count(&self, wc: &WorkingCopy, domain: &str) -> Result<usize, Error> {
        let mut count = 0;
        for (target, (key, host)) in links(&wc.book) {
            if let Some(account) = self.account(&target, &key, &host, domain) {
                count += unresolved(wc, &target, &key, domain, account)?.0.len();
            }
        }
        Ok(count)
    }
}

fn unresolved(
    wc: &WorkingCopy,
    target: &str,
    key: &str,
    domain: &str,
    account: &Account,
) -> Result<(HashSet<String>, usize), Error> {
    let (mut statement, _) = account.statement()?;
    let plan = plan_statement(
        wc,
        target,
        &mut statement,
        &ImportOptions::default(),
        Some(&feed(wc, target, domain, key)),
    )?;
    let mut matches = 0;
    let mut ids = HashSet::new();
    for row in plan.rows {
        matches += usize::from(matches!(row.outcome, Outcome::Matched(_)));
        if matches!(row.outcome, Outcome::New(_) | Outcome::Matched(_)) {
            ids.insert(row.row.import_id);
        }
    }
    Ok((ids, matches))
}

pub(super) async fn stage(
    state: &Service,
    owner: Owner,
    access: &Access,
    wc: &WorkingCopy,
    snapshot: &Snapshot,
    starts: &BTreeMap<String, NaiveDate>,
    domain: &str,
) -> Result<Value, Error> {
    let end = snapshot.window.expect("transaction fetch has a window").end;
    let previous = Pending::load(access)?;
    let current_links = links(&wc.book);
    let mut pending = Pending {
        id: snapshot.id.clone(),
        end: previous.as_ref().map_or(end, |p| p.end.max(end)),
        warnings: warning_values(snapshot),
        accounts: BTreeMap::new(),
    };
    let mut added = 0;
    let mut matched = 0;
    let mut fetched_rows = 0;
    for (target, (key, host)) in current_links {
        let old = previous
            .as_ref()
            .and_then(|p| p.account(&target, &key, &host, domain));
        let fresh = starts.get(&target).and_then(|start| {
            let mut account = snapshot
                .data
                .accounts
                .iter()
                .find(|account| {
                    matches_link(&key, &host, account, domain)
                        && !snapshot.data.errlist.iter().any(|w| w.blocks(account))
                })?
                .clone();
            fetched_rows += account.transactions.len();
            account.transactions.retain(|row| {
                !row.pending
                    && row.posted != 0
                    && utc_date(row.posted).is_none_or(|date| date >= *start && date < end)
            });
            Some(account)
        });
        if fetched_rows > state.settings.statement_limits.max_rows {
            return Err(ImportError::TooLarge.into());
        }
        let mut account = match (fresh, old) {
            (Some(mut fresh), Some(old)) => {
                if currency_code(&fresh.currency)? != currency_code(&old.currency)? {
                    return Err(ImportError::Field {
                        location: fresh.key(),
                        field: "currency".into(),
                    }
                    .into());
                }
                let seen: HashSet<_> = fresh.transactions.iter().map(|row| &row.id).collect();
                let older: Vec<_> = old
                    .transactions
                    .iter()
                    .filter(|row| !seen.contains(&row.id))
                    .cloned()
                    .collect();
                fresh.transactions.extend(older);
                fresh
            }
            (Some(fresh), None) => fresh,
            (None, Some(old)) => old.clone(),
            (None, None) => continue,
        };
        let (mut ids, matches) = unresolved(wc, &target, &key, domain, &account)?;
        added += ids.len() - matches;
        matched += matches;
        account
            .transactions
            .retain(|row| ids.remove(&SourceKind::Simplefin.import_id(&row.id)));
        if !account.transactions.is_empty() {
            pending.accounts.insert(target, account);
        }
    }
    let encoded = serde_json::to_string(&pending)
        .map_err(|e| store_error(kabacus_store::StoreError::Invalid(e.to_string())))?;
    if !state
        .store
        .save_simplefin_pending(state.ledger, owner, access, &encoded)
        .await
        .map_err(store_error)?
    {
        return Err(expired());
    }
    Ok(
        json!({"head": wc.head, "counts": {"new": added, "matched": matched}, "pending_count": added + matched, "warnings": pending.warnings}),
    )
}
