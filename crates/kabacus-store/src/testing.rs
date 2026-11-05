// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Throwaway PostgreSQL databases for tests (the `testing` feature).
//!
//! `KABACUS_TEST_POSTGRES` names a server to use, as a connection string
//! for a role that may create roles and databases. Without it, a private
//! cluster is started from the `initdb` and `postgres` found on the `PATH`
//! or in the usual install directories, and stopped when the tests end.
//! With neither, Postgres tests are skipped, unless
//! `KABACUS_REQUIRE_POSTGRES` is set.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The non-superuser role the stores connect as, so row-level security
/// applies to them.
const ROLE: &str = "kabacus_test";

struct Server {
    admin: String,
    host: String,
    port: Option<String>,
    _dir: Option<tempfile::TempDir>,
}

fn find(binary: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(binary);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for base in ["/usr/lib/postgresql", "/usr/lib64/postgresql", "/usr/lib"] {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("bin").join(binary))
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        if let Some(last) = found.pop() {
            return Some(last);
        }
    }
    None
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

fn start_cluster() -> Option<Server> {
    if is_root() {
        return None;
    }
    let initdb = find("initdb")?;
    let postgres = find("postgres")?;
    let dir = tempfile::Builder::new().prefix("kpg").tempdir().ok()?;
    let data = dir.path().join("data");
    let ok = Command::new(&initdb)
        .args([
            "-U",
            "postgres",
            "--auth=trust",
            "-E",
            "UTF8",
            "--locale=C",
            "-D",
        ])
        .arg(&data)
        .output()
        .ok()?
        .status
        .success();
    if !ok {
        return None;
    }
    // Stopped by a watchdog when this test process ends.
    let script = format!(
        "\"$1\" -D \"$2\" -k \"$3\" -c listen_addresses= -F -c max_connections=200 >\"$3/log\" 2>&1 & pg=$!; \
         while kill -0 {} 2>/dev/null && kill -0 $pg 2>/dev/null; do sleep 1; done; kill $pg 2>/dev/null; wait $pg; rm -rf \"$3\"",
        std::process::id()
    );
    Command::new("sh")
        .args(["-c", &script, "sh"])
        .arg(&postgres)
        .arg(&data)
        .arg(dir.path())
        .spawn()
        .ok()?;
    let host = dir.path().display().to_string();
    let admin = format!("host={host} user=postgres dbname=postgres");
    for _ in 0..100 {
        if dir.path().join(".s.PGSQL.5432").exists() {
            std::thread::sleep(std::time::Duration::from_millis(200));
            return Some(Server {
                admin,
                host,
                port: None,
                _dir: Some(dir),
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

fn server() -> Option<&'static Server> {
    static SERVER: OnceLock<Option<Server>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            if let Ok(admin) = std::env::var("KABACUS_TEST_POSTGRES") {
                let config: tokio_postgres::Config = admin.parse().expect("KABACUS_TEST_POSTGRES");
                let host = match config.get_hosts().first() {
                    Some(tokio_postgres::config::Host::Unix(p)) => p.display().to_string(),
                    Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
                    None => "localhost".into(),
                };
                let port = config.get_ports().first().map(u16::to_string);
                return Some(Server {
                    admin,
                    host,
                    port,
                    _dir: None,
                });
            }
            start_cluster()
        })
        .as_ref()
}

/// A connection string for a new, empty database owned by a role row-level
/// security applies to; `None` when no server is available.
pub async fn database() -> Option<String> {
    let Some(server) = server() else {
        if std::env::var_os("KABACUS_REQUIRE_POSTGRES").is_some() {
            panic!("PostgreSQL is required (KABACUS_REQUIRE_POSTGRES) but none can be started");
        }
        eprintln!("no PostgreSQL to test against: skipping");
        return None;
    };
    let (client, connection) = tokio_postgres::connect(&server.admin, tokio_postgres::NoTls)
        .await
        .expect("connect to the test server");
    tokio::spawn(connection);
    let exists = client
        .query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&ROLE])
        .await
        .unwrap()
        .is_some();
    if !exists {
        let _ = client
            .batch_execute(&format!("CREATE ROLE {ROLE} LOGIN NOSUPERUSER"))
            .await;
    }
    let name = format!("kabacus_{}", uuid_like());
    client
        .batch_execute(&format!("CREATE DATABASE {name} OWNER {ROLE}"))
        .await
        .expect("create a test database");
    let port = server
        .port
        .as_ref()
        .map(|p| format!(" port={p}"))
        .unwrap_or_default();
    Some(format!(
        "host={} user={ROLE} dbname={name}{port}",
        server.host
    ))
}

/// The admin connection string, for tests that look under the store.
pub fn admin() -> Option<String> {
    server().map(|s| s.admin.clone())
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "{}_{}_{}",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// `path` exists and holds a SQLite file.
pub fn is_sqlite(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|b| b.starts_with(b"SQLite format 3\0"))
}
