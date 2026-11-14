// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Journal reading: supported features, amounts, and what is reported or
//! refused.

use super::*;
use crate::Side;

fn source(files: &[(&str, &str)]) -> MemorySource {
    MemorySource(
        files
            .iter()
            .map(|(name, text)| (PathBuf::from(format!("/books/{name}")), text.to_string()))
            .collect(),
    )
}

fn import_one(text: &str) -> Result<ImportOutcome, ImportError> {
    import(
        &source(&[("main.journal", text)]),
        Path::new("/books/main.journal"),
    )
}

const HEAD: &str = "commodity 1000.00 USD\ncommodity 1000.00 EUR\naccount assets:checking\naccount expenses:groceries\naccount income:salary\n\n";

fn stopped(result: Result<ImportOutcome, ImportError>) -> (usize, String) {
    match result {
        Err(ImportError::Stopped { line, message, .. }) => (line, message),
        other => panic!("expected a stop, got {other:?}"),
    }
}

#[test]
fn reads_transactions_postings_and_comments() {
    // E5: date, status, code, description with a pipe, comments and tags.
    let text = format!(
        "{HEAD}2026-01-02 ! (C1) Trader Joe's | weekly shop ; receipt kept, trip:march\n    ; second line\n    * expenses:groceries  45.17 USD  ; card:visa\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    let tx = &out.book.transactions[0];
    assert_eq!(tx.id, 1);
    assert_eq!(tx.status, Status::Pending);
    assert_eq!(tx.code, "C1");
    assert_eq!(tx.description, "Trader Joe's | weekly shop");
    assert_eq!(tx.comment, "receipt kept, trip:march\nsecond line");
    assert_eq!(tx.tags, vec![("trip".to_string(), "march".to_string())]);
    assert_eq!(tx.postings[0].status, Status::Cleared);
    assert_eq!(tx.postings[0].comment, "card:visa");
    assert_eq!(tx.postings[1].amount.money.to_string(), "-45.17");
    let counts = out.report.counts();
    assert_eq!(counts.get(&Noted::Tag), Some(&2));
}

#[test]
fn dates_take_three_separators() {
    // E1
    let text = format!(
        "{HEAD}2026/01/02 a\n    expenses:groceries  1.00 USD\n    assets:checking\n\n2026.1.3 b\n    expenses:groceries  1.00 USD\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    assert_eq!(out.book.transactions[0].date.to_string(), "2026-01-02");
    assert_eq!(out.book.transactions[1].date.to_string(), "2026-01-03");
}

#[test]
fn a_secondary_date_moves_into_the_comment() {
    let text = format!(
        "{HEAD}2026-01-05=2026-01-03 Corner Grocer  ; bank posted late\n    expenses:groceries  12.00 USD\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    let tx = &out.book.transactions[0];
    assert_eq!(tx.date.to_string(), "2026-01-05");
    assert_eq!(tx.comment, "bank posted late, date2:2026-01-03");
    assert_eq!(out.report.counts().get(&Noted::SecondaryDate), Some(&1));
}

#[test]
fn stops_on_what_would_misread_the_book() {
    // E2/E3, E16, E17, E19, E20, E34
    let cases = [
        (
            "01/02 no year\n    assets:checking  1.00 USD\n",
            "A date needs a year.",
        ),
        ("alias a=zz\n", "alias isn't supported."),
        ("apply account pre\n", "apply account pre isn't supported."),
        ("D 1000.00 USD\n", "D isn't supported."),
        ("decimal-mark ,\n", "decimal-mark isn't supported."),
        ("Y2020\n", "Y isn't supported."),
        ("year 2020\n", "year isn't supported."),
        (
            "not a journal at all\n",
            "This line isn't a transaction or a supported directive.",
        ),
        ("2026-02-30 bad date\n", "2026-02-30 isn't a real date."),
    ];
    for (body, message) in cases {
        let (line, got) = stopped(import_one(&format!("{HEAD}{body}")));
        assert_eq!(line, 7, "{body}");
        assert_eq!(got, message, "{body}");
    }
}

#[test]
fn ignores_and_reports_what_kabacus_doesnt_support() {
    // E14, E15, E21, E22, P lines
    let text = format!(
        "{HEAD}; a note\n# another\n* and another\ncomment\nanything at all\nend comment\npayee Acme\ntag trip\nP 2026-01-01 EUR 1.10 USD\n= expenses:groceries\n    (budget)  *-1\n~ monthly\n    expenses:groceries  10.00 USD\n    assets:checking\n\n2026-01-02 Shop\n    expenses:groceries  10.00 USD\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    assert_eq!(out.book.transactions.len(), 1);
    let counts = out.report.counts();
    assert_eq!(counts.get(&Noted::Comment), Some(&3));
    assert_eq!(counts.get(&Noted::CommentBlock), Some(&1));
    assert_eq!(counts.get(&Noted::PayeeDirective), Some(&1));
    assert_eq!(counts.get(&Noted::TagDirective), Some(&1));
    assert_eq!(counts.get(&Noted::PriceLine), Some(&1));
    assert_eq!(counts.get(&Noted::AutomatedPostings), Some(&1));
    assert_eq!(counts.get(&Noted::PeriodicTransaction), Some(&1));
}

#[test]
fn includes_read_in_place_relative_to_the_including_file() {
    // E9, E13, E11, E12
    let src = source(&[
        (
            "main.journal",
            "commodity 1000.00 USD\ninclude accounts.journal\ninclude years/*.journal\n",
        ),
        (
            "accounts.journal",
            "account assets:checking\naccount income:salary\n",
        ),
        (
            "years/2025.journal",
            "2025-12-31 Pay\n    assets:checking  100.00 USD\n    income:salary\n",
        ),
        (
            "years/2026.journal",
            "2026-01-31 Pay\n    assets:checking  100.00 USD\n    income:salary\n",
        ),
    ]);
    let main = Path::new("/books/main.journal");
    let out = import(&src, main).unwrap();
    assert_eq!(out.book.transactions.len(), 2);
    assert_eq!(out.origins[&1], ("years/2025.journal".to_string(), 1));
    let names: Vec<String> = files(&src, main)
        .unwrap()
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "/books/main.journal",
            "/books/accounts.journal",
            "/books/years/2025.journal",
            "/books/years/2026.journal"
        ]
    );

    let (line, message) = stopped(import_one("include nope.journal\n"));
    assert_eq!(
        (line, message.as_str()),
        (1, "No file matches nope.journal.")
    );
    let cyclic = source(&[
        ("main.journal", "include b.journal\n"),
        ("b.journal", "include main.journal\n"),
    ]);
    assert!(matches!(
        import(&cyclic, main),
        Err(ImportError::Stopped { ref message, .. }) if message.contains("cycle")
    ));
}

#[test]
fn account_names_end_at_two_spaces_or_a_tab() {
    // E23, and the account directive's comment kept as its tags.
    let out = import_one(
        "account x ; k:v\naccount assets:checking  ; ynab-account:Checking\naccount income:salary\t; note\n    ; type:R\n",
    )
    .unwrap();
    let names: Vec<&str> = out.book.accounts.keys().map(String::as_str).collect();
    assert_eq!(names, ["assets:checking", "income:salary", "x ; k:v"]);
    assert_eq!(
        out.book.accounts["assets:checking"].tags,
        "ynab-account:Checking"
    );
    assert_eq!(out.book.accounts["income:salary"].tags, "note\ntype:R");
}

#[test]
fn commodity_directives_set_styles_and_settle_marks_wherever_they_are() {
    // A1, D1, D4, F1, F2, F3, F4, D5
    let out = import_one(
        "account a\naccount b\n\n2026-01-02 s\n    a  $1,234\n    b\n\n2026-01-03 t\n    a  1,234 EUR\n    b\n\ncommodity $1000.00\ncommodity $1,000.00\ncommodity 1000,00 EUR\ncommodity GBP\n    format GBP 1,000.00\n",
    )
    .unwrap();
    let a = &out.book.transactions[0].postings[0].amount;
    assert_eq!(
        (a.money.mantissa().to_string(), a.money.places()),
        ("1234".into(), 0)
    );
    let e = &out.book.transactions[1].postings[0].amount;
    assert_eq!(
        (e.money.mantissa().to_string(), e.money.places()),
        ("1234".into(), 3)
    );
    let dollar = &out.book.commodities["$"].style;
    assert_eq!((dollar.group_mark, dollar.places), (Some(','), Some(2)));
    let pound = &out.book.commodities["GBP"].style;
    assert_eq!((pound.side, pound.group_mark), (Side::Left, Some(',')));

    let (line, message) = stopped(import_one("commodity $1000\n"));
    assert_eq!(line, 1);
    assert!(message.contains("needs a decimal mark"), "{message}");
}

#[test]
fn a_bare_commodity_takes_its_style_from_use() {
    // F3, E4
    let out = import_one(
        "commodity $\naccount a\naccount b\n\n2026-01-02 s\n    a  $1.5\n    b  $ -1.50\n",
    )
    .unwrap();
    let style = &out.book.commodities["$"].style;
    assert_eq!(
        (style.side, style.spaced, style.places),
        (Side::Left, false, None)
    );
}

#[test]
fn more_places_than_the_directive_raises_the_commoditys_places() {
    // F7, F9
    let text =
        format!("{HEAD}2026-01-02 Fuel\n    expenses:groceries  1.001 USD\n    assets:checking\n");
    let out = import_one(&text).unwrap();
    assert_eq!(out.book.commodities["USD"].style.places, Some(3));
    let raised: Vec<&ReportEntry> = out
        .report
        .entries
        .iter()
        .filter(|e| e.noted == Noted::PlacesRaised)
        .collect();
    assert_eq!(raised.len(), 1);
    assert_eq!(
        raised[0].detail,
        "USD now shows 3 decimal places because of 1.001 USD on expenses:groceries."
    );
    assert_eq!(raised[0].line, 7);
}

#[test]
fn exponents_huge_numbers_and_the_255_place_limit() {
    let text = format!(
        "{HEAD}2026-01-02 Big\n    expenses:groceries  1e3 USD\n    expenses:groceries  123456789012345678901234567890.12 USD\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    let ps = &out.book.transactions[0].postings;
    assert_eq!(ps[0].amount.money.to_string(), "1000");
    assert_eq!(
        ps[2].amount.money.to_string(),
        "-123456789012345678901234568890.12"
    );
    let too_many = format!(
        "{HEAD}2026-01-02 Tiny\n    expenses:groceries  0.{}1 USD\n    assets:checking\n",
        "0".repeat(255)
    );
    let (line, message) = stopped(import_one(&too_many));
    assert_eq!(line, 8);
    assert!(
        message.contains("more than 255 decimal places"),
        "{message}"
    );
}

#[test]
fn costs_and_several_commodities_in_one_posting() {
    // E18, E27
    let text = format!(
        "{HEAD}2026-01-02 Exchange\n    expenses:groceries  10.00 EUR @ 1.10 USD\n    assets:checking\n\n2026-01-03 Mixed\n    expenses:groceries  10.00 USD\n    expenses:groceries  5.00 EUR\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    let fx = &out.book.transactions[0];
    assert_eq!(fx.postings[0].cost.as_ref().unwrap().kind, CostKind::Unit);
    assert_eq!(fx.postings[1].amount.to_string_for_test(), "-11 USD");
    let mixed = &out.book.transactions[1];
    assert_eq!(mixed.postings.len(), 4);
    assert_eq!(mixed.postings[2].amount.commodity, "EUR");
    assert_eq!(mixed.postings[3].amount.commodity, "USD");
}

trait ForTest {
    fn to_string_for_test(&self) -> String;
}

impl ForTest for Amount {
    fn to_string_for_test(&self) -> String {
        format!("{} {}", self.money.normalized(), self.commodity)
    }
}

#[test]
fn a_failing_assertion_is_skipped_and_reported() {
    let text = format!(
        "{HEAD}2026-01-02 Pay\n    assets:checking  100.00 USD = 90.00 USD\n    income:salary\n"
    );
    let out = import_one(&text).unwrap();
    assert!(out.book.transactions[0].postings[0]
        .balance_assertion
        .is_none());
    let skipped: Vec<&ReportEntry> = out
        .report
        .entries
        .iter()
        .filter(|e| e.noted == Noted::AssertionSkipped)
        .collect();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].line, 8);
}

#[test]
fn a_book_that_fails_the_strict_check_is_refused() {
    let text = format!(
        "{HEAD}2026-01-02 Shop\n    expenses:zzz  10.00 USD\n    assets:checking  -9.00 USD\n\n2026-01-03 Shop\n    expenses:yyy  1.00 GBP\n    assets:checking\n"
    );
    let failures = match import_one(&text) {
        Err(ImportError::Invalid(failures)) => failures,
        other => panic!("{other:?}"),
    };
    let kinds: Vec<FailureKind> = failures.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        [
            FailureKind::Balanced,
            FailureKind::Commodity,
            FailureKind::Account
        ]
    );
    assert_eq!(
        failures[0].message(),
        "The transaction on 2026-01-02 doesn't balance: 1.00 USD is left over. (main.journal:7)"
    );
    assert_eq!(
        failures[2].message(),
        "The account expenses:yyy hasn't been declared. (main.journal:12)"
    );
}

#[test]
fn an_assertion_without_an_amount_is_refused() {
    let text =
        format!("{HEAD}2026-01-02 Pay\n    assets:checking  = 90.00 USD\n    income:salary\n");
    let failures = match import_one(&text) {
        Err(ImportError::Invalid(failures)) => failures,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        failures[0].message(),
        "The posting on assets:checking has an assertion but no amount. (main.journal:8)"
    );
}

#[test]
fn empty_and_one_posting_transactions_are_read() {
    // E33; a lone posting without an amount has nothing to take one from.
    let text = format!(
        "{HEAD}2026-01-02\n    assets:checking  0.00 USD\n\n2026-01-03 Nothing\n\n2026-01-04 Alone\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    assert_eq!(out.book.transactions.len(), 3);
    assert_eq!(out.book.transactions[0].description, "");
    assert!(out.book.transactions[1].postings.is_empty());
    assert!(out.book.transactions[2].postings.is_empty());
    assert_eq!(
        out.report.counts().get(&Noted::PostingWithoutAmount),
        Some(&1)
    );
}

#[test]
fn transactions_get_ids_in_read_order() {
    let text = format!(
        "{HEAD}2026-01-05 Later\n    expenses:groceries  1.00 USD\n    assets:checking\n\n2026-01-04 Earlier\n    expenses:groceries  1.00 USD\n    assets:checking\n"
    );
    let out = import_one(&text).unwrap();
    let ids: Vec<(u64, &str)> = out
        .book
        .transactions
        .iter()
        .map(|t| (t.id, t.description.as_str()))
        .collect();
    assert_eq!(ids, [(1, "Later"), (2, "Earlier")]);
}

#[test]
fn crlf_journals_read_the_same() {
    let text =
        format!("{HEAD}2026-01-02 Shop\n    expenses:groceries  1.00 USD\n    assets:checking\n")
            .replace('\n', "\r\n");
    let out = import_one(&text).unwrap();
    assert_eq!(out.book.transactions[0].description, "Shop");
}
