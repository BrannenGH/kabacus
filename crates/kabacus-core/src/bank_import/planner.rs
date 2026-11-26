// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{clean_text, encode_id, ImportError, Row, SourceKind};
use crate::funds::{card_and_reserve, UNCATEGORIZED};
use crate::{
    plan_card_purchase, AccountKind, AccountName, Chart, Money, PlannedTransaction, PostingKind,
    PurchaseSource, Tag, Transaction,
};
use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

const IMPORT_ID: &str = "import-id";
const IMPORT_TARGET: &str = "import-target";
const SIMPLEFIN_FEED: &str = "simplefin-feed";

/// Record the target separately from the public import ID so another posting
/// in the transaction cannot make the same ID a duplicate in a second account.
pub fn import_tags(target: &str, id: &str) -> [Tag; 2] {
    [
        (IMPORT_ID.into(), id.into()),
        (IMPORT_TARGET.into(), format!("{}/{id}", encode_id(target))),
    ]
}

pub struct SimplefinFeed {
    pub key: String,
    pub accept_legacy: bool,
}

impl SimplefinFeed {
    pub fn tag(&self) -> Tag {
        (SIMPLEFIN_FEED.into(), self.key.clone())
    }

    pub fn matches(&self, tx: &Transaction) -> bool {
        tx.tag(SIMPLEFIN_FEED)
            .map_or(self.accept_legacy, |key| key == self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Decision {
    pub import_id: String,
    #[serde(flatten)]
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Add,
    Skip,
    Match { transaction: u64 },
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Duplicate,
    Skipped,
    Matched(u64),
    New(PlannedTransaction),
}

#[derive(Debug, Clone)]
pub struct PlannedRow {
    pub row: Row,
    pub outcome: Outcome,
    pub candidates: Vec<u64>,
}

pub struct PlanInput<'a> {
    pub chart: &'a Chart,
    pub transactions: &'a [Transaction],
    pub target: &'a str,
    pub commodity: &'a str,
    pub source: SourceKind,
    pub simplefin_feed: Option<&'a SimplefinFeed>,
    pub since: Option<NaiveDate>,
    pub decisions: &'a [Decision],
}

pub fn plan_rows(input: PlanInput<'_>, rows: Vec<Row>) -> Result<Vec<PlannedRow>, ImportError> {
    let PlanInput {
        chart,
        transactions,
        target,
        commodity,
        source,
        simplefin_feed,
        since,
        decisions,
    } = input;
    let account = chart
        .get(target)
        .filter(|a| !a.synthetic)
        .ok_or_else(|| ImportError::AccountKind(target.into()))?;
    match account.kind {
        AccountKind::Money => (),
        AccountKind::RevolvingDebt => {
            card_and_reserve(chart, target)?;
        }
        _ => return Err(ImportError::AccountKind(target.into())),
    }
    let mut overrides = HashMap::new();
    let mut row_ids = HashMap::new();
    for row in &rows {
        let value = (row.date, &row.amount);
        if row_ids
            .insert(row.import_id.as_str(), value)
            .is_some_and(|prior| prior != value)
        {
            return Err(ImportError::Field {
                location: row.import_id.clone(),
                field: "ID".into(),
            });
        }
    }
    for decision in decisions {
        let id = decision.import_id.as_str();
        if !row_ids.contains_key(id) || overrides.insert(id, &decision.action).is_some() {
            return Err(ImportError::Decision(id.into()));
        }
    }

    let target_prefix = format!("{}/", encode_id(target));
    let source_prefix = format!("{}:", source.name());
    let mut ids = HashSet::new();
    let mut eligible: HashMap<&Money, BTreeMap<NaiveDate, Vec<u64>>> = HashMap::new();
    for tx in transactions {
        let target_amounts: HashSet<_> = tx
            .postings
            .iter()
            .filter(|p| {
                p.account == account.account
                    && p.kind == PostingKind::Real
                    && p.amount.commodity == commodity
            })
            .map(|p| &p.amount.money)
            .collect();
        if target_amounts.is_empty() {
            continue;
        }
        let scoped: HashSet<_> = tx
            .tags
            .iter()
            .filter(|(key, _)| key == IMPORT_TARGET)
            .filter_map(|(_, value)| value.split_once('/').map(|(_, id)| id))
            .collect();
        if simplefin_feed.is_none_or(|feed| feed.matches(tx)) {
            for (key, value) in &tx.tags {
                if key == IMPORT_TARGET {
                    if let Some(id) = value.strip_prefix(&target_prefix) {
                        ids.insert(id.to_owned());
                    }
                } else if key == IMPORT_ID && !scoped.contains(value.as_str()) {
                    ids.insert(value.clone());
                }
            }
        }
        if tx
            .tags
            .iter()
            .any(|(key, value)| key == IMPORT_ID && value.starts_with(&source_prefix))
        {
            continue;
        }
        for money in target_amounts {
            eligible
                .entry(money)
                .or_default()
                .entry(tx.date)
                .or_default()
                .push(tx.id);
        }
    }

    let mut claimed = HashSet::new();
    rows.into_iter()
        .map(|mut row| {
            if let Some(memo) = row.description.strip_prefix(" | ") {
                row.description = format!("{} | {memo}", clean_text(&account.display_name));
            } else if row.description.is_empty() {
                row.description = clean_text(&account.display_name);
            }
            let override_action = overrides.get(row.import_id.as_str()).copied();
            let mut candidates = Vec::new();
            let outcome = if ids.contains(&row.import_id) {
                Outcome::Duplicate
            } else if row.amount.is_zero()
                || since.is_some_and(|since| row.date < since)
                || override_action == Some(&Action::Skip)
            {
                if matches!(override_action, Some(Action::Match { .. })) {
                    return Err(ImportError::Decision(row.import_id));
                }
                Outcome::Skipped
            } else {
                if let Some(dates) = eligible.get(&row.amount) {
                    let first = row
                        .date
                        .checked_sub_days(Days::new(5))
                        .unwrap_or(NaiveDate::MIN);
                    let last = row.date.succ_opt().unwrap_or(NaiveDate::MAX);
                    let mut matches: Vec<_> = dates
                        .range(first..=last)
                        .flat_map(|(date, ids)| {
                            ids.iter()
                                .filter(|id| !claimed.contains(*id))
                                .map(move |id| {
                                    (row.date.signed_duration_since(*date).num_days().abs(), *id)
                                })
                        })
                        .collect();
                    matches.sort_unstable();
                    candidates = matches.into_iter().map(|(_, id)| id).collect();
                }
                let chosen = match override_action {
                    Some(Action::Match { transaction }) if candidates.contains(transaction) => {
                        Some(*transaction)
                    }
                    Some(Action::Match { .. }) => return Err(ImportError::Decision(row.import_id)),
                    Some(Action::Add) => None,
                    _ => candidates.first().copied(),
                };
                ids.insert(row.import_id.clone());
                if let Some(id) = chosen {
                    claimed.insert(id);
                    Outcome::Matched(id)
                } else {
                    Outcome::New(if account.kind == AccountKind::RevolvingDebt {
                        plan_card_purchase(
                            chart,
                            target,
                            UNCATEGORIZED,
                            -row.amount.clone(),
                            &PurchaseSource::Free,
                            &row.description,
                        )?
                    } else {
                        PlannedTransaction {
                            description: row.description.clone(),
                            postings: vec![
                                (account.account.clone(), row.amount.clone()),
                                (AccountName::new(UNCATEGORIZED), -row.amount.clone()),
                            ],
                        }
                    })
                }
            };
            Ok(PlannedRow {
                row,
                outcome,
                candidates,
            })
        })
        .collect()
}
