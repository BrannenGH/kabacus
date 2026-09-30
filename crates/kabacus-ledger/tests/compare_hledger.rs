// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine against the `hledger` binary, run as a black box on our own
//! synthetic journals: the same verdict from the strict check, the same
//! first kind of failure, the same transactions read, and journals the
//! engine renders accepted by hledger with the same values. The
//! deliberate differences (engine implementation specification, section
//! 3.1) are left out of the generated books or asserted as differences.
//!
//! hledger is `$KABACUS_HLEDGER` or `hledger` on the `PATH`. Without it the
//! suite is skipped, unless `KABACUS_REQUIRE_HLEDGER` is set (as in CI).

use kabacus_core::json::parse_transactions;
use kabacus_core::{Amount, PostingKind, Transaction};
use kabacus_ledger::{read, render_book, FailureKind, FsSource};
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hledger() -> Option<String> {
    let binary = std::env::var("KABACUS_HLEDGER").unwrap_or_else(|_| "hledger".to_string());
    let found = Command::new(&binary)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if found {
        return Some(binary);
    }
    if std::env::var_os("KABACUS_REQUIRE_HLEDGER").is_some() {
        panic!("hledger ({binary}) is required for the comparison suite but can't be run");
    }
    eprintln!("hledger not found: skipping the comparison with it");
    None
}

struct Hledger {
    binary: String,
}

impl Hledger {
    fn run(&self, file: &Path, args: &[&str]) -> std::process::Output {
        Command::new(&self.binary)
            .arg("-f")
            .arg(file)
            .args(args)
            .output()
            .expect("hledger runs")
    }

    /// The first failure's kind, or none when `check -s` passes.
    fn verdict(&self, file: &Path) -> Option<FailureKind> {
        let out = self.run(file, &["check", "-s"]);
        if out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stderr);
        Some(
            if text.contains("has not been declared") && text.contains("account \"") {
                FailureKind::Account
            } else if text.contains("has not been declared") && text.contains("commodity \"") {
                FailureKind::Commodity
            } else if text.contains("Balance assertion failed") {
                FailureKind::Assertion
            } else if text.contains("unbalanced")
                || text.contains("sum should be 0")
                || text.contains("no amount")
            {
                FailureKind::Balanced
            } else {
                FailureKind::Readable
            },
        )
    }

    /// The transactions hledger reads. Its JSON writes a mantissa too big
    /// for a double as a float, so then its own `print` is read instead.
    fn print(&self, file: &Path) -> Vec<Transaction> {
        let out = self.run(file, &["print", "-O", "json"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if let Ok(txs) = parse_transactions(&String::from_utf8_lossy(&out.stdout)) {
            return txs;
        }
        let printed = self.run(file, &["print"]);
        let dir = tempfile::tempdir().unwrap();
        let declarations: String = std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .filter(|l| l.starts_with("commodity") || l.starts_with("account"))
            .map(|l| format!("{l}\n"))
            .collect();
        let text = format!(
            "{declarations}\n{}",
            String::from_utf8_lossy(&printed.stdout)
        );
        let path = write(dir.path(), "printed.journal", &text);
        read(&FsSource, &path)
            .expect("hledger's print")
            .book
            .transactions
    }
}

/// hledger's JSON gives quantities to at most 10 decimal places (its
/// `print` shows them all), so values are compared to 10 places.
fn ten_places(m: &kabacus_core::Money) -> kabacus_core::Money {
    if m.places() <= 10 {
        return m.normalized();
    }
    let scale = num_bigint::BigInt::from(10).pow(m.places() - 10);
    kabacus_core::Money::from_big(m.mantissa() / scale, 10).normalized()
}

/// What both readers must agree on, values only.
fn shape(txs: &[Transaction]) -> Vec<String> {
    let amount = |a: &Amount| format!("{} {:?}", ten_places(&a.money), a.commodity);
    let mut out: Vec<String> = txs
        .iter()
        .map(|t| {
            let postings: Vec<String> = t
                .postings
                .iter()
                .map(|p| {
                    format!(
                        "{:?} {:?} {:?} {} cost={} assert={}",
                        p.status,
                        p.kind,
                        p.account.as_str(),
                        amount(&p.amount),
                        p.cost
                            .as_ref()
                            .map(|c| format!("{:?} {}", c.kind, amount(&c.amount)))
                            .unwrap_or_default(),
                        p.balance_assertion
                            .as_ref()
                            .map(|b| format!("{} {}", b.operator(), amount(&b.amount)))
                            .unwrap_or_default(),
                    )
                })
                .collect();
            format!(
                "{} {:?} {:?} {:?} [{}]",
                t.date,
                t.status,
                t.code,
                t.description,
                postings.join("; ")
            )
        })
        .collect();
    out.sort();
    out
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Compare everything about one journal; `differs` names a declared
/// difference in the verdict.
fn compare(h: &Hledger, text: &str) {
    let dir = tempfile::tempdir().unwrap();
    let main = write(dir.path(), "main.journal", text);
    let theirs = h.verdict(&main);
    let ours = match read(&FsSource, &main) {
        Ok(read) => read.check().first().map(|f| f.kind),
        Err(e) => e.failures().first().map(|f| f.kind),
    };
    assert_eq!(ours, theirs, "first failure differs for\n{text}");
    if theirs.is_some() {
        return;
    }
    let book = read(&FsSource, &main).unwrap().book;
    assert_eq!(
        shape(&book.transactions),
        shape(&h.print(&main)),
        "read differently:\n{text}"
    );
    let rendered = render_book(&book);
    let again = write(dir.path(), "rendered.journal", &rendered);
    assert_eq!(
        h.verdict(&again),
        None,
        "hledger refuses what the engine wrote:\n{rendered}"
    );
    assert_eq!(
        shape(&h.print(&again)),
        shape(&book.transactions),
        "hledger reads what the engine wrote differently:\n{rendered}"
    );
}

const HEAD: &str = "commodity $1,000.00\ncommodity 1.000,00 EUR\ncommodity 1000.00 CAD\naccount assets:bank:checking\naccount assets:bank:checking:emergency\naccount assets:bank:savings\naccount liabilities:credit-card:visa\naccount expenses:food:groceries\naccount expenses:transport:fuel\naccount expenses:travel\naccount income:salary\naccount equity:opening\n\n";

#[test]
fn the_specification_examples_agree() {
    let Some(binary) = hledger() else { return };
    let h = Hledger { binary };
    let cases = [
        // Balancing (the balancing page).
        "2026-01-02 Trader Joe's\n    expenses:food:groceries  $45.17\n    assets:bank:checking\n",
        "2026-01-02 Split\n    expenses:food:groceries  $10\n    expenses:travel  5,00 EUR\n    assets:bank:checking\n",
        "2026-01-02 Off by a cent\n    expenses:food:groceries  $1.001\n    assets:bank:checking  $-1.00\n",
        "2026-01-02 Wrong commodity\n    expenses:food:groceries  $5\n    assets:bank:checking  -5,00 EUR\n",
        "2026-01-02 Two omitted\n    expenses:food:groceries  $5\n    assets:bank:checking\n    assets:bank:savings\n",
        "2026-01-02 Virtual\n    expenses:food:groceries  $10\n    assets:bank:checking  $-10\n    [assets:bank:checking:emergency]  $5\n    [assets:bank:savings]\n",
        "2026-01-02 Virtual off\n    expenses:food:groceries  $10\n    assets:bank:checking  $-10\n    [assets:bank:checking:emergency]  $5\n    [assets:bank:savings]  $-4\n",
        "2026-01-02 Exchange\n    expenses:travel  10,00 EUR @ $1.10\n    assets:bank:checking\n",
        "2026-01-02 Exchange total\n    expenses:travel  10,00 EUR @@ $11.00\n    assets:bank:checking\n",
        "2026-01-02 Tiny and huge\n    expenses:food:groceries  $1000000000000000000\n    assets:bank:checking  $-1000000000000000000\n    expenses:food:groceries  $0.000000000000000001\n    assets:bank:checking  $-0.000000000000000001\n",
        // Strict checks.
        "2026-01-02 Shop\n    expenses:food:snacks  $10\n    assets:bank:checking\n",
        "2026-01-02 Shop\n    expenses:food:groceries  10.00 GBP\n    assets:bank:checking\n",
        "2026-01-02 Shop\n    expenses:food:snacks  10.00 GBP\n    assets:bank:checking\n",
        "2026-02-30 Bad date\n    expenses:food:groceries  $10\n    assets:bank:checking\n",
        "Payday\n",
        // Assertions without same-day order dependence.
        "2026-01-01 Opening\n    assets:bank:checking  $1,000.00\n    equity:opening\n\n2026-01-05 Check\n    assets:bank:checking  $0 = $1,000.00\n",
        "2026-01-01 Opening\n    assets:bank:checking  $1,000.00\n    equity:opening\n\n2026-01-05 Check\n    assets:bank:checking  $0 = $999.00\n",
        "2026-01-01 Opening\n    assets:bank:checking:emergency  $300\n    assets:bank:checking  $700\n    equity:opening\n\n2026-01-05 Check\n    assets:bank:checking  $0 =* $1,000\n",
        "2026-01-01 Opening\n    assets:bank:checking  $10\n    equity:opening\n\n2026-01-05 Check\n    assets:bank:checking  $0 == $10\n",
        // Amounts and styles.
        "2026-01-02 Grouped\n    expenses:food:groceries  $1,234.5\n    assets:bank:checking\n",
        "2026-01-02 Comma\n    expenses:travel  1.234,5 EUR\n    assets:bank:checking\n",
        "2026-01-02 Exponent\n    expenses:food:groceries  $1e3\n    assets:bank:checking\n",
        "2026-01-02=2026-01-01 * (1042) Fuel | pump 4  ; receipt\n    ; on the dash\n    ! expenses:transport:fuel  45.10 CAD  ; diesel\n    assets:bank:checking\n",
        "2026-01-02\n    assets:bank:checking  $0\n\n2026-01-03 Nothing\n",
    ];
    for case in cases {
        compare(&h, &format!("{HEAD}{case}"));
    }
}

#[test]
fn the_declared_differences_are_differences() {
    let Some(binary) = hledger() else { return };
    let h = Hledger { binary };
    let dir = tempfile::tempdir().unwrap();
    // Same-day order: hledger fails one order, the engine passes both.
    let dep = "2026-01-05 Paycheck\n    assets:bank:checking  $10\n    income:salary\n";
    let chk = "2026-01-05 Check\n    assets:bank:checking  $0 = $10\n";
    let swapped = write(
        dir.path(),
        "swapped.journal",
        &format!("{HEAD}{chk}\n{dep}"),
    );
    assert_eq!(h.verdict(&swapped), Some(FailureKind::Assertion));
    assert!(read(&FsSource, &swapped).unwrap().check().is_empty());
    // A total inclusive assertion: hledger only looks at the account's own
    // commodities.
    let tree = write(
        dir.path(),
        "tree.journal",
        &format!(
            "{HEAD}2026-01-01 Opening\n    assets:bank:checking:emergency  $10\n    assets:bank:checking  2,00 EUR\n    equity:opening  $-10\n    equity:opening  -2,00 EUR\n\n2026-01-02 Check\n    assets:bank:checking  0 EUR ==* 2,00 EUR\n"
        ),
    );
    assert_eq!(h.verdict(&tree), None);
    let ours = read(&FsSource, &tree).unwrap().check();
    assert_eq!(ours[0].kind, FailureKind::Assertion);
    // A date without a year.
    let no_year = write(
        dir.path(),
        "noyear.journal",
        &format!("{HEAD}01/02 Shop\n    expenses:food:groceries  $1\n    assets:bank:checking\n"),
    );
    assert_eq!(h.verdict(&no_year), None);
    assert!(read(&FsSource, &no_year).is_err());
}

/// The most decimal places hledger reads: at least the engine's 255.
#[test]
fn hledger_reads_at_least_255_places() {
    let Some(binary) = hledger() else { return };
    let h = Hledger { binary };
    let dir = tempfile::tempdir().unwrap();
    let accepts = |places: usize| {
        let amount = format!("0.{}1", "0".repeat(places - 1));
        let path = write(
            dir.path(),
            "places.journal",
            &format!(
                "commodity 1000.00 USD\naccount a\naccount b\n\n2026-01-02 x\n    a  {amount} USD\n    b\n"
            ),
        );
        h.verdict(&path).is_none()
    };
    assert!(accepts(255), "hledger refuses 255 places");
    let (mut low, mut high) = (255, 4096);
    if accepts(high) {
        eprintln!("hledger reads at least {high} decimal places");
        return;
    }
    while high - low > 1 {
        let mid = (low + high) / 2;
        if accepts(mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    eprintln!("hledger reads up to {low} decimal places");
}

const PAYEES: &[&str] = &[
    "Trader Joe's",
    "Shell",
    "Corner Grocer",
    "Acme Payroll",
    "Blue Bottle Coffee",
    "Air Canada | YVR-SFO",
    "Costco Wholesale",
];
const SPEND: &[&str] = &[
    "expenses:food:groceries",
    "expenses:transport:fuel",
    "expenses:travel",
];

#[derive(Debug, Clone)]
struct Generated {
    day: u32,
    payee: usize,
    status: u8,
    legs: Vec<(usize, i64, u8)>,
    omit: bool,
    commodity: u8,
    assert_after: bool,
    break_it: u8,
}

fn generated() -> impl Strategy<Value = Generated> {
    (
        1u32..365,
        0..PAYEES.len(),
        0u8..3,
        proptest::collection::vec((0..SPEND.len(), 1i64..500_000, 0u8..3), 1..4),
        any::<bool>(),
        0u8..3,
        any::<bool>(),
        0u8..12,
    )
        .prop_map(
            |(day, payee, status, legs, omit, commodity, assert_after, break_it)| Generated {
                day,
                payee,
                status,
                legs,
                omit,
                commodity,
                assert_after,
                break_it,
            },
        )
}

fn money(cents: i64, commodity: u8) -> String {
    let whole = cents / 100;
    let frac = (cents % 100).abs();
    let sign = if cents < 0 { "-" } else { "" };
    match commodity {
        0 => format!(
            "{sign}${},{:03}.{frac:02}",
            whole.abs() / 1000,
            whole.abs() % 1000
        )
        .replace("$0,", "$")
        .replace("$00", "$")
        .replace("$0", "$"),
        1 => format!("{sign}{}.{frac:02} CAD", whole.abs()),
        _ => format!("{sign}{},{frac:02} EUR", whole.abs()),
    }
}

/// A book of generated transactions, one per day, each asserting the
/// checking account's end-of-day balance when `assert_after`, with one
/// change that may break it.
fn journal(txs: &[Generated]) -> String {
    let mut used_days = std::collections::BTreeSet::new();
    let mut out = String::from(HEAD);
    let mut balance: [i64; 3] = [0; 3];
    let mut sorted: Vec<&Generated> = txs.iter().collect();
    sorted.sort_by_key(|g| g.day);
    for g in sorted {
        if !used_days.insert(g.day) {
            continue;
        }
        let date = chrono::NaiveDate::from_yo_opt(2026, g.day).unwrap();
        let status = ["", "* ", "! "][g.status as usize];
        out.push_str(&format!("\n{date} {status}{}\n", PAYEES[g.payee]));
        let mut total = 0;
        for (account, cents, mark) in &g.legs {
            let mark = ["", "* ", "! "][*mark as usize];
            out.push_str(&format!(
                "    {mark}{}  {}\n",
                SPEND[*account],
                money(*cents, g.commodity)
            ));
            total += cents;
        }
        let account = match g.break_it {
            0 => "assets:bank:chequing",
            _ => "assets:bank:checking",
        };
        let off = if g.break_it == 1 { 1 } else { 0 };
        if g.omit && off == 0 {
            out.push_str(&format!("    {account}\n"));
        } else {
            out.push_str(&format!(
                "    {account}  {}\n",
                money(-total + off, g.commodity)
            ));
        }
        balance[g.commodity as usize] -= total;
        if g.assert_after {
            let asserted = balance[g.commodity as usize] + if g.break_it == 2 { 100 } else { 0 };
            out.push_str(&format!(
                "    assets:bank:checking  {} = {}\n",
                money(0, g.commodity),
                money(asserted, g.commodity)
            ));
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 40, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn generated_books_agree(txs in proptest::collection::vec(generated(), 1..12)) {
        let Some(binary) = hledger() else { return Ok(()) };
        compare(&Hledger { binary }, &journal(&txs));
    }
}

#[test]
fn kinds_of_postings_survive_the_round_trip() {
    let Some(binary) = hledger() else { return };
    let h = Hledger { binary };
    compare(
        &h,
        &format!("{HEAD}2026-03-01 Budget\n    (expenses:food:groceries)  $-200\n    [assets:bank:checking:emergency]  $50\n    [assets:bank:checking]  $-50\n"),
    );
    let book = {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.journal",
            &format!("{HEAD}2026-03-01 Budget\n    (expenses:food:groceries)  $-200\n"),
        );
        read(&FsSource, &main).unwrap().book
    };
    assert_eq!(book.transactions[0].postings[0].kind, PostingKind::Virtual);
}
