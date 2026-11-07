// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The books database on SQLite, which is also the `.kabacus` file
//! format: its schema and versioned migrations. The schema holds only
//! declarative integrity rules; amounts are decimal text and all
//! arithmetic happens in Rust.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// Migrations in order; entry `i` takes the schema from version `i` to
/// `i + 1`, recorded in `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_initial.sql"),
    include_str!("migrations/0002_kabacus_file.sql"),
];

/// `PRAGMA application_id` of a `.kabacus` file: ASCII `KBCS`.
pub const APPLICATION_ID: i32 = 0x4B42_4353;

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("the database schema is version {found}, newer than the {known} this server knows")]
    Newer { found: usize, known: usize },
    #[error("{0} isn't a Kabacus file")]
    NotKabacus(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// The schema version this build creates.
pub fn latest_version() -> usize {
    MIGRATIONS.len()
}

/// Turns on foreign keys and applies every migration the database lacks.
pub fn migrate(conn: &mut Connection) -> Result<(), SchemaError> {
    migrate_with(conn, MIGRATIONS)
}

/// Applies the migrations up to `version` only.
pub fn migrate_to(conn: &mut Connection, version: usize) -> Result<(), SchemaError> {
    migrate_with(conn, &MIGRATIONS[..version.min(MIGRATIONS.len())])
}

/// All pending migrations run in one transaction, so a failure part-way
/// leaves the schema as it was.
fn migrate_with(conn: &mut Connection, migrations: &[&str]) -> Result<(), SchemaError> {
    conn.pragma_update(None, "foreign_keys", true)?;
    let found = schema_version(conn)?;
    if found > migrations.len() {
        return Err(SchemaError::Newer {
            found,
            known: migrations.len(),
        });
    }
    if found == migrations.len() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    for (index, sql) in migrations.iter().enumerate().skip(found) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index + 1)?;
    }
    tx.commit()?;
    Ok(())
}

fn schema_version(conn: &Connection) -> rusqlite::Result<usize> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

pub fn application_id(conn: &Connection) -> rusqlite::Result<i32> {
    conn.pragma_query_value(None, "application_id", |row| row.get(0))
}

/// A file is Kabacus's when it carries the application id, is still at
/// the first schema (before the id was set), or is empty.
#[cfg(not(target_arch = "wasm32"))]
fn check_identity(conn: &Connection, path: &Path) -> Result<(), SchemaError> {
    let version = schema_version(conn)?;
    let id = application_id(conn)?;
    let empty: i64 = conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
    let ours = id == APPLICATION_ID || (version == 1 && id == 0) || (version == 0 && empty == 0);
    if ours {
        Ok(())
    } else {
        Err(SchemaError::NotKabacus(path.display().to_string()))
    }
}

/// Opens the database file, creating it if missing, and brings it to the
/// current schema. An existing database that needs migrating is first
/// copied to `<file>.v<N>.bak`, `N` being the version it was at; one with
/// a newer schema than this build knows, or a SQLite file that isn't a
/// `.kabacus` file, is refused without being changed.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_file(path: &Path) -> Result<Connection, SchemaError> {
    open_file_with(path, MIGRATIONS)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file_with(path: &Path, migrations: &[&str]) -> Result<Connection, SchemaError> {
    let mut conn = Connection::open(path)?;
    check_identity(&conn, path)?;
    let found = schema_version(&conn)?;
    if found > migrations.len() {
        return Err(SchemaError::Newer {
            found,
            known: migrations.len(),
        });
    }
    // A fresh file has nothing worth keeping.
    if found > 0 && found < migrations.len() {
        let mut backup = path.as_os_str().to_owned();
        backup.push(format!(".v{found}.bak"));
        let backup = PathBuf::from(backup);
        match std::fs::remove_file(&backup) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)).into()),
        }
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
    }
    migrate_with(&mut conn, migrations)?;
    Ok(conn)
}

/// Opens an in-memory database with the current schema.
pub fn open_in_memory() -> Result<Connection, SchemaError> {
    let mut conn = Connection::open_in_memory()?;
    migrate(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
