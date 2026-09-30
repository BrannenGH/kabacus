// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The shared storage suite: every test runs against each store, which
//! must behave identically. PostgreSQL tests use a throwaway database
//! (`kabacus_store::testing`).

use kabacus_core::{AccountName, Amount, Money, Posting, PostingKind, Status, Transaction};
use kabacus_ledger::{import, Book, MemorySource};
use kabacus_store::{
    checked, AnyStore, BookStore, ChangeSet, LedgerDump, PostgresStore, SqliteStore, StoreError,
    Who,
};
use std::path::{Path, PathBuf};

enum Under {
    Sqlite(PathBuf),
    Postgres(String),
}

struct Fixture {
    store: AnyStore,
    under: Under,
    _dir: tempfile::TempDir,
}

fn sqlite() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.kabacus");
    Fixture {
        store: AnyStore::Sqlite(SqliteStore::open(&path).unwrap()),
        under: Under::Sqlite(path),
        _dir: dir,
    }
}

async fn postgres() -> Option<Fixture> {
    let url = kabacus_store::testing::database().await?;
    Some(Fixture {
        store: AnyStore::Postgres(PostgresStore::connect(&url, 8).await.unwrap()),
        under: Under::Postgres(url),
        _dir: tempfile::tempdir().unwrap(),
    })
}

/// Run SQL under the store, as the server's own operations do.
async fn raw(under: &Under, sql: &str) -> Result<(), String> {
    match under {
        Under::Sqlite(path) => {
            let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
            conn.execute_batch(sql).map_err(|e| e.to_string())
        }
        Under::Postgres(url) => {
            let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
                .await
                .map_err(|e| e.to_string())?;
            tokio::spawn(connection);
            client
                .batch_execute(&format!("SET kabacus.scope = 'all'; {sql}"))
                .await
                .map_err(|e| match e.as_db_error() {
                    Some(db) => db.message().to_string(),
                    None => e.to_string(),
                })
        }
    }
}

fn book(text: &str) -> Book {
    let src = MemorySource(
        [(PathBuf::from("/b/main.journal"), text.to_string())]
            .into_iter()
            .collect(),
    );
    import(&src, Path::new("/b/main.journal")).unwrap().book
}

const BOOKS: &str = "commodity $1,000.00\ncommodity 1.000,00 EUR\naccount assets:checking  ; ynab-account:Everyday Checking\naccount expenses:groceries\naccount expenses:travel\naccount income:salary\naccount equity:opening\n\n2026-01-01 Opening\n    assets:checking  $1,000.00\n    equity:opening\n\n2026-01-03 * Trader Joe's | weekly shop  ; receipt kept\n    expenses:groceries  $45.17\n    assets:checking\n\n2026-01-05 Hotel Lisboa\n    expenses:travel  100,00 EUR @@ $110.00\n    assets:checking\n";

fn who(user: i64, kind: &str, summary: &str) -> Who {
    Who {
        user,
        actor: "app".into(),
        kind: kind.into(),
        summary: summary.into(),
        details: serde_json::json!({"client": "kabacus-app", "session": "s-81f2"}),
    }
}

fn usd(cents: i128) -> Amount {
    Amount::new("$", Money::from_decimal(cents, 2))
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

fn purchase(id: u64, day: u32, payee: &str, cents: i128) -> Transaction {
    Transaction {
        id,
        date: chrono::NaiveDate::from_ymd_opt(2026, 2, day).unwrap(),
        status: Status::Unmarked,
        code: String::new(),
        description: payee.into(),
        comment: String::new(),
        tags: vec![],
        postings: vec![
            posting("expenses:groceries", usd(cents)),
            posting("assets:checking", usd(-cents)),
        ],
    }
}

/// A ledger with `BOOKS` imported, and its owner.
async fn seeded(store: &AnyStore) -> (i64, i64) {
    let owner = store
        .user_for_identity(
            "https://id.example.org/realms/home",
            "f2c1-7a",
            "Maya Rivera",
        )
        .await
        .unwrap();
    let ledger = store.create_ledger("Household", owner).await.unwrap();
    store
        .import(
            ledger,
            &book(BOOKS),
            &who(owner, "import", "import: main.journal"),
        )
        .await
        .unwrap();
    (ledger, owner)
}

/// Add `txs` on top of what is loaded now.
async fn add(
    store: &AnyStore,
    ledger: i64,
    user: i64,
    txs: Vec<Transaction>,
) -> Result<i64, StoreError> {
    let loaded = store.load(ledger, None).await?;
    let mut after = loaded.book.clone();
    after.transactions.extend(txs);
    store
        .commit(
            ledger,
            loaded.revision,
            ChangeSet::between(&loaded.book, &after),
            &who(user, "add", "add"),
        )
        .await
}

async fn revisions_count_up_and_a_stale_write_is_a_conflict(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    assert_eq!(store.revision(ledger).await.unwrap(), 2);
    let loaded = store.load(ledger, None).await.unwrap();
    assert_eq!(loaded.next_id, 4);
    assert_eq!(
        add(
            store,
            ledger,
            user,
            vec![purchase(4, 2, "Corner Grocer", 1250)]
        )
        .await
        .unwrap(),
        3
    );
    // Written against revision 2, after revision 3 exists.
    let mut stale = loaded.book.clone();
    stale
        .transactions
        .push(purchase(5, 3, "Blue Bottle Coffee", 450));
    let err = store
        .commit(
            ledger,
            2,
            ChangeSet::between(&loaded.book, &stale),
            &who(user, "add", "add"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::Conflict {
                expected: 2,
                current: 3
            }
        ),
        "{err:?}"
    );
    assert_eq!(store.revision(ledger).await.unwrap(), 3);
    assert!(matches!(
        store
            .commit(ledger, 3, ChangeSet::default(), &who(user, "add", "add"))
            .await,
        Err(StoreError::Invalid(_))
    ));
}

async fn a_write_is_atomic(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let hundred: Vec<Transaction> = (0..100)
        .map(|i| {
            purchase(
                4 + i,
                1 + (i as u32 % 28),
                "Costco Wholesale",
                1000 + i as i128,
            )
        })
        .collect();
    assert_eq!(add(store, ledger, user, hundred).await.unwrap(), 3);
    assert_eq!(
        store
            .load(ledger, None)
            .await
            .unwrap()
            .book
            .transactions
            .len(),
        103
    );
    // The last posting names an account that isn't declared: nothing is
    // written, not even the transactions before it.
    let mut bad = vec![
        purchase(104, 4, "Shell", 4210),
        purchase(105, 5, "Shell", 3890),
    ];
    bad[1].postings[0].account = AccountName::new("expenses:fuel");
    assert!(add(store, ledger, user, bad).await.is_err());
    let loaded = store.load(ledger, None).await.unwrap();
    assert_eq!(loaded.revision, 3);
    assert_eq!(loaded.book.transactions.len(), 103);
    assert_eq!(store.history(ledger, 10).await.unwrap().len(), 3);
}

async fn history_and_the_rows_each_write_changed(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    add(
        store,
        ledger,
        user,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
    let history = store.history(ledger, 10).await.unwrap();
    let kinds: Vec<(i64, &str, usize)> = history
        .iter()
        .map(|h| (h.revision, h.kind.as_str(), h.rows))
        .collect();
    assert_eq!(kinds, [(3, "add", 1), (2, "import", 10), (1, "create", 0)]);
    assert_eq!(history[0].user, "Maya Rivera");
    assert_eq!(history[0].details["session"], "s-81f2");
    assert!(history[0].at.starts_with("20"), "{}", history[0].at);
    assert_eq!(store.history(ledger, 1).await.unwrap().len(), 1);
    let change = store.change(ledger, 3).await.unwrap();
    assert_eq!(change.changes.len(), 1);
    assert!(change.changes[0].before.is_none());
    assert!(matches!(
        store.change(ledger, 9).await,
        Err(StoreError::NotFound(_))
    ));
}

async fn books_at_a_past_revision(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let at2 = store.load(ledger, None).await.unwrap().book;
    add(
        store,
        ledger,
        user,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
    // Revision 4 edits transaction 2 and deletes transaction 3.
    let loaded = store.load(ledger, None).await.unwrap();
    let mut after = loaded.book.clone();
    after.transactions.retain(|t| t.id != 3);
    after
        .transactions
        .iter_mut()
        .find(|t| t.id == 2)
        .unwrap()
        .description = "Trader Joe's".into();
    store
        .commit(
            ledger,
            3,
            ChangeSet::between(&loaded.book, &after),
            &who(user, "edit", "edit"),
        )
        .await
        .unwrap();
    assert_eq!(store.load(ledger, Some(2)).await.unwrap().book, at2);
    assert_eq!(store.load(ledger, Some(3)).await.unwrap().book, loaded.book);
    assert_eq!(store.load(ledger, Some(4)).await.unwrap().book, after);
    assert!(store
        .load(ledger, Some(1))
        .await
        .unwrap()
        .book
        .transactions
        .is_empty());
    assert!(store.load(ledger, Some(5)).await.is_err());
}

async fn money_is_exact_far_beyond_128_bits_and_to_255_places(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let huge: num_bigint::BigInt = format!("{}17", "9".repeat(60)).parse().unwrap();
    let tiny = Money::from_decimal(1, 255);
    let mut tx = purchase(4, 2, "Kapital Bank", 0);
    tx.postings = vec![
        posting(
            "expenses:groceries",
            Amount::new("$", Money::from_big(huge.clone(), 2)),
        ),
        posting(
            "assets:checking",
            Amount::new("$", Money::from_big(-huge.clone(), 2)),
        ),
        posting("expenses:groceries", Amount::new("$", tiny.clone())),
        posting("assets:checking", Amount::new("$", -tiny.clone())),
    ];
    let loaded = store.load(ledger, None).await.unwrap();
    let mut after = loaded.book.clone();
    after.transactions.push(tx.clone());
    after.commodities.get_mut("$").unwrap().style.places = Some(255);
    store
        .commit(
            ledger,
            loaded.revision,
            ChangeSet::between(&loaded.book, &after),
            &who(user, "add", "add"),
        )
        .await
        .unwrap();
    let back = store.load(ledger, None).await.unwrap().book;
    let stored = back.transaction(4).unwrap();
    assert_eq!(stored, &tx);
    assert_eq!(stored.postings[0].amount.money.mantissa(), &huge);
    assert_eq!(stored.postings[2].amount.money.places(), 255);
}

async fn names_sort_by_their_bytes(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let loaded = store.load(ledger, None).await.unwrap();
    let mut after = loaded.book.clone();
    for name in [
        "expenses:Ärger",
        "expenses:zebra",
        "expenses:Zoo",
        "expenses:épicerie",
        "expenses:apple",
    ] {
        after.accounts.insert(
            name.into(),
            kabacus_ledger::Account {
                name: AccountName::new(name),
                tags: String::new(),
            },
        );
    }
    store
        .commit(
            ledger,
            loaded.revision,
            ChangeSet::between(&loaded.book, &after),
            &who(user, "rewrite", "declare"),
        )
        .await
        .unwrap();
    let names: Vec<String> = store
        .dump(ledger)
        .await
        .unwrap()
        .accounts
        .into_iter()
        .map(|a| a.name)
        .filter(|n| n.starts_with("expenses:") && !n.contains("groceries") && !n.contains("travel"))
        .collect();
    assert_eq!(
        names,
        [
            "expenses:Zoo",
            "expenses:apple",
            "expenses:zebra",
            "expenses:Ärger",
            "expenses:épicerie"
        ]
    );
}

async fn a_write_that_breaks_an_assertion_is_refused(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let loaded = store.load(ledger, None).await.unwrap();
    let mut asserting = purchase(4, 10, "Reconciliation", 0);
    asserting.postings = vec![posting("assets:checking", usd(0))];
    asserting.postings[0].balance_assertion = Some(kabacus_core::BalanceAssertion::new(
        Amount::new("$", Money::from_decimal(84483, 2)),
    ));
    let mut after = loaded.book.clone();
    after.transactions.push(asserting);
    let changes = ChangeSet::between(&loaded.book, &after);
    let (_, failures) = checked(&loaded.book, &changes).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    store
        .commit(
            ledger,
            loaded.revision,
            changes,
            &who(user, "reconcile", "reconcile"),
        )
        .await
        .unwrap();

    // A purchase dated before the assertion would make it fail.
    let loaded = store.load(ledger, None).await.unwrap();
    let mut after = loaded.book.clone();
    after
        .transactions
        .push(purchase(5, 4, "Corner Grocer", 1250));
    let (_, failures) = checked(&loaded.book, &ChangeSet::between(&loaded.book, &after)).unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, kabacus_ledger::FailureKind::Assertion);
    // Dated after it, it doesn't.
    let mut after = loaded.book.clone();
    after
        .transactions
        .push(purchase(5, 11, "Corner Grocer", 1250));
    let (_, failures) = checked(&loaded.book, &ChangeSet::between(&loaded.book, &after)).unwrap();
    assert!(failures.is_empty());
}

async fn the_audit_log_is_append_only(f: Fixture) {
    let store = &f.store;
    let (ledger, _) = seeded(store).await;
    assert!(raw(&f.under, "UPDATE audit_log SET summary = 'rewritten'")
        .await
        .is_err());
    assert!(raw(&f.under, "DELETE FROM audit_log WHERE revision = 2")
        .await
        .is_err());
    assert_eq!(
        store.history(ledger, 10).await.unwrap()[0].summary,
        "import: main.journal"
    );
    // Deleting the ledger takes its history with it.
    raw(
        &f.under,
        &format!("DELETE FROM ledgers WHERE id = {ledger}"),
    )
    .await
    .unwrap();
    assert!(store.ledgers().await.unwrap().is_empty());
}

async fn ledgers_are_kept_apart(f: Fixture) {
    let store = &f.store;
    let (household, user) = seeded(store).await;
    let business = store
        .create_ledger("Rivera Design LLC", user)
        .await
        .unwrap();
    store
        .import(business, &book("commodity $1,000.00\naccount assets:checking\naccount income:clients\n\n2026-01-15 Invoice 1042\n    assets:checking  $2,400.00\n    income:clients\n"), &who(user, "import", "import"))
        .await
        .unwrap();
    let h = store.load(household, None).await.unwrap();
    let b = store.load(business, None).await.unwrap();
    assert_eq!(h.book.transactions.len(), 3);
    assert_eq!(b.book.transactions.len(), 1);
    assert_eq!(b.book.transactions[0].id, 1);
    assert_eq!((h.revision, b.revision), (2, 2));
    add(
        store,
        household,
        user,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
    assert_eq!(store.revision(business).await.unwrap(), 2);
    let names: Vec<String> = store
        .ledgers()
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(names, ["Household", "Rivera Design LLC"]);
}

async fn transaction_ids_are_never_reused(f: Fixture) {
    let store = &f.store;
    let (ledger, user) = seeded(store).await;
    let loaded = store.load(ledger, None).await.unwrap();
    let mut after = loaded.book.clone();
    after.transactions.retain(|t| t.id != 3);
    store
        .commit(
            ledger,
            loaded.revision,
            ChangeSet::between(&loaded.book, &after),
            &who(user, "delete", "delete"),
        )
        .await
        .unwrap();
    let loaded = store.load(ledger, None).await.unwrap();
    assert_eq!(loaded.next_id, 4);
    assert!(add(
        store,
        ledger,
        user,
        vec![purchase(3, 2, "Corner Grocer", 1250)]
    )
    .await
    .is_err());
    add(
        store,
        ledger,
        user,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
}

async fn users_identities_and_memberships(f: Fixture) {
    let store = &f.store;
    let (ledger, owner) = seeded(store).await;
    let again = store
        .user_for_identity("https://id.example.org/realms/home", "f2c1-7a", "Maya R.")
        .await
        .unwrap();
    assert_eq!(again, owner);
    let jordan = store
        .user_for_identity("https://accounts.example.com", "f2c1-7a", "Jordan Lee")
        .await
        .unwrap();
    assert_ne!(jordan, owner);
    assert_eq!(
        store.memberships(owner).await.unwrap(),
        [(ledger, kabacus_store::Role::Rewrite)]
    );
    assert!(store.memberships(jordan).await.unwrap().is_empty());
    store
        .set_membership(ledger, jordan, Some(kabacus_store::Role::Read))
        .await
        .unwrap();
    assert_eq!(
        store.memberships(jordan).await.unwrap(),
        [(ledger, kabacus_store::Role::Read)]
    );
    store.set_membership(ledger, jordan, None).await.unwrap();
    assert!(store.memberships(jordan).await.unwrap().is_empty());
    let local = store.local_user("Maya").await.unwrap();
    assert_eq!(store.local_user("Maya").await.unwrap(), local);
}

/// What must survive export and import: everything but the ids the
/// target store gives users.
fn comparable(d: &LedgerDump) -> impl PartialEq + std::fmt::Debug {
    let names: std::collections::BTreeMap<i64, String> = d.users.iter().cloned().collect();
    (
        d.name.clone(),
        d.revision,
        d.next_transaction_id,
        d.accounts.clone(),
        d.commodities.clone(),
        d.transactions.clone(),
        d.audit
            .iter()
            .map(|a| {
                (
                    a.revision,
                    a.n,
                    names[&a.user].clone(),
                    a.actor.clone(),
                    a.kind.clone(),
                    a.summary.clone(),
                    serde_json::from_str::<serde_json::Value>(&a.payload).unwrap(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

async fn export_and_import_keep_rows_and_history(f: Fixture) {
    let store = &f.store;
    let (ledger, owner) = seeded(store).await;
    add(
        store,
        ledger,
        owner,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    store.export_file(&[ledger], &file).await.unwrap();
    assert!(kabacus_store::testing::is_sqlite(&file));
    assert!(!dir.path().join("household.kabacus-wal").exists());
    let conn = rusqlite::Connection::open(&file).unwrap();
    let id: i32 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    assert_eq!(id, 0x4B42_4353);
    let leaked: i64 = conn
        .query_row(
            "SELECT (SELECT count(*) FROM identities) + (SELECT count(*) FROM memberships)
                  + (SELECT count(*) FROM users WHERE password_hash IS NOT NULL OR is_admin)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
    drop(conn);

    let importer = store.local_user("Jordan Lee").await.unwrap();
    let ids = store.import_file(&file, importer).await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_ne!(ids[0], ledger);
    assert_eq!(
        comparable(&store.dump(ids[0]).await.unwrap()),
        comparable(&store.dump(ledger).await.unwrap())
    );
    assert_eq!(
        store.memberships(importer).await.unwrap(),
        [(ids[0], kabacus_store::Role::Rewrite)]
    );
    let history = store.history(ids[0], 10).await.unwrap();
    assert_eq!(history[0].user, "Maya Rivera");
}

async fn an_untrusted_file_is_checked_before_anything_is_written(f: Fixture) {
    let store = &f.store;
    let (ledger, owner) = seeded(store).await;
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.kabacus");
    store.export_file(&[ledger], &good).await.unwrap();
    let before = store.ledgers().await.unwrap().len();

    let variant = |name: &str, sql: &str| {
        let path = dir.path().join(name);
        std::fs::copy(&good, &path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        path
    };
    let cases = [
        variant("wrong-id.kabacus", "PRAGMA application_id = 0;"),
        variant("newer.kabacus", "PRAGMA user_version = 99;"),
        variant("trigger.kabacus", "CREATE TRIGGER sneak AFTER INSERT ON users BEGIN SELECT 1; END;"),
        variant("view.kabacus", "CREATE VIEW peek AS SELECT * FROM users;"),
        variant(
            "history.kabacus",
            "UPDATE transactions SET description = 'Altered' WHERE id = 2;",
        ),
        variant(
            "assertion.kabacus",
            "UPDATE postings SET assert_op = '=', assert_mantissa = '1', assert_places = 2, assert_commodity = '$' WHERE transaction_id = 2 AND position = 0;",
        ),
    ];
    for path in &cases {
        let err = store.import_file(path, owner).await.unwrap_err();
        assert!(
            matches!(err, StoreError::File(_)),
            "{}: {err:?}",
            path.display()
        );
    }
    // A damaged page.
    let corrupt = dir.path().join("corrupt.kabacus");
    let mut bytes = std::fs::read(&good).unwrap();
    let page = 4096 * 3;
    for b in bytes.iter_mut().skip(page).take(512) {
        *b = 0xA5;
    }
    std::fs::write(&corrupt, &bytes).unwrap();
    assert!(store.import_file(&corrupt, owner).await.is_err());
    let plain = dir.path().join("plain.kabacus");
    rusqlite::Connection::open(&plain)
        .unwrap()
        .execute_batch("CREATE TABLE notes (body TEXT);")
        .unwrap();
    assert!(matches!(
        store.import_file(&plain, owner).await,
        Err(StoreError::File(_))
    ));
    assert_eq!(store.ledgers().await.unwrap().len(), before);
}

// Every backend runs the whole suite: it is what shows they behave alike.
macro_rules! on {
    ($backend:ident, $fixture:expr, $($name:ident),* $(,)?) => {
        mod $backend {
            $(
                #[tokio::test]
                async fn $name() {
                    let Some(f) = $fixture else { return };
                    super::$name(f).await
                }
            )*
        }
    };
}

on!(
    on_sqlite,
    Some(super::sqlite()),
    revisions_count_up_and_a_stale_write_is_a_conflict,
    a_write_is_atomic,
    history_and_the_rows_each_write_changed,
    books_at_a_past_revision,
    money_is_exact_far_beyond_128_bits_and_to_255_places,
    names_sort_by_their_bytes,
    a_write_that_breaks_an_assertion_is_refused,
    the_audit_log_is_append_only,
    ledgers_are_kept_apart,
    transaction_ids_are_never_reused,
    users_identities_and_memberships,
    export_and_import_keep_rows_and_history,
    an_untrusted_file_is_checked_before_anything_is_written,
);

on!(
    on_postgres,
    super::postgres().await,
    revisions_count_up_and_a_stale_write_is_a_conflict,
    a_write_is_atomic,
    history_and_the_rows_each_write_changed,
    books_at_a_past_revision,
    money_is_exact_far_beyond_128_bits_and_to_255_places,
    names_sort_by_their_bytes,
    a_write_that_breaks_an_assertion_is_refused,
    the_audit_log_is_append_only,
    ledgers_are_kept_apart,
    transaction_ids_are_never_reused,
    users_identities_and_memberships,
    export_and_import_keep_rows_and_history,
    an_untrusted_file_is_checked_before_anything_is_written,
);

/// A ledger exported from Postgres, imported into SQLite, exported again
/// and imported back into Postgres has the rows and history it started
/// with, and both stores export the same rows.
#[tokio::test]
async fn ledgers_move_between_the_stores() {
    let Some(pg) = postgres().await else { return };
    let lite = sqlite();
    let (ledger, owner) = seeded(&pg.store).await;
    add(
        &pg.store,
        ledger,
        owner,
        vec![purchase(4, 2, "Corner Grocer", 1250)],
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("from-postgres.kabacus");
    pg.store.export_file(&[ledger], &first).await.unwrap();
    let someone = lite.store.local_user("Maya Rivera").await.unwrap();
    let in_lite = lite.store.import_file(&first, someone).await.unwrap()[0];
    let second = dir.path().join("from-sqlite.kabacus");
    lite.store.export_file(&[in_lite], &second).await.unwrap();
    let back = pg.store.import_file(&second, owner).await.unwrap()[0];
    let original = comparable(&pg.store.dump(ledger).await.unwrap());
    assert_eq!(
        comparable(&lite.store.dump(in_lite).await.unwrap()),
        original
    );
    assert_eq!(comparable(&pg.store.dump(back).await.unwrap()), original);
    assert_eq!(
        pg.store.load(back, Some(2)).await.unwrap().book,
        pg.store.load(ledger, Some(2)).await.unwrap().book
    );
}

/// Row-level security: without a ledger set, nothing is visible; with one
/// set, only its rows.
#[tokio::test]
async fn postgres_rows_are_visible_only_for_the_ledger_set() {
    let Some(f) = postgres().await else { return };
    let (household, user) = seeded(&f.store).await;
    let other = f
        .store
        .create_ledger("Rivera Design LLC", user)
        .await
        .unwrap();
    f.store
        .import(other, &book("commodity $1,000.00\naccount assets:checking\naccount income:clients\n\n2026-01-15 Invoice 1042\n    assets:checking  $2,400.00\n    income:clients\n"), &who(user, "import", "import"))
        .await
        .unwrap();
    let Under::Postgres(url) = &f.under else {
        unreachable!()
    };
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    let count = |sql: String| {
        let client = &client;
        async move { client.query_one(&sql, &[]).await.unwrap().get::<_, i64>(0) }
    };
    for table in [
        "ledgers",
        "transactions",
        "postings",
        "accounts",
        "audit_log",
        "memberships",
    ] {
        assert_eq!(
            count(format!("SELECT count(*) FROM {table}")).await,
            0,
            "{table}"
        );
    }
    client
        .batch_execute(&format!("BEGIN; SET LOCAL kabacus.ledger = '{household}';"))
        .await
        .unwrap();
    assert_eq!(count("SELECT count(*) FROM transactions".into()).await, 3);
    assert_eq!(
        count(format!(
            "SELECT count(*) FROM transactions WHERE ledger_id = {other}"
        ))
        .await,
        0
    );
    assert!(client
        .batch_execute(&format!(
            "INSERT INTO accounts (ledger_id, name) VALUES ({other}, 'expenses:sneaky')"
        ))
        .await
        .is_err());
    client.batch_execute("ROLLBACK").await.unwrap();
    // Pooled connections don't carry a ledger from one transaction to the
    // next.
    assert_eq!(count("SELECT count(*) FROM transactions".into()).await, 0);
}
