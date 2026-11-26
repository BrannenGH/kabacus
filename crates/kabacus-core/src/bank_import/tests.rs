// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::funds::{cash_on_hand, DEFAULT_RESERVE_TEMPLATE, UNCATEGORIZED};
use crate::{
    default_kind_rules, AccountName, Amount, Balances, Chart, Posting, PostingKind, Status,
    Transaction,
};
use proptest::prelude::*;
use std::collections::HashMap;

const CHECKING: &str = "assets:bank:checking";
const SAVINGS: &str = "assets:bank:savings";
const CARD: &str = "liabilities:credit-card:visa";
const RESERVE: &str = "assets:bank:checking:card-visa";

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
}

fn chart() -> Chart {
    Chart::from_books(
        default_kind_rules(),
        DEFAULT_RESERVE_TEMPLATE.into(),
        [CHECKING, SAVINGS, CARD, RESERVE, UNCATEGORIZED].map(AccountName::new),
        &[],
        &HashMap::new(),
    )
}

fn row(id: &str, day: u32, cents: i128) -> Row {
    Row {
        import_id: SourceKind::Ofx.import_id(id),
        date: date(day),
        amount: Money::from_decimal(cents, 2),
        description: "Harbor Street Cafe".into(),
    }
}

fn existing(day: u32, id: u64, target: &str, cents: i128) -> Transaction {
    transaction(
        date(day),
        id,
        "Harbor Street Cafe | Lunch",
        vec![
            (AccountName::new(target), Money::from_decimal(cents, 2)),
            (
                AccountName::new("expenses:dining"),
                Money::from_decimal(-cents, 2),
            ),
        ],
    )
}

fn transaction(
    date: NaiveDate,
    id: u64,
    description: &str,
    postings: Vec<(AccountName, Money)>,
) -> Transaction {
    Transaction {
        id,
        date,
        status: Status::Unmarked,
        code: String::new(),
        description: description.into(),
        comment: String::new(),
        tags: vec![],
        postings: postings
            .into_iter()
            .map(|(account, money)| Posting {
                account,
                amount: Amount::new("USD", money),
                status: Status::Unmarked,
                kind: PostingKind::Real,
                comment: String::new(),
                tags: vec![],
                cost: None,
                balance_assertion: None,
            })
            .collect(),
    }
}

fn plan(
    rows: Vec<Row>,
    target: &str,
    transactions: &[Transaction],
    decisions: &[Decision],
) -> Vec<PlannedRow> {
    plan_rows(
        PlanInput {
            chart: &chart(),
            transactions,
            target,
            commodity: "USD",
            source: SourceKind::Ofx,
            simplefin_feed: None,
            since: None,
            decisions,
        },
        rows,
    )
    .unwrap()
}

fn apply(rows: Vec<PlannedRow>, target: &str, transactions: &mut Vec<Transaction>) {
    for PlannedRow { row, outcome, .. } in rows {
        let tx = match outcome {
            Outcome::New(planned) => {
                transactions.push(transaction(
                    row.date,
                    transactions.len() as u64 + 100,
                    &planned.description,
                    planned.postings,
                ));
                transactions.last_mut().unwrap()
            }
            Outcome::Matched(id) => transactions.iter_mut().find(|tx| tx.id == id).unwrap(),
            _ => continue,
        };
        tx.status = Status::Cleared;
        tx.tags.extend(import_tags(target, &row.import_id));
    }
}

#[test]
fn matching_orders_candidates_and_preserves_the_categorized_transaction() {
    let mut txs = vec![
        existing(12, 34, CHECKING, -1840),
        existing(15, 31, CHECKING, -1840),
        existing(15, 25, CHECKING, -1840),
        existing(19, 38, CHECKING, -1840),
    ];
    let input = row("Café 04/18, 26", 16, -1840);
    assert_eq!(input.import_id, "ofx:Caf%C3%A9%2004%2F18%2C%2026");
    let planned = plan(
        vec![input.clone(), row("D-261016", 16, -1840)],
        CHECKING,
        &txs,
        &[],
    );
    assert_eq!(planned[0].candidates, [25, 31, 34]);
    assert!(matches!(planned[0].outcome, Outcome::Matched(25)));
    assert!(matches!(planned[1].outcome, Outcome::Matched(31)));
    let old = txs.clone();
    apply(planned, CHECKING, &mut txs);
    assert_eq!(txs[2].postings, old[2].postings);
    assert_eq!(txs[2].description, old[2].description);
    assert_eq!(txs[2].date, old[2].date);
    assert_eq!(txs[2].status, Status::Cleared);
    assert_eq!(
        txs[2].tags[0],
        ("import-id".into(), input.import_id.clone())
    );
    assert!(matches!(
        plan(vec![input], CHECKING, &txs, &[])[0].outcome,
        Outcome::Duplicate
    ));
}

#[test]
fn ids_are_scoped_to_the_feed_account_even_with_multiple_postings() {
    let input = row("018292", 17, -6200);
    let mut txs = vec![];
    apply(plan(vec![input.clone()], CARD, &txs, &[]), CARD, &mut txs);
    assert!(matches!(
        plan(vec![input.clone()], CHECKING, &txs, &[])[0].outcome,
        Outcome::New(_)
    ));
    assert!(matches!(
        plan(vec![input], CARD, &txs, &[])[0].outcome,
        Outcome::Duplicate
    ));
}

#[test]
fn imported_descriptions_keep_the_journal_separator_unambiguous() {
    let text = statement_description("Oak|Harbor\tCafe; order", "Refund | | order 1846");
    assert_eq!(text, "Oak/Harbor Cafe, order | Refund / / order 1846");
    assert!(crate::check_description(&text).is_ok());
}

#[test]
fn review_can_choose_a_different_candidate_or_skip_or_add() {
    let txs = vec![
        existing(15, 31, CHECKING, -1840),
        existing(14, 32, CHECKING, -1840),
    ];
    let inputs = vec![
        row("D-261016", 16, -1840),
        row("D-261017", 17, -1840),
        row("D-261018", 18, -1840),
    ];
    let decisions = [
        Decision {
            import_id: inputs[0].import_id.clone(),
            action: Action::Match { transaction: 32 },
        },
        Decision {
            import_id: inputs[1].import_id.clone(),
            action: Action::Skip,
        },
        Decision {
            import_id: inputs[2].import_id.clone(),
            action: Action::Add,
        },
    ];
    let planned = plan(inputs.clone(), CHECKING, &txs, &decisions);
    assert!(matches!(planned[0].outcome, Outcome::Matched(32)));
    assert!(matches!(planned[1].outcome, Outcome::Skipped));
    assert!(matches!(planned[2].outcome, Outcome::New(_)));
    let invalid = Decision {
        action: Action::Match { transaction: 98 },
        ..decisions[0].clone()
    };
    assert!(plan_rows(
        PlanInput {
            chart: &chart(),
            transactions: &txs,
            target: CHECKING,
            commodity: "USD",
            source: SourceKind::Ofx,
            simplefin_feed: None,
            since: None,
            decisions: &[invalid]
        },
        inputs
    )
    .is_err());
}

proptest! {
    #[test]
    fn overlapping_imports_are_idempotent_and_preserve_balances_and_reserves(
        cents in prop::collection::vec((-10_000i32..10_000).prop_filter("nonzero", |c| *c != 0), 3..20),
        card in any::<bool>(),
    ) {
        let target = if card { CARD } else { CHECKING };
        let rows: Vec<_> = cents.iter().enumerate().map(|(i, &cents)| row(&format!("HBR-{i:08}"), i as u32 + 1, cents.into())).collect();
        let middle = rows.len() / 2;
        let a = &rows[..=middle];
        let b = &rows[middle..];
        let mut union = vec![];
        apply(plan(rows.clone(), target, &union, &[]), target, &mut union);
        let fingerprint = |txs: &[Transaction]| {
            let mut items: Vec<_> = txs.iter().map(|tx| (tx.date, tx.postings.clone(), tx.tags.clone())).collect();
            items.sort_by_key(|item| item.0);
            items
        };
        for (first, second) in [(a, b), (b, a)] {
            let mut txs = vec![];
            for subset in [first, second] {
                apply(plan(subset.to_vec(), target, &txs, &[]), target, &mut txs);
            }
            prop_assert_eq!(fingerprint(&txs), fingerprint(&union));
            prop_assert!(plan(rows.clone(), target, &txs, &[]).iter().all(|p| matches!(p.outcome, Outcome::Duplicate)));
        }
        for tx in &union {
            prop_assert!(tx.postings.iter().map(|p| p.amount.money.clone()).sum::<Money>().is_zero());
        }
        let balances = Balances::compute(&union, "USD", None);
        let expected = rows.iter().map(|row| row.amount.clone()).sum::<Money>();
        prop_assert_eq!(balances.exclusive(&AccountName::new(target)).money, expected.clone());
        if card {
            prop_assert_eq!(balances.exclusive(&AccountName::new(RESERVE)).money, -expected);
        }
        let report = cash_on_hand(&chart(), &union, "USD", None);
        prop_assert_eq!(report.cash_on_hand.money, report.money_on_hand.money - report.future_allocations.money - report.receivables.money);
    }
}

#[test]
fn reads_both_statement_formats_and_masks_account_numbers() {
    let input = b"OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\nCHARSET:1252\r\n\r\n<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\n\
        <CURDEF>USD<BANKACCTFROM><BANKID>12345678<ACCTID>3156042<ACCTTYPE>CHECKING</BANKACCTFROM>\n\
        <BANKTRANLIST><DTSTART>20261001<DTEND>20261031<STMTTRN><DTPOSTED>20261014103000.000[-5:EST]<TRNAMT>-62.18<FITID>20261014-0031<NAME>Corner   Market<MEMO>Corner Market</STMTTRN></BANKTRANLIST>\n\
        <LEDGERBAL><BALAMT>2413.07<DTASOF>20261031</LEDGERBAL></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
    let statements = parse_ofx(input, 2, 5000).unwrap();
    let bank = &statements[0];
    assert_eq!(bank.kind, Some(AccountKind::Money));
    assert_eq!(bank.account_hint.as_deref(), Some("…6042"));
    assert_eq!(bank.currency.as_deref(), Some("USD"));
    assert_eq!(bank.rows[0].date, date(14));
    assert_eq!(bank.rows[0].description, "Corner Market");
    assert_eq!(
        bank.reported_balance.as_ref().unwrap().amount.to_string(),
        "2413.07"
    );
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?><?OFX OFXHEADER="200" VERSION="220"?>
        <OFX><CREDITCARDMSGSRSV1><CCSTMTTRNRS><CCSTMTRS><CURDEF>USD</CURDEF>
        <CCACCTFROM><ACCTID>43911867</ACCTID></CCACCTFROM><BANKTRANLIST>
        <STMTTRN><DTPOSTED>20261016010000[+11:AET]</DTPOSTED><TRNAMT>18.40</TRNAMT>
        <FITID>Harbor &amp; 46</FITID><PAYEE><NAME>Harbor &amp; Hearth Cafe</NAME></PAYEE>
        <MEMO><![CDATA[Refund | order 1846]]></MEMO></STMTTRN>
        </BANKTRANLIST></CCSTMTRS></CCSTMTTRNRS></CREDITCARDMSGSRSV1></OFX>"#;
    let cards = parse_ofx(xml, 2, 5000).unwrap();
    assert_eq!(cards[0].kind, Some(AccountKind::RevolvingDebt));
    let row = &cards[0].rows[0];
    assert_eq!(row.date, date(16));
    assert_eq!(row.import_id, "ofx:Harbor%20%26%2046");
    assert_eq!(
        row.description,
        "Harbor & Hearth Cafe | Refund / order 1846"
    );
    assert_eq!(row.amount.to_string(), "18.40");
    assert!(matches!(parse_ofx(input, 2, 0), Err(ImportError::TooLarge)));
}

#[test]
fn csv_handles_windows_1252_bom_decimal_commas_and_repeated_rows() {
    let input = b"Date;Payee;Memo;Amount;Balance\r\n24/10/2026;Caf\xe9 du Port;Order 1642;(\x801.234,56);2413,07\r\n24/10/2026;Caf\xe9 du Port;Order 1642;1.234,56-;1178,51\r\n";
    let detected = inspect_csv(input, 0).unwrap();
    assert_eq!(detected.delimiter, ';');
    assert!(detected.header);
    let mut mapping = detected.mapping.unwrap();
    mapping.description = vec![1, 2];
    let statement = parse_csv(input, &mapping, 2, 5000).unwrap();
    assert_eq!(statement.rows[0].description, "Café du Port Order 1642");
    assert_eq!(statement.rows[0].amount.to_string(), "-1234.56");
    assert_eq!(statement.rows[1].amount, statement.rows[0].amount);
    assert_ne!(statement.rows[0].import_id, statement.rows[1].import_id);
    assert_eq!(
        statement.reported_balance.unwrap().amount.to_string(),
        "1178.51"
    );
    let bom = b"\xef\xbb\xbfDate,Description,Amount\n03/04/2026,Oak Street Bakery,-18.40\n";
    let mapping = inspect_csv(bom, 0).unwrap().mapping.unwrap();
    assert!(matches!(
        parse_csv(bom, &mapping, 2, 5000),
        Err(ImportError::DateFormat)
    ));
    let mapping = CsvMapping {
        date_format: Some("%d/%m/%Y".into()),
        ..mapping
    };
    assert_eq!(
        parse_csv(bom, &mapping, 2, 5000).unwrap().rows[0].date,
        NaiveDate::from_ymd_opt(2026, 4, 3).unwrap()
    );
}

fn ofx_transaction(posted: &str, amount: &str, details: &str) -> String {
    format!(
        "OFXHEADER:100\nDATA:OFXSGML\nVERSION:102\n\n        <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD<BANKTRANLIST>        <STMTTRN><DTPOSTED>{posted}<TRNAMT>{amount}<FITID>HBR-1846{details}</STMTTRN>        </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>"
    )
}

#[test]
fn ofx_xml_without_a_prolog_preserves_cdata() {
    let xml = br#"<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD</CURDEF>
        <BANKTRANLIST><STMTTRN><DTPOSTED>20261016</DTPOSTED><TRNAMT>-18.40</TRNAMT>
        <FITID>HBR-1846</FITID><NAME><![CDATA[Oak & Harbor Market]]></NAME></STMTTRN>
        </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>"#;
    let statements = parse_ofx(xml, 2, 5).unwrap();
    assert_eq!(statements[0].rows[0].description, "Oak & Harbor Market");
}

#[test]
fn ofx_sgml_accepts_empty_optional_transaction_fields() {
    let file = ofx_transaction("20261016", "-18.40", "<CHECKNUM><REFNUM><NAME>Oak Bakery");
    let (_, headerless) = file.split_once("<OFX>").unwrap();
    for input in [&file, &format!("<OFX>{headerless}")] {
        let statements = parse_ofx(input.as_bytes(), 2, 5).unwrap();
        assert_eq!(statements[0].rows[0].description, "Oak Bakery");
    }
}

#[test]
fn ofx_rejects_invalid_timestamp_suffixes() {
    for posted in ["20261016TBD", "20261016260100", "20261016130100[-5:EST"] {
        let file = ofx_transaction(posted, "-18.40", "<NAME>Oak Bakery");
        assert!(
            matches!(
                parse_ofx(file.as_bytes(), 2, 5),
                Err(ImportError::Field { field, .. }) if field == "DTPOSTED"
            ),
            "{posted}"
        );
    }
}

#[test]
fn bank_feeds_reject_amounts_with_both_signs() {
    let file = ofx_transaction("20261016", "+-18.40", "<NAME>Oak Bakery");
    let ofx = parse_ofx(file.as_bytes(), 2, 5);
    let mut account = simplefin_account("HBR-1846", 1_792_108_800);
    account.transactions[0].amount = "+-18.40".into();
    let simplefin = account.statement();
    assert!(matches!(ofx, Err(ImportError::Field { field, .. }) if field == "TRNAMT"));
    assert!(matches!(simplefin, Err(ImportError::Field { field, .. }) if field == "amount"));
    account.transactions[0].amount = "+18.40".into();
    assert_eq!(
        account.statement().unwrap().0.rows[0].amount.to_string(),
        "18.40"
    );
}

#[test]
fn csv_rejects_incomplete_mapping_even_without_data_rows() {
    let file = b"Date,Payee,Amount\n";
    let mut mapping = inspect_csv(file, 0).unwrap().mapping.unwrap();
    mapping.amount = None;
    assert!(matches!(
        parse_csv(file, &mapping, 2, 5),
        Err(ImportError::CsvMapping)
    ));
}

#[test]
fn csv_inspects_header_only_exports_with_recognized_aliases() {
    let file = b"Booking Date;Merchant;Net Amount\n";
    let inspection = inspect_csv(file, 0).unwrap();
    assert!(inspection.header);
    assert!(inspection.mapping.is_some());
    assert!(inspection.sample.is_empty());
}

#[test]
fn csv_prefers_a_delimiter_that_exposes_recognized_columns() {
    let file = b"Date\tPayee\tAmount\tCity, State, Country, Branch, Terminal\n        2026-10-16\tOak Bakery\t-18.40\tPortland, ME, US, Old Port, 12\n";
    let inspection = inspect_csv(file, 0).unwrap();
    assert_eq!(inspection.delimiter, '\t');
    let statement = parse_csv(file, &inspection.mapping.unwrap(), 2, 5).unwrap();
    assert_eq!(statement.rows.len(), 1);
}

#[test]
fn review_rejects_match_decisions_on_rows_that_cannot_be_matched() {
    let txs = [existing(15, 31, CHECKING, -1840)];
    let chart = chart();
    for (input, since) in [
        (row("HBR-1846", 16, 0), None),
        (row("HBR-1846", 16, -1840), Some(date(17))),
    ] {
        let decisions = [Decision {
            import_id: input.import_id.clone(),
            action: Action::Match { transaction: 31 },
        }];
        assert!(matches!(
            plan_rows(
                PlanInput {
                    chart: &chart,
                    transactions: &txs,
                    target: CHECKING,
                    commodity: "USD",
                    source: SourceKind::Ofx,
                    simplefin_feed: None,
                    since,
                    decisions: &decisions,
                },
                vec![input],
            ),
            Err(ImportError::Decision(_))
        ));
    }
}

fn simplefin_account(id: &str, posted: i64) -> simplefin::Account {
    simplefin::Account {
        id: "harbor-checking-6042".into(),
        conn_id: "harbor-credit-union".into(),
        name: "Harbor Checking".into(),
        currency: "USD".into(),
        balance: "2413.07".into(),
        balance_date: 1_792_108_800,
        transactions: vec![simplefin::BankTransaction {
            id: id.into(),
            posted,
            amount: "-18.40".into(),
            description: "Oak Bakery".into(),
            payee: None,
            pending: false,
        }],
    }
}

#[test]
fn simplefin_rejects_posted_rows_without_a_transaction_id() {
    let account = simplefin_account("", 1_792_108_800);
    assert!(matches!(
        account.statement(),
        Err(ImportError::Field { field, .. }) if field == "ID"
    ));
}

#[test]
fn import_refuses_conflicting_rows_with_the_same_id() {
    let first = row("HBR-1846", 16, -1840);
    let second = row("HBR-1846", 16, -1210);
    let result = plan_rows(
        PlanInput {
            chart: &chart(),
            transactions: &[],
            target: CHECKING,
            commodity: "USD",
            source: SourceKind::Ofx,
            simplefin_feed: None,
            since: None,
            decisions: &[],
        },
        vec![first.clone(), second],
    );
    assert!(matches!(result, Err(ImportError::Field { field, .. }) if field == "ID"));
    let decisions = [Decision {
        import_id: first.import_id.clone(),
        action: Action::Match { transaction: 31 },
    }];
    let repeated = plan(
        vec![first.clone(), first],
        CHECKING,
        &[existing(15, 31, CHECKING, -1840)],
        &decisions,
    );
    assert!(matches!(repeated[0].outcome, Outcome::Matched(31)));
    assert!(matches!(repeated[1].outcome, Outcome::Duplicate));
}
