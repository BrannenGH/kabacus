// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Statement parsing and planning shared by local and server books.

mod csv;
mod ofx;
mod planner;

use crate::{AccountKind, Money};
use chrono::NaiveDate;
use encoding_rs::WINDOWS_1252;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::Write;
use thiserror::Error;

pub use csv::{inspect_csv, parse_csv, CsvInspection, CsvMapping};
pub use ofx::parse_ofx;
pub use planner::{import_tags, plan_rows, Action, Decision, Outcome, PlanInput, PlannedRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Ofx,
    Csv,
    Simplefin,
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ofx => "ofx",
            Self::Csv => "csv",
            Self::Simplefin => "simplefin",
        }
    }

    pub fn import_id(self, external_id: &str) -> String {
        format!("{}:{}", self.name(), encode_id(external_id))
    }
}

pub fn encode_id(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to a String");
        }
    }
    encoded
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub import_id: String,
    pub date: NaiveDate,
    pub amount: Money,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedBalance {
    pub date: NaiveDate,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub source: SourceKind,
    pub kind: Option<AccountKind>,
    pub currency: Option<String>,
    pub account_hint: Option<String>,
    pub rows: Vec<Row>,
    pub reported_balance: Option<ReportedBalance>,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("The statement exceeds the import limit")]
    TooLarge,
    #[error("The statement does not contain a supported bank or credit card statement")]
    NoStatement,
    #[error("The OFX document could not be read")]
    Ofx,
    #[error("The CSV could not be read at line {line}")]
    Csv { line: u64 },
    #[error("Choose the date format for this statement")]
    DateFormat,
    #[error("Choose a date, description, and either an amount or debit and credit columns")]
    CsvMapping,
    #[error("Invalid {field} at {location}")]
    Field { location: String, field: String },
    #[error("Import into a bank or credit card account: {0}")]
    AccountKind(String),
    #[error("The statement currency {reported} differs from the books' currency {commodity}")]
    CurrencyMismatch { reported: String, commodity: String },
    #[error("The import decision for {0} does not identify an eligible row or transaction")]
    Decision(String),
    #[error(transparent)]
    Funds(#[from] crate::FundsError),
}

pub fn clean_text(value: &str) -> String {
    let squeezed: String = value
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some(' ')
            } else if c.is_control() {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    squeezed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
        .replace(';', ",")
}

pub fn statement_description(payee: &str, memo: &str) -> String {
    let payee = clean_text(payee);
    let memo = clean_text(memo);
    if memo.is_empty() || (!payee.is_empty() && payee.to_lowercase().contains(&memo.to_lowercase()))
    {
        payee
    } else {
        format!("{payee} | {memo}")
    }
}

fn decode_text(bytes: &[u8]) -> Cow<'_, str> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => WINDOWS_1252.decode_without_bom_handling(bytes).0,
    }
}

#[cfg(test)]
mod tests;
