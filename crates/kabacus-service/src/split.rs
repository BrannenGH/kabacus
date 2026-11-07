// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared-expense requests use the core's counterparty netting rule.
use crate::{
    caller::{Caller, Permission as Role},
    error::Error as ApiError,
    service::{self, AmountInput, DraftPostingDto},
    state::Service,
};
use kabacus_core::{AccountKind, AccountName, Balances, FlowDirection, Money};

use serde::Deserialize;
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SplitBody {
    pub date: String,
    pub description: String,
    pub paid_with: String,
    pub category: String,
    pub person: String,
    pub total: String,
    pub their_share: String,
    #[serde(default = "preview_default")]
    pub dry_run: bool,
}
pub async fn record(
    state: &Service,
    principal: &dyn Caller,
    body: SplitBody,
    head: &str,
) -> Result<service::WriteResultDto, ApiError> {
    service::require(principal, Role::Write)?;
    let commodity = service::books_commodity(state).await?;
    let (chart, txs) = service::books_chart(state).await?;
    let draft = draft(body, &commodity, &chart, &txs)?;
    service::add_transaction(state, principal, draft, head).await
}

pub fn draft(
    body: SplitBody,
    commodity: &str,
    chart: &kabacus_core::Chart,
    txs: &[kabacus_core::Transaction],
) -> Result<service::AddTransactionBody, ApiError> {
    let total = AmountInput::decimal(&body.total)
        .to_amount(commodity, "total")?
        .money;
    let theirs = AmountInput::decimal(&body.their_share)
        .to_amount(commodity, "their_share")?
        .money;
    if !total.is_positive() || !theirs.is_positive() || theirs > total {
        return Err(service::invalid(
            "split",
            "Their share must be positive and no more than the total".into(),
        ));
    }
    let balances = Balances::compute(txs, commodity, None);
    let person = body.person.trim().to_lowercase();
    let receivable = AccountName::new(format!("assets:receivable:{person}"));
    let payable = AccountName::new(format!("liabilities:payable:{person}"));
    let sides = kabacus_core::select_counterparty_sides(
        balances.exclusive(&receivable).money.max(Money::zero()),
        balances.exclusive(&payable).money.min(Money::zero()),
        theirs.clone(),
        FlowDirection::ToPerson,
    );
    let mut postings = Vec::new();
    let mut post = |account: String, money: Money| {
        if !money.is_zero() {
            postings.push(DraftPostingDto {
                account,
                amount: Some(AmountInput::decimal(&money.to_string())),
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            });
        }
    };
    post(body.category, total.clone() - theirs);
    post(receivable.to_string(), sides.receivable_delta);
    post(payable.to_string(), sides.payable_delta);
    post(body.paid_with.clone(), -total.clone());
    let paid_with = AccountName::new(&body.paid_with);
    if chart.kind_of(&paid_with) == AccountKind::RevolvingDebt {
        let reserve = chart.reserve_for(&paid_with);
        let home = chart
            .get(reserve.as_str())
            .and_then(|account| account.home.clone())
            .ok_or_else(|| service::invalid("reserve", "The card has no reserve account".into()))?;
        post(home.to_string(), -total.clone());
        post(reserve.to_string(), total);
    }
    Ok(service::AddTransactionBody {
        date: body.date,
        description: body.description,
        postings,
        tags: vec![],
        dry_run: body.dry_run,
        include_diff: false,
    })
}

fn preview_default() -> bool {
    true
}
