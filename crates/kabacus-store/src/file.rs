// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `.kabacus` file: a SQLite database in the SQLite store's schema,
//! marked with the application id `KBCS`. Writing one makes a single
//! self-contained file; reading one treats it as untrusted. Implements the
//! engine implementation specification, section 1.8.

use crate::sqlite::schema::{self, APPLICATION_ID};
use crate::{book_of, replay, LedgerDump, Result, StoreError};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeSet;
use std::path::Path;

/// Write `dumps` to a new `.kabacus` file at `to`: rollback-journal mode,
/// nothing beside it. Of users it holds only the display names the audit
/// logs point at.
pub fn write(dumps: &[LedgerDump], to: &Path) -> Result<()> {
    let dir = to.parent().unwrap_or(Path::new("."));
    let scratch = tempfile::Builder::new()
        .prefix(".kabacus-export")
        .tempfile_in(dir)?;
    {
        let mut conn = Connection::open(scratch.path())?;
        conn.pragma_update(None, "journal_mode", "delete")?;
        schema::migrate(&mut conn)?;
        let mut next_user = 1;
        for dump in dumps {
            // User ids are the file's own: renumber each ledger's.
            let mut renumbered = dump.clone();
            let map: std::collections::BTreeMap<i64, i64> = dump
                .users
                .iter()
                .map(|(old, _)| {
                    let new = next_user;
                    next_user += 1;
                    (*old, new)
                })
                .collect();
            renumbered.users = dump
                .users
                .iter()
                .map(|(old, n)| (map[old], n.clone()))
                .collect();
            for row in &mut renumbered.audit {
                row.user = *map.get(&row.user).ok_or_else(|| {
                    StoreError::Invalid(format!("audit user {} is missing", row.user))
                })?;
            }
            crate::sqlite::write_dump(&mut conn, &renumbered)?;
        }
    }
    scratch.persist(to).map_err(|e| e.error)?;
    Ok(())
}

/// Refuse with one sentence.
fn refuse(reason: impl Into<String>) -> StoreError {
    StoreError::File(reason.into())
}

/// The schema objects our migrations create at `version`.
fn reference_schema(version: usize) -> Result<BTreeSet<(String, String, String)>> {
    let mut conn = Connection::open_in_memory()?;
    schema::migrate_to(&mut conn, version)?;
    schema_objects(&conn)
}

fn schema_objects(conn: &Connection) -> Result<BTreeSet<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, coalesce(sql, '') FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(rows)
}

/// Every ledger in the `.kabacus` file at `from`, checked: the
/// application id, a schema no newer than ours, an intact file with
/// nothing in it our schema doesn't make, an audit log that replays to
/// each ledger's rows, and books that pass the strict check. The file
/// itself is never written to.
pub fn read(from: &Path) -> Result<Vec<LedgerDump>> {
    let scratch = tempfile::Builder::new()
        .prefix(".kabacus-import")
        .tempfile()?;
    std::fs::copy(from, scratch.path()).map_err(|_| refuse("The file can't be read."))?;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let version = {
        let conn = Connection::open_with_flags(scratch.path(), flags)
            .map_err(|_| refuse("This isn't a Kabacus file."))?;
        harden(&conn)?;
        let id = schema::application_id(&conn).map_err(|_| refuse("This isn't a Kabacus file."))?;
        if id != APPLICATION_ID {
            return Err(refuse("This isn't a Kabacus file."));
        }
        let version: usize = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| refuse("This isn't a Kabacus file."))?;
        if version > schema::latest_version() {
            return Err(refuse(
                "This file was made by a newer Kabacus; update Kabacus to import it.",
            ));
        }
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|_| refuse("The file is damaged."))?;
        if integrity != "ok" {
            return Err(refuse("The file is damaged."));
        }
        if schema_objects(&conn)? != reference_schema(version)? {
            return Err(refuse(
                "The file holds things a Kabacus file doesn't, such as views or triggers.",
            ));
        }
        version
    };
    // An older schema is brought up to date on the copy only.
    if version < schema::latest_version() {
        let mut conn = Connection::open(scratch.path())?;
        harden(&conn)?;
        schema::migrate(&mut conn)?;
    }
    let mut conn = Connection::open_with_flags(scratch.path(), flags)?;
    harden(&conn)?;
    let dumps = crate::sqlite::read_dumps(&mut conn).map_err(|e| match e {
        StoreError::Row(e) => refuse(format!("The file holds a malformed row: {e}.")),
        other => other,
    })?;
    if dumps.is_empty() {
        return Err(refuse("The file holds no ledger."));
    }
    for dump in &dumps {
        check_dump(dump)?;
    }
    Ok(dumps)
}

fn harden(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    Ok(())
}

/// A ledger's audit log replays to its rows, and its book passes the
/// strict check.
pub fn check_dump(dump: &LedgerDump) -> Result<()> {
    let name = &dump.name;
    let live = book_of(dump)
        .map_err(|e| refuse(format!("The ledger {name} holds a malformed row: {e}.")))?;
    let replayed = replay(dump)
        .map_err(|e| refuse(format!("The ledger {name}'s history can't be read: {e}.")))?;
    if replayed != live {
        return Err(refuse(format!(
            "The ledger {name}'s history doesn't lead to its books."
        )));
    }
    let revisions: BTreeSet<i64> = dump.audit.iter().map(|a| a.revision).collect();
    if revisions.iter().next_back().copied().unwrap_or(0) != dump.revision {
        return Err(refuse(format!(
            "The ledger {name}'s history doesn't end at its revision."
        )));
    }
    if let Some(max) = dump.transactions.iter().map(|(t, _, _)| t.id).max() {
        if max >= dump.next_transaction_id {
            return Err(refuse(format!("The ledger {name} reuses transaction ids.")));
        }
    }
    let failures = kabacus_ledger::check(&live);
    if let Some(first) = failures.first() {
        return Err(refuse(format!(
            "The ledger {name} doesn't pass the checks: {}",
            first.message()
        )));
    }
    Ok(())
}
