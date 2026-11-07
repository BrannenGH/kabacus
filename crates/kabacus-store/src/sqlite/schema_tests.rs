// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use rusqlite::params;
use std::path::{Path, PathBuf};

fn db() -> Connection {
    open_in_memory().unwrap()
}

fn ledger(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO ledgers (name, created_at) VALUES (?1, '2026-03-01T09:00:00Z')",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn user(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO users (display_name, created_at) VALUES (?1, '2026-03-01T09:00:00Z')",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn account(conn: &Connection, ledger: i64, name: &str) {
    conn.execute(
        "INSERT INTO accounts (ledger_id, name) VALUES (?1, ?2)",
        params![ledger, name],
    )
    .unwrap();
}

fn commodity(conn: &Connection, ledger: i64, symbol: &str) {
    conn.execute(
        "INSERT INTO commodities (ledger_id, symbol, places) VALUES (?1, ?2, 2)",
        params![ledger, symbol],
    )
    .unwrap();
}

fn transaction(conn: &Connection, ledger: i64, id: i64, date: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO transactions (ledger_id, id, date, description, revision_created, revision_changed)
         VALUES (?1, ?2, ?3, 'Trader Joe''s', 1, 1)",
        params![ledger, id, date],
    )
}

fn posting(
    conn: &Connection,
    ledger: i64,
    txn: i64,
    position: i64,
    account: &str,
    mantissa: &str,
    commodity: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity)
         VALUES (?1, ?2, ?3, ?4, ?5, 2, ?6)",
        params![ledger, txn, position, account, mantissa, commodity],
    )
}

fn seeded() -> (Connection, i64) {
    let conn = db();
    let l = ledger(&conn, "Household");
    account(&conn, l, "expenses:groceries");
    account(&conn, l, "assets:checking");
    commodity(&conn, l, "$");
    (conn, l)
}

fn audit(conn: &Connection, ledger: i64, user: i64, revision: i64) {
    conn.execute(
        "INSERT INTO audit_log (ledger_id, revision, n, user_id, actor, kind, summary, payload)
         VALUES (?1, ?2, 0, ?3, 'mrivera', 'add', 'Added a transaction', '{}')",
        params![ledger, revision, user],
    )
    .unwrap();
}

#[test]
fn migrations_are_versioned_and_idempotent() {
    let mut conn = db();
    let v: usize = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, latest_version());
    migrate(&mut conn).unwrap();
}

#[test]
fn refuses_a_newer_schema() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", latest_version() + 1)
        .unwrap();
    assert!(matches!(migrate(&mut conn), Err(SchemaError::Newer { .. })));
}

#[test]
fn every_table_is_strict() {
    let conn = db();
    let mut stmt = conn
        .prepare("SELECT name, strict FROM pragma_table_list WHERE schema = 'main' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 9);
    assert!(rows.iter().all(|(_, strict)| *strict == 1), "{rows:?}");
}

#[test]
fn a_posting_cannot_cross_ledgers() {
    let (conn, a) = seeded();
    let b = ledger(&conn, "Side business");
    account(&conn, b, "income:consulting");
    commodity(&conn, b, "$");
    transaction(&conn, a, 1, "2026-03-02").unwrap();

    // Account belongs to the other ledger.
    assert!(posting(&conn, a, 1, 0, "income:consulting", "100", "$").is_err());
    // Transaction belongs to the other ledger.
    assert!(posting(&conn, b, 1, 0, "income:consulting", "100", "$").is_err());
    // A commodity declared only elsewhere.
    commodity(&conn, b, "EUR");
    assert!(posting(&conn, a, 1, 0, "assets:checking", "100", "EUR").is_err());
    posting(&conn, a, 1, 0, "assets:checking", "100", "$").unwrap();
}

#[test]
fn a_cost_commodity_must_be_declared_in_the_ledger() {
    let (conn, l) = seeded();
    transaction(&conn, l, 1, "2026-03-02").unwrap();
    let insert = |commodity: &str| {
        conn.execute(
            "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity,
                                   cost_kind, cost_mantissa, cost_places, cost_commodity)
             VALUES (?1, 1, 0, 'assets:checking', '10', 0, '$', 'total', '9', 2, ?2)",
            params![l, commodity],
        )
    };
    assert!(insert("EUR").is_err());
    commodity(&conn, l, "EUR");
    insert("EUR").unwrap();
}

#[test]
fn cost_and_assertion_columns_come_whole_or_not_at_all() {
    let (conn, l) = seeded();
    transaction(&conn, l, 1, "2026-03-02").unwrap();
    let cost = conn.execute(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity, cost_kind)
         VALUES (?1, 1, 0, 'assets:checking', '10', 0, '$', 'unit')",
        [l],
    );
    assert!(cost.is_err());
    let assertion = conn.execute(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity, assert_op)
         VALUES (?1, 1, 0, 'assets:checking', '10', 0, '$', '==')",
        [l],
    );
    assert!(assertion.is_err());
    conn.execute(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity,
                               assert_op, assert_mantissa, assert_places, assert_commodity)
         VALUES (?1, 1, 0, 'assets:checking', '10', 0, '$', '=*', '120050', 2, '$')",
        [l],
    )
    .unwrap();
    let bad_op = conn.execute(
        "UPDATE postings SET assert_op = '=>' WHERE ledger_id = ?1",
        [l],
    );
    assert!(bad_op.is_err());
}

#[test]
fn amounts_of_any_size_up_to_255_places_are_stored_exactly() {
    let (conn, l) = seeded();
    transaction(&conn, l, 1, "2026-03-02").unwrap();
    let huge = format!("-{}", "9".repeat(80));
    conn.execute(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, mantissa, places, commodity)
         VALUES (?1, 1, 0, 'assets:checking', ?2, 255, '$')",
        params![l, huge],
    )
    .unwrap();
    let (m, p): (String, i64) = conn
        .query_row("SELECT mantissa, places FROM postings", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((m, p), (huge, 255));
    assert!(conn
        .execute("UPDATE postings SET places = 256", [])
        .is_err());
}

#[test]
fn a_mantissa_must_be_a_decimal_integer() {
    let (conn, l) = seeded();
    transaction(&conn, l, 1, "2026-03-02").unwrap();
    for bad in ["", "-", "12.50", "1e3", "abc", "+5", "1 000"] {
        assert!(
            posting(&conn, l, 1, 0, "assets:checking", bad, "$").is_err(),
            "{bad:?} accepted"
        );
    }
    for good in ["0", "-1250", "00042"] {
        let position = conn
            .query_row("SELECT COUNT(*) FROM postings", [], |r| r.get::<_, i64>(0))
            .unwrap();
        posting(&conn, l, 1, position, "assets:checking", good, "$").unwrap();
    }
}

#[test]
fn transaction_ids_are_per_ledger_and_dates_are_calendar_dates() {
    let (conn, a) = seeded();
    let b = ledger(&conn, "Side business");
    transaction(&conn, a, 1, "2026-03-02").unwrap();
    transaction(&conn, b, 1, "2026-03-02").unwrap();
    assert!(transaction(&conn, a, 1, "2026-03-03").is_err());
    for bad in ["2026-02-30", "03/02/2026", "2026-03-02 10:00", ""] {
        assert!(transaction(&conn, a, 2, bad).is_err(), "{bad:?} accepted");
    }
    assert!(transaction(&conn, a, 0, "2026-03-02").is_err());
}

#[test]
fn deleting_a_transaction_removes_its_postings() {
    let (conn, l) = seeded();
    transaction(&conn, l, 1, "2026-03-02").unwrap();
    posting(&conn, l, 1, 0, "assets:checking", "-4250", "$").unwrap();
    posting(&conn, l, 1, 1, "expenses:groceries", "4250", "$").unwrap();
    conn.execute("DELETE FROM transactions", []).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM postings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn account_names_sort_by_utf8_bytes() {
    let (conn, l) = seeded();
    for name in [
        "expenses:Zoo",
        "expenses:école",
        "expenses:apple",
        "expenses:Éclair",
    ] {
        account(&conn, l, name);
    }
    let mut stmt = conn
        .prepare("SELECT name FROM accounts WHERE name LIKE 'expenses:%' ORDER BY name")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut expected = names.clone();
    expected.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
    assert_eq!(names, expected);
    assert_eq!(names.last().unwrap(), "expenses:école");
}

#[test]
fn account_names_compare_exactly() {
    let (conn, l) = seeded();
    account(&conn, l, "Expenses:Groceries");
    assert!(conn
        .execute(
            "INSERT INTO accounts (ledger_id, name) VALUES (?1, 'expenses:groceries')",
            [l]
        )
        .is_err());
}

#[test]
fn the_audit_log_cannot_be_rewritten() {
    let (conn, l) = seeded();
    let u = user(&conn, "M. Rivera");
    audit(&conn, l, u, 1);
    assert!(conn
        .execute("UPDATE audit_log SET summary = 'x'", [])
        .is_err());
    assert!(conn.execute("DELETE FROM audit_log", []).is_err());
    assert!(conn
        .execute(
            "DELETE FROM audit_log WHERE ledger_id = ?1 AND revision = 1",
            [l]
        )
        .is_err());
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn audit_rows_are_unique_per_revision_and_must_be_json() {
    let (conn, l) = seeded();
    let u = user(&conn, "M. Rivera");
    audit(&conn, l, u, 1);
    let dup = conn.execute(
        "INSERT INTO audit_log VALUES (?1, 1, 0, ?2, 'mrivera', 'add', 's', '{}')",
        params![l, u],
    );
    assert!(dup.is_err());
    let not_json = conn.execute(
        "INSERT INTO audit_log VALUES (?1, 2, 0, ?2, 'mrivera', 'add', 's', 'not json')",
        params![l, u],
    );
    assert!(not_json.is_err());
}

#[test]
fn deleting_a_ledger_deletes_its_rows_and_only_its_rows() {
    let (conn, a) = seeded();
    let b = ledger(&conn, "Side business");
    let u = user(&conn, "M. Rivera");
    for l in [a, b] {
        if l == b {
            account(&conn, b, "assets:checking");
            commodity(&conn, b, "$");
        }
        transaction(&conn, l, 1, "2026-03-02").unwrap();
        posting(&conn, l, 1, 0, "assets:checking", "100", "$").unwrap();
        conn.execute(
            "INSERT INTO memberships (ledger_id, user_id, role) VALUES (?1, ?2, 'rewrite')",
            params![l, u],
        )
        .unwrap();
        audit(&conn, l, u, 1);
    }
    conn.execute("DELETE FROM ledgers WHERE id = ?1", [a])
        .unwrap();
    for table in [
        "transactions",
        "postings",
        "accounts",
        "commodities",
        "memberships",
        "audit_log",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE ledger_id = ?1");
        let in_a: i64 = conn.query_row(&sql, [a], |r| r.get(0)).unwrap();
        let in_b: i64 = conn.query_row(&sql, [b], |r| r.get(0)).unwrap();
        assert_eq!((in_a, in_b), (0, 1), "{table}");
    }
}

#[test]
fn memberships_and_identities() {
    let (conn, l) = seeded();
    let u = user(&conn, "M. Rivera");
    let insert = |role: &str| {
        conn.execute(
            "INSERT INTO memberships (ledger_id, user_id, role) VALUES (?1, ?2, ?3)",
            params![l, u, role],
        )
    };
    assert!(insert("owner").is_err());
    insert("write").unwrap();
    assert!(insert("read").is_err());

    let ident = |user: i64| {
        conn.execute(
            "INSERT INTO identities (provider, subject, user_id) VALUES ('https://id.example.org/realms/home', 'f3a1c2d4', ?1)",
            [user],
        )
    };
    assert!(ident(9999).is_err());
    ident(u).unwrap();
    assert!(ident(u).is_err());
}

#[test]
fn ledger_revisions_are_never_negative() {
    let (conn, l) = seeded();
    conn.execute(
        "UPDATE ledgers SET revision = revision + 1 WHERE id = ?1",
        [l],
    )
    .unwrap();
    assert!(conn
        .execute("UPDATE ledgers SET revision = -1 WHERE id = ?1", [l])
        .is_err());
}

const INITIAL: &str = include_str!("migrations/0001_initial.sql");

fn tables(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn backup_of(path: &Path, version: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".v{version}.bak"));
    PathBuf::from(name)
}

fn books_at_version_one(path: &Path) {
    let conn = open_file_with(path, &[INITIAL]).unwrap();
    ledger(&conn, "Household");
}

#[test]
fn a_new_file_is_created_without_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.db");
    let conn = open_file(&path).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), latest_version());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn migrating_copies_the_file_aside_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.db");
    books_at_version_one(&path);
    let conn = open_file_with(
        &path,
        &[INITIAL, "CREATE TABLE tags (name TEXT NOT NULL) STRICT;"],
    )
    .unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 2);
    assert!(tables(&conn).contains(&"tags".to_string()));

    let backup = Connection::open(backup_of(&path, 1)).unwrap();
    assert_eq!(schema_version(&backup).unwrap(), 1);
    assert!(!tables(&backup).contains(&"tags".to_string()));
    let name: String = backup
        .query_row("SELECT name FROM ledgers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Household");
}

#[test]
fn an_up_to_date_database_is_not_copied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.db");
    open_file(&path).unwrap();
    open_file(&path).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_newer_schema_is_refused_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.db");
    open_file(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", latest_version() + 1)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        open_file(&path),
        Err(SchemaError::Newer { found, known })
            if found == latest_version() + 1 && known == latest_version()
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_migration_that_fails_part_way_leaves_the_original_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.db");
    books_at_version_one(&path);
    let result = open_file_with(
        &path,
        &[
            INITIAL,
            "CREATE TABLE tags (name TEXT NOT NULL) STRICT;",
            "INSERT INTO no_such_table VALUES (1);",
        ],
    );
    assert!(matches!(result, Err(SchemaError::Sqlite(_))));

    let conn = Connection::open(&path).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 1);
    assert!(!tables(&conn).contains(&"tags".to_string()));
    let name: String = conn
        .query_row("SELECT name FROM ledgers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Household");
    assert!(backup_of(&path, 1).exists());
}

#[test]
fn a_new_file_is_a_kabacus_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("books.kabacus");
    let conn = open_file(&path).unwrap();
    assert_eq!(application_id(&conn).unwrap(), APPLICATION_ID);
    assert_eq!(APPLICATION_ID.to_be_bytes(), *b"KBCS");
}

#[test]
fn a_sqlite_file_that_isnt_kabacus_is_refused_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('milk');")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let err = open_file(&path).unwrap_err();
    assert!(matches!(err, SchemaError::NotKabacus(_)), "{err:?}");
    assert_eq!(
        err.to_string(),
        format!("{} isn't a Kabacus file", path.display())
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
