// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A whole book in memory: declared accounts, declared commodities with
//! their styles, and transactions. Implements the engine implementation
//! specification, section 2.2 (`Book`).

use crate::amount::Style;
use kabacus_core::{AccountName, Transaction};
use std::collections::BTreeMap;

/// A declared account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: AccountName,
    /// The declaration's comment, tags included, as written.
    pub tags: String,
}

/// A declared commodity and how its amounts are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commodity {
    pub symbol: String,
    pub style: Style,
}

/// Every commodity's style, by symbol.
pub type Styles = BTreeMap<String, Style>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book {
    /// By name, in byte order.
    pub accounts: BTreeMap<String, Account>,
    /// By symbol, in byte order.
    pub commodities: BTreeMap<String, Commodity>,
    /// In id order.
    pub transactions: Vec<Transaction>,
}

impl Book {
    pub fn is_declared(&self, account: &str) -> bool {
        self.accounts.contains_key(account)
    }

    /// Every declared account name, in byte order.
    pub fn declared_accounts(&self) -> impl Iterator<Item = &AccountName> {
        self.accounts.values().map(|a| &a.name)
    }

    /// The style an amount of `symbol` is shown in.
    pub fn style(&self, symbol: &str) -> Style {
        self.commodities
            .get(symbol)
            .map(|c| c.style.clone())
            .unwrap_or_else(|| Style::default_for(symbol))
    }

    pub fn styles(&self) -> Styles {
        self.commodities
            .iter()
            .map(|(symbol, c)| (symbol.clone(), c.style.clone()))
            .collect()
    }

    /// The next transaction id: one more than the highest so far.
    pub fn next_id(&self) -> u64 {
        self.transactions.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    pub fn transaction(&self, id: u64) -> Option<&Transaction> {
        self.transactions.iter().find(|t| t.id == id)
    }
}

/// The style for `symbol` in `styles`, or the default for it.
pub fn style_in(styles: &Styles, symbol: &str) -> Style {
    styles
        .get(symbol)
        .cloned()
        .unwrap_or_else(|| Style::default_for(symbol))
}
