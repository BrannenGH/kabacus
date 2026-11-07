// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parse the initial chart used by New Books in each host.
use crate::{error::Error, service::invalid};
pub fn initial_book(currency: &str, account: &str) -> Result<kabacus_ledger::Book, Error> {
    if currency.is_empty() || !currency.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(invalid("currency", "Enter a currency symbol".into()));
    }
    if !account.starts_with("assets:") || account.contains(['\n', '\r', ';']) {
        return Err(invalid(
            "account",
            "Enter the first account under assets:".into(),
        ));
    }
    let main = std::path::Path::new("/new/main.journal");
    let journal=format!("commodity {currency}1.00\naccount {account}\naccount equity:opening\naccount income:earnings\naccount expenses:general\n");
    let source =
        kabacus_ledger::MemorySource(std::collections::BTreeMap::from([(main.into(), journal)]));
    kabacus_ledger::import(&source, main)
        .map(|outcome| outcome.book)
        .map_err(crate::import::import_error)
}
