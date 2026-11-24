// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{clean_text, encode_id, ImportError, ReportedBalance, Row, SourceKind, Statement};
use chrono::{DateTime, NaiveDate};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct AccountSet {
    pub connections: Vec<Connection>,
    pub accounts: Vec<Account>,
    pub errlist: Vec<Warning>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Clone, Deserialize)]
pub struct Connection {
    pub conn_id: String,
    pub name: String,
}

#[derive(Clone, Deserialize)]
pub struct Account {
    pub id: String,
    pub conn_id: String,
    pub name: String,
    pub currency: String,
    pub balance: String,
    #[serde(rename = "balance-date")]
    pub balance_date: i64,
    #[serde(default)]
    pub transactions: Vec<BankTransaction>,
}

#[derive(Clone, Deserialize)]
pub struct BankTransaction {
    pub id: String,
    pub posted: i64,
    pub amount: String,
    pub description: String,
    pub payee: Option<String>,
    #[serde(default)]
    pub pending: bool,
}

#[derive(Clone, Deserialize)]
pub struct Warning {
    pub code: String,
    pub msg: String,
    pub conn_id: Option<String>,
    pub account_id: Option<String>,
}

pub fn link_key(connection: &str, account: &str) -> String {
    format!("{}/{}", encode_id(connection), encode_id(account))
}

pub fn utc_date(timestamp: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp(timestamp, 0).map(|value| value.date_naive())
}

fn invalid(location: &str, field: &str) -> ImportError {
    ImportError::Field {
        location: location.into(),
        field: field.into(),
    }
}

impl Account {
    pub fn key(&self) -> String {
        link_key(&self.conn_id, &self.id)
    }

    pub fn statement(&self) -> Result<(Statement, usize), ImportError> {
        let mut pending = 0;
        let mut rows = Vec::new();
        for tx in &self.transactions {
            if tx.pending || tx.posted == 0 {
                pending += 1;
                continue;
            }
            let import_id = SourceKind::Simplefin.import_id(&tx.id);
            let date = utc_date(tx.posted).ok_or_else(|| invalid(&import_id, "date"))?;
            let amount = decimal(&tx.amount).ok_or_else(|| invalid(&import_id, "amount"))?;
            let description = clean_text(&tx.description);
            let payee = tx
                .payee
                .as_deref()
                .map(clean_text)
                .filter(|name| !name.is_empty());
            let description = match payee {
                Some(payee) if !description.is_empty() && payee != description => {
                    format!("{payee} | {description}")
                }
                Some(payee) => payee,
                None => description,
            };
            rows.push(Row {
                import_id,
                date,
                amount,
                description,
            });
        }
        let key = self.key();
        Ok((
            Statement {
                source: SourceKind::Simplefin,
                kind: None,
                currency: Some(self.currency.clone()),
                account_hint: None,
                reported_balance: Some(ReportedBalance {
                    date: utc_date(self.balance_date)
                        .ok_or_else(|| invalid(&key, "balance date"))?,
                    amount: decimal(&self.balance).ok_or_else(|| invalid(&key, "balance"))?,
                }),
                rows,
            },
            pending,
        ))
    }
}

fn decimal(text: &str) -> Option<crate::Money> {
    let text = text.strip_prefix('+').unwrap_or(text);
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || b"-.".contains(&byte))
        .then_some(text)?
        .parse()
        .ok()
}

impl Warning {
    pub fn affects(&self, account: &Account) -> bool {
        self.conn_id
            .as_ref()
            .is_none_or(|conn| conn == &account.conn_id)
            && self.account_id.as_ref().is_none_or(|id| id == &account.id)
    }
    pub fn blocks(&self, account: &Account) -> bool {
        self.affects(account)
            && (self.code.starts_with("con.")
                || (self.code.starts_with("act.") && self.code != "act.missingdata"))
    }
}
