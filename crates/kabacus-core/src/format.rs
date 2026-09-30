// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Formatted money strings for display (`"$1,234.56"`), always
//! alongside the exact `Amount` itself — this module never rounds to a
//! float, it just adds thousands separators and a currency symbol to the
//! exact digits already in a [`Money`].

use crate::money::{Amount, Money};
use num_bigint::BigUint;

/// A handful of well-known commodity symbols. Anything else falls back to
/// `<code> <amount>` (e.g. `"JPY 1,234"`).
fn symbol_for(commodity: &str) -> Option<&'static str> {
    match commodity {
        "USD" | "$" => Some("$"),
        "EUR" | "€" => Some("€"),
        "GBP" | "£" => Some("£"),
        _ => None,
    }
}

/// Format with digit grouping and, when known, a currency symbol. Negative
/// amounts are shown with a leading `-` before the symbol, e.g. `"-$45.00"`.
pub fn format_money(amount: &Amount) -> String {
    format_grouped(
        amount.money.clone(),
        symbol_for(&amount.commodity),
        &amount.commodity,
    )
}

fn format_grouped(money: Money, symbol: Option<&str>, code: &str) -> String {
    let negative = money.is_negative();
    let (whole, frac) = money.to_parts();

    let grouped = group_digits(&whole);

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    match symbol {
        Some(s) => out.push_str(s),
        None => {
            out.push_str(code);
            out.push(' ');
        }
    }
    out.push_str(&grouped);
    if money.places() > 0 {
        out.push('.');
        out.push_str(&format!("{frac:0width$}", width = money.places() as usize));
    }
    out
}

fn group_digits(n: &BigUint) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_usd_with_grouping() {
        let a = Amount::new("USD", Money::from_decimal(123456, 2));
        assert_eq!(format_money(&a), "$1,234.56");
    }
    #[test]
    fn formats_negative_amounts() {
        let a = Amount::new("USD", Money::from_decimal(-4500, 2));
        assert_eq!(format_money(&a), "-$45.00");
    }
    #[test]
    fn formats_negative_amounts_under_one() {
        let a = Amount::new("USD", Money::from_decimal(-1, 2));
        assert_eq!(format_money(&a), "-$0.01");
    }
    #[test]
    fn formats_small_amounts_without_grouping() {
        let a = Amount::new("USD", Money::from_decimal(99, 2));
        assert_eq!(format_money(&a), "$0.99");
    }
    #[test]
    fn unknown_commodity_falls_back_to_code() {
        let a = Amount::new("JPY", Money::from_decimal(1234, 0));
        assert_eq!(format_money(&a), "JPY 1,234");
    }
    #[test]
    fn three_decimal_places_preserved() {
        let a = Amount::new("USD", Money::from_decimal(45678, 3));
        assert_eq!(format_money(&a), "$45.678");
    }
}
