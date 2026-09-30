// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Journal text for transactions and whole books, rendered from stored
//! values. Implements the engine implementation specification, sections
//! 2.2 (`render_transaction`, `render_book`) and 3.3, and the rendering
//! rules of "Ledger spec: amounts, commodity styles and precision".

use crate::amount::{render_amount, render_example, render_symbol};
use crate::book::{style_in, Book, Styles};
use kabacus_core::{Amount, CostKind, Posting, PostingKind, Status, Transaction};

fn status_mark(status: Status) -> &'static str {
    match status {
        Status::Unmarked => "",
        Status::Pending => "! ",
        Status::Cleared => "* ",
    }
}

pub fn render_money(amount: &Amount, styles: &Styles) -> String {
    render_amount(amount, &style_in(styles, &amount.commodity))
}

/// Append `comment`'s lines: the first on the current line, the rest
/// each on an indented line of its own.
fn push_comment(out: &mut String, comment: &str) {
    let mut lines = comment.split('\n');
    if let Some(first) = lines.next() {
        if !first.trim().is_empty() {
            out.push_str("  ; ");
            out.push_str(first.trim());
        }
    }
    for line in lines {
        out.push_str("\n    ;");
        if !line.trim().is_empty() {
            out.push(' ');
            out.push_str(line.trim());
        }
    }
}

/// One posting line (and its comment lines), indented.
pub fn render_posting(posting: &Posting, styles: &Styles) -> String {
    let mut line = String::from("    ");
    line.push_str(status_mark(posting.status));
    let account = posting.account.as_str();
    match posting.kind {
        PostingKind::Real => line.push_str(account),
        PostingKind::Virtual => line.push_str(&format!("({account})")),
        PostingKind::BalancedVirtual => line.push_str(&format!("[{account}]")),
    }
    line.push_str("  ");
    line.push_str(&render_money(&posting.amount, styles));
    if let Some(cost) = &posting.cost {
        line.push_str(match cost.kind {
            CostKind::Unit => " @ ",
            CostKind::Total => " @@ ",
        });
        line.push_str(&render_money(&cost.amount, styles));
    }
    if let Some(assertion) = &posting.balance_assertion {
        line.push(' ');
        line.push_str(assertion.operator());
        line.push(' ');
        line.push_str(&render_money(&assertion.amount, styles));
    }
    push_comment(&mut line, &posting.comment);
    line
}

/// A transaction as journal text, ending in a newline.
pub fn render_transaction(tx: &Transaction, styles: &Styles) -> String {
    let mut out = tx.date.format("%Y-%m-%d").to_string();
    match tx.status {
        Status::Unmarked => {}
        Status::Pending => out.push_str(" !"),
        Status::Cleared => out.push_str(" *"),
    }
    if !tx.code.is_empty() {
        out.push_str(&format!(" ({})", tx.code));
    }
    if !tx.description.is_empty() {
        out.push(' ');
        out.push_str(&tx.description);
    }
    push_comment(&mut out, &tx.comment);
    out.push('\n');
    for posting in &tx.postings {
        out.push_str(&render_posting(posting, styles));
        out.push('\n');
    }
    out
}

/// A whole book as one journal: commodity and account directives, then
/// every transaction in date order. For the comparison suite.
pub fn render_book(book: &Book) -> String {
    let styles = book.styles();
    let mut out = String::new();
    for commodity in book.commodities.values() {
        if commodity.style.places.is_some() {
            out.push_str(&format!(
                "commodity {}\n",
                render_example(&commodity.symbol, &commodity.style)
            ));
        } else if commodity.symbol.is_empty() {
            // The empty commodity can only be declared by example.
            out.push_str("commodity 1000.\n");
        } else {
            out.push_str(&format!("commodity {}\n", render_symbol(&commodity.symbol)));
        }
    }
    for account in book.accounts.values() {
        out.push_str("account ");
        out.push_str(account.name.as_str());
        push_comment(&mut out, &account.tags);
        out.push('\n');
    }
    let mut txs: Vec<&Transaction> = book.transactions.iter().collect();
    txs.sort_by_key(|t| (t.date, t.id));
    for tx in txs {
        out.push('\n');
        out.push_str(&render_transaction(tx, &styles));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::style_from_example;
    use chrono::NaiveDate;
    use kabacus_core::{AccountName, BalanceAssertion, Cost, Money};

    fn usd(m: i128) -> Amount {
        Amount::new("USD", Money::from_decimal(m, 2))
    }

    fn posting(account: &str, amount: Amount) -> Posting {
        Posting {
            account: AccountName::new(account),
            amount,
            status: Status::Unmarked,
            kind: PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: None,
        }
    }

    #[test]
    fn a_transaction_with_every_part() {
        let mut styles = Styles::new();
        styles.insert("USD".into(), style_from_example("1,000.00 USD").unwrap().1);
        let mut checking = posting("assets:checking", usd(-4500));
        checking.status = Status::Cleared;
        checking.balance_assertion = Some(BalanceAssertion {
            inclusive: true,
            ..BalanceAssertion::new(usd(95500))
        });
        checking.comment = "card:visa".into();
        let mut fx = posting(
            "expenses:travel",
            Amount::new("EUR", Money::from_decimal(10, 0)),
        );
        fx.cost = Some(Cost {
            kind: CostKind::Total,
            amount: usd(1100),
        });
        fx.kind = PostingKind::BalancedVirtual;
        let tx = Transaction {
            id: 3,
            date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            status: Status::Pending,
            code: "1042".into(),
            description: "Trader Joe's | weekly shop".into(),
            comment: "receipt in the drawer\nsecond line".into(),
            tags: vec![],
            postings: vec![posting("expenses:groceries", usd(4500)), checking, fx],
        };
        assert_eq!(
            render_transaction(&tx, &styles),
            "2026-03-02 ! (1042) Trader Joe's | weekly shop  ; receipt in the drawer\n    ; second line\n    expenses:groceries  45.00 USD\n    * assets:checking  -45.00 USD =* 955.00 USD  ; card:visa\n    [expenses:travel]  10 EUR @@ 11.00 USD\n"
        );
    }

    #[test]
    fn a_comment_without_an_inline_line_starts_on_its_own_line() {
        let tx = Transaction {
            id: 1,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: String::new(),
            comment: "\nnote".into(),
            tags: vec![],
            postings: vec![],
        };
        assert_eq!(
            render_transaction(&tx, &Styles::new()),
            "2026-01-01\n    ; note\n"
        );
    }
}
