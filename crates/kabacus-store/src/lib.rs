// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus-store`: where the books are kept. The [`BookStore`] trait is
//! made of bookkeeping operations (load a ledger at a revision, commit a
//! change against an expected revision, history, users and memberships,
//! `.kabacus` export and import), with two implementations that behave
//! identically: [`SqliteStore`] for local mode and small self-hosted
//! servers, and `PostgresStore` for the hosted service.

pub mod file;
#[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
pub mod postgres;
pub mod rows;
pub mod sqlite;
#[cfg(all(
    feature = "testing",
    not(any(target_arch = "wasm32", target_os = "ios", target_os = "android"))
))]
pub mod testing;

#[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
pub use postgres::PostgresStore;
pub use rows::{ChangeSet, Row, RowChange, RowKey};
pub use sqlite::SqliteStore;

use kabacus_ledger::Book;
use serde::{Deserialize, Serialize};
use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

pub type LedgerId = i64;
pub type Revision = i64;
pub type UserId = i64;

/// Kabacus's roles on a ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Read,
    Write,
    Rewrite,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Read => "read",
            Role::Write => "write",
            Role::Rewrite => "rewrite",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "read" => Some(Role::Read),
            "write" => Some(Role::Write),
            "rewrite" => Some(Role::Rewrite),
            _ => None,
        }
    }
}

/// Who makes a write and what it is, as the audit log records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Who {
    pub user: UserId,
    /// `app`, `mcp` or the client.
    pub actor: String,
    /// `add`, `edit`, `delete`, ...
    pub kind: String,
    pub summary: String,
    /// The rest of who it was: client, session, issuer.
    pub details: serde_json::Value,
}

/// A ledger as loaded: its book, its revision and the id the next new
/// transaction takes.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub book: Book,
    pub revision: Revision,
    pub next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerInfo {
    pub id: LedgerId,
    pub name: String,
    pub revision: Revision,
    pub created_at: String,
}

/// One write, as history lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeInfo {
    pub revision: Revision,
    /// When, RFC 3339.
    pub at: String,
    /// The user's display name.
    pub user: String,
    pub actor: String,
    pub kind: String,
    pub summary: String,
    pub details: serde_json::Value,
    /// How many rows it changed.
    pub rows: usize,
}

/// One write with every row it changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRecord {
    pub info: ChangeInfo,
    pub changes: Vec<RowChange>,
}

/// What one audit row holds beside its columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditPayload {
    pub at: String,
    #[serde(default)]
    pub who: serde_json::Value,
    /// The row changed; none for the row that records a ledger's creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<RowChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the books changed since revision {expected}; they are at revision {current}")]
    Conflict {
        expected: Revision,
        current: Revision,
    },
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    /// A `.kabacus` file refused on import: one sentence naming the
    /// ledger and the reason.
    #[error("{0}")]
    File(String),
    #[error(transparent)]
    Row(#[from] rows::RowError),
    #[error(transparent)]
    Schema(#[from] sqlite::schema::SchemaError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    #[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
    Postgres(#[from] tokio_postgres::Error),
    #[error("the database connection pool: {0}")]
    Pool(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// The current time as the store records it.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A ledger's every stored row, users reduced to display names: what a
/// `.kabacus` file carries and what both stores read and write it from.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerDump {
    pub name: String,
    pub created_at: String,
    pub revision: Revision,
    pub next_transaction_id: u64,
    pub accounts: Vec<rows::AccountRow>,
    pub commodities: Vec<rows::CommodityRow>,
    /// With the revisions that created and last changed each.
    pub transactions: Vec<(rows::TxRow, Revision, Revision)>,
    pub audit: Vec<AuditRow>,
    /// The users the audit log points at: id in the dump, display name.
    pub users: Vec<(UserId, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub revision: Revision,
    pub n: i64,
    pub user: UserId,
    pub actor: String,
    pub kind: String,
    pub summary: String,
    pub payload: String,
}

/// The bookkeeping operations every store provides. A shared test suite
/// runs against each store; they must behave identically.
pub trait BookStore: Send + Sync {
    /// A new, empty ledger at revision 1, owned by `owner` (`rewrite`).
    fn create_ledger(
        &self,
        name: &str,
        owner: UserId,
    ) -> impl Future<Output = Result<LedgerId>> + Send;

    fn ledgers(&self) -> impl Future<Output = Result<Vec<LedgerInfo>>> + Send;

    /// The ledger's current revision.
    fn revision(&self, ledger: LedgerId) -> impl Future<Output = Result<Revision>> + Send;

    /// The ledger now, or as it was at revision `at`.
    fn load(
        &self,
        ledger: LedgerId,
        at: Option<Revision>,
    ) -> impl Future<Output = Result<Loaded>> + Send;

    /// Apply `changes` as one write, if the ledger is still at `expected`;
    /// otherwise [`StoreError::Conflict`]. Returns the new revision.
    fn commit(
        &self,
        ledger: LedgerId,
        expected: Revision,
        changes: ChangeSet,
        who: &Who,
    ) -> impl Future<Output = Result<Revision>> + Send;

    /// The latest `limit` writes, newest first.
    fn history(
        &self,
        ledger: LedgerId,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<ChangeInfo>>> + Send;

    /// The writes that added, changed or removed transaction `id`, newest
    /// first.
    fn transaction_history(
        &self,
        ledger: LedgerId,
        id: u64,
    ) -> impl Future<Output = Result<Vec<ChangeInfo>>> + Send;

    /// What revision `revision` changed.
    fn change(
        &self,
        ledger: LedgerId,
        revision: Revision,
    ) -> impl Future<Output = Result<ChangeRecord>> + Send;

    /// The user an external sign-in belongs to, made on first sign-in.
    fn user_for_identity(
        &self,
        provider: &str,
        subject: &str,
        display_name: &str,
    ) -> impl Future<Output = Result<UserId>> + Send;

    /// A user without a sign-in of their own (single-user mode), made if
    /// there isn't one by that name.
    fn local_user(&self, display_name: &str) -> impl Future<Output = Result<UserId>> + Send;

    fn memberships(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<Vec<(LedgerId, Role)>>> + Send;

    fn set_membership(
        &self,
        ledger: LedgerId,
        user: UserId,
        role: Option<Role>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Every stored row of `ledger`, read consistently.
    fn dump(&self, ledger: LedgerId) -> impl Future<Output = Result<LedgerDump>> + Send;

    /// `dump` as a new ledger owned by `owner`, keeping its name, revision,
    /// transaction ids and audit log.
    fn restore(
        &self,
        dump: LedgerDump,
        owner: UserId,
    ) -> impl Future<Output = Result<LedgerId>> + Send;

    /// Add every row of `book` to the empty ledger `ledger` as one write.
    fn import(
        &self,
        ledger: LedgerId,
        book: &Book,
        who: &Who,
    ) -> impl Future<Output = Result<Revision>> + Send {
        async move {
            let expected = self.revision(ledger).await?;
            let loaded = self.load(ledger, None).await?;
            if !loaded.book.transactions.is_empty()
                || !loaded.book.accounts.is_empty()
                || !loaded.book.commodities.is_empty()
            {
                return Err(StoreError::Invalid(
                    "books are only imported into an empty ledger".into(),
                ));
            }
            self.commit(ledger, expected, ChangeSet::adding(book), who)
                .await
        }
    }

    fn export_bytes(&self, ledgers: &[LedgerId]) -> impl Future<Output = Result<Vec<u8>>> + Send {
        async move {
            let mut dumps = Vec::new();
            for &ledger in ledgers {
                dumps.push(self.dump(ledger).await?);
            }
            file::write_bytes(&dumps)
        }
    }

    fn import_bytes(
        &self,
        bytes: &[u8],
        owner: UserId,
    ) -> impl Future<Output = Result<Vec<LedgerId>>> + Send {
        async move {
            let dumps = file::read_bytes(bytes)?;
            let mut ids = Vec::new();
            for dump in dumps {
                ids.push(self.restore(dump, owner).await?);
            }
            Ok(ids)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_file(
        &self,
        ledgers: &[LedgerId],
        to: &Path,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let mut dumps = Vec::new();
            for &ledger in ledgers {
                dumps.push(self.dump(ledger).await?);
            }
            let to = to.to_path_buf();
            tokio::task::spawn_blocking(move || file::write(&dumps, &to))
                .await
                .map_err(|e| StoreError::Invalid(e.to_string()))?
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn import_file(
        &self,
        from: &Path,
        owner: UserId,
    ) -> impl Future<Output = Result<Vec<LedgerId>>> + Send {
        async move {
            let from = from.to_path_buf();
            let dumps = tokio::task::spawn_blocking(move || file::read(&from))
                .await
                .map_err(|e| StoreError::Invalid(e.to_string()))??;
            let mut ids = Vec::new();
            for dump in dumps {
                ids.push(self.restore(dump, owner).await?);
            }
            Ok(ids)
        }
    }
}

/// Either store, chosen by configuration.
pub enum AnyStore {
    Sqlite(SqliteStore),
    #[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
    Postgres(PostgresStore),
}

macro_rules! delegate {
    ($self:ident, $s:ident => $e:expr) => {
        match $self {
            AnyStore::Sqlite($s) => $e.await,
            #[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
            AnyStore::Postgres($s) => $e.await,
        }
    };
}

impl BookStore for AnyStore {
    async fn create_ledger(&self, name: &str, owner: UserId) -> Result<LedgerId> {
        delegate!(self, s => s.create_ledger(name, owner))
    }
    async fn ledgers(&self) -> Result<Vec<LedgerInfo>> {
        delegate!(self, s => s.ledgers())
    }
    async fn revision(&self, ledger: LedgerId) -> Result<Revision> {
        delegate!(self, s => s.revision(ledger))
    }
    async fn load(&self, ledger: LedgerId, at: Option<Revision>) -> Result<Loaded> {
        delegate!(self, s => s.load(ledger, at))
    }
    async fn commit(
        &self,
        ledger: LedgerId,
        expected: Revision,
        changes: ChangeSet,
        who: &Who,
    ) -> Result<Revision> {
        delegate!(self, s => s.commit(ledger, expected, changes, who))
    }
    async fn history(&self, ledger: LedgerId, limit: usize) -> Result<Vec<ChangeInfo>> {
        delegate!(self, s => s.history(ledger, limit))
    }
    async fn transaction_history(&self, ledger: LedgerId, id: u64) -> Result<Vec<ChangeInfo>> {
        delegate!(self, s => s.transaction_history(ledger, id))
    }
    async fn change(&self, ledger: LedgerId, revision: Revision) -> Result<ChangeRecord> {
        delegate!(self, s => s.change(ledger, revision))
    }
    async fn user_for_identity(
        &self,
        provider: &str,
        subject: &str,
        display_name: &str,
    ) -> Result<UserId> {
        delegate!(self, s => s.user_for_identity(provider, subject, display_name))
    }
    async fn local_user(&self, display_name: &str) -> Result<UserId> {
        delegate!(self, s => s.local_user(display_name))
    }
    async fn memberships(&self, user: UserId) -> Result<Vec<(LedgerId, Role)>> {
        delegate!(self, s => s.memberships(user))
    }
    async fn set_membership(
        &self,
        ledger: LedgerId,
        user: UserId,
        role: Option<Role>,
    ) -> Result<()> {
        delegate!(self, s => s.set_membership(ledger, user, role))
    }
    async fn dump(&self, ledger: LedgerId) -> Result<LedgerDump> {
        delegate!(self, s => s.dump(ledger))
    }
    async fn restore(&self, dump: LedgerDump, owner: UserId) -> Result<LedgerId> {
        delegate!(self, s => s.restore(dump, owner))
    }
}

/// The book `before` becomes with `changes`, and the strict check's
/// failures on it. Only what the changes can affect is checked: on a book
/// that passed before, the full check's verdict.
pub fn checked(
    before: &Book,
    changes: &ChangeSet,
) -> Result<(Book, Vec<kabacus_ledger::CheckFailure>)> {
    let mut after = before.clone();
    let mut change = kabacus_ledger::Change::default();
    for c in &changes.changes {
        rows::apply(&mut after, c)?;
        match (&c.before, &c.after) {
            (Some(Row::Transaction(t)), _) => {
                change.before.push(rows::transaction(t)?);
                if let Some(Row::Transaction(n)) = &c.after {
                    change.after.insert(n.id);
                }
            }
            (None, Some(Row::Transaction(t))) => {
                change.after.insert(t.id);
            }
            (Some(Row::Account(a)), None) => {
                change.undeclared_accounts.insert(a.name.clone());
            }
            (Some(Row::Commodity(c)), None) => {
                change.undeclared_commodities.insert(c.symbol.clone());
            }
            _ => {}
        }
    }
    let failures = kabacus_ledger::check_change(&after, &change);
    Ok((after, failures))
}

/// Every row of a ledger as a book, for checking a dump.
pub fn book_of(dump: &LedgerDump) -> Result<Book> {
    let mut book = Book::default();
    for a in &dump.accounts {
        book.accounts.insert(a.name.clone(), rows::account(a));
    }
    for c in &dump.commodities {
        book.commodities
            .insert(c.symbol.clone(), rows::commodity(c)?);
    }
    for (t, _, _) in &dump.transactions {
        book.transactions.push(rows::transaction(t)?);
    }
    book.transactions.sort_by_key(|t| t.id);
    Ok(book)
}

/// The book the audit log of `dump` builds from nothing.
pub fn replay(dump: &LedgerDump) -> Result<Book> {
    let mut book = Book::default();
    let mut audit: Vec<&AuditRow> = dump.audit.iter().collect();
    audit.sort_by_key(|a| (a.revision, a.n));
    for row in audit {
        let payload: AuditPayload = serde_json::from_str(&row.payload)?;
        if let Some(change) = payload.change {
            rows::apply(&mut book, &change)?;
        }
    }
    Ok(book)
}
