// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Settling up with a person: money you paid them or they paid you, netted
//! against what's already owed by the core's counterparty rule.
use crate::{
    caller::{Caller, Permission as Role},
    error::Error as ApiError,
    service::{self, AmountInput, DraftPostingDto},
    state::Service,
};
use kabacus_core::{AccountKind, AccountName, Balances, FlowDirection, Money};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettleDirection {
    /// You paid them.
    Paid,
    /// They paid you.
    Received,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SettleBody {
    pub date: String,
    pub description: String,
    /// The money account the payment left or reached.
    pub account: String,
    pub person: String,
    pub amount: String,
    pub direction: SettleDirection,
    #[serde(default = "preview_default")]
    pub dry_run: bool,
}

pub async fn record(
    state: &Service,
    principal: &dyn Caller,
    body: SettleBody,
    head: &str,
) -> Result<service::WriteResultDto, ApiError> {
    service::require(principal, Role::Write)?;
    let commodity = service::books_commodity(state).await?;
    let (chart, txs) = service::books_chart(state).await?;
    let draft = draft(body, &commodity, &chart, &txs)?;
    service::add_transaction(state, principal, draft, head).await
}

pub fn draft(
    body: SettleBody,
    commodity: &str,
    chart: &kabacus_core::Chart,
    txs: &[kabacus_core::Transaction],
) -> Result<service::AddTransactionBody, ApiError> {
    let amount = AmountInput::decimal(&body.amount)
        .to_amount(commodity, "amount")?
        .money;
    if !amount.is_positive() {
        return Err(service::invalid(
            "settle",
            "The amount must be more than zero".into(),
        ));
    }
    let account = AccountName::new(&body.account);
    let card = chart.kind_of(&account) == AccountKind::RevolvingDebt;
    if card && body.direction == SettleDirection::Received {
        return Err(service::invalid(
            "settle",
            "A payment to you can't go onto a card".into(),
        ));
    }
    let balances = Balances::compute(txs, commodity, None);
    let person = body.person.trim().to_lowercase();
    let receivable = AccountName::new(format!("assets:receivable:{person}"));
    let payable = AccountName::new(format!("liabilities:payable:{person}"));
    let paid = body.direction == SettleDirection::Paid;
    let sides = kabacus_core::select_counterparty_sides(
        balances.exclusive(&receivable).money.max(Money::zero()),
        balances.exclusive(&payable).money.min(Money::zero()),
        amount.clone(),
        if paid {
            FlowDirection::ToPerson
        } else {
            FlowDirection::FromPerson
        },
    );
    let mut postings = Vec::new();
    let mut post = |account: String, money: Money| {
        if !money.is_zero() {
            postings.push(DraftPostingDto {
                cost: None,
                clear_cost: false,
                account,
                amount: Some(AmountInput::decimal(&money.to_string())),
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            });
        }
    };
    post(receivable.to_string(), sides.receivable_delta);
    post(payable.to_string(), sides.payable_delta);
    post(
        body.account.clone(),
        if paid {
            -amount.clone()
        } else {
            amount.clone()
        },
    );
    if card {
        let reserve = chart.reserve_for(&account);
        let home = chart
            .get(reserve.as_str())
            .and_then(|a| a.home.clone())
            .ok_or_else(|| service::invalid("reserve", "The card has no reserve account".into()))?;
        post(home.to_string(), -amount.clone());
        post(reserve.to_string(), amount);
    }
    Ok(service::AddTransactionBody {
        date: body.date,
        description: body.description,
        postings,
        tags: vec![],
        new_accounts: vec![],
        dry_run: body.dry_run,
        include_diff: false,
    })
}

fn preview_default() -> bool {
    true
}
