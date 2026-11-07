// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The store on PostgreSQL, for the hosted service: pooled connections,
//! row-level security keyed on the ledger each transaction sets with
//! `SET LOCAL`, and the same tables and behaviour as the SQLite store.
//! Implements the engine implementation specification, sections 1.2 to
//! 1.4 and 2.4.

use crate::rows::{AccountRow, CommodityRow, PostingRow, Row, RowChange, TxRow};
use crate::sqlite::next_id_after;
use crate::{
    now, AuditPayload, AuditRow, BookStore, ChangeInfo, ChangeRecord, ChangeSet, LedgerDump,
    LedgerId, LedgerInfo, Loaded, Result, Revision, Role, StoreError, UserId, Who,
};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use kabacus_ledger::Book;
use std::collections::BTreeMap;
use tokio_postgres::{IsolationLevel, NoTls, Transaction as PgTx};

/// Migrations in order; entry `i` takes the schema from version `i` to
/// `i + 1`, recorded in `kabacus_schema`.
const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_initial.sql")];

/// Serializes migrations between replicas starting together.
const MIGRATION_LOCK: i64 = 0x4B42_4353;

#[derive(Clone)]
pub struct PostgresStore {
    pool: Pool,
}

fn pool_error(e: impl std::fmt::Display) -> StoreError {
    StoreError::Pool(e.to_string())
}

/// Which rows a transaction may see.
enum Scope {
    Ledger(LedgerId),
    All,
}

impl PostgresStore {
    /// Connect to the database at `url` (a `postgresql://` URL or
    /// key=value string) and bring its schema up to date.
    pub async fn connect(url: &str, max_connections: usize) -> Result<PostgresStore> {
        let config: tokio_postgres::Config = url.parse()?;
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(max_connections.max(1))
            .build()
            .map_err(pool_error)?;
        let store = PostgresStore { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        let mut client = self.pool.get().await.map_err(pool_error)?;
        let tx = client.transaction().await?;
        tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])
            .await?;
        tx.batch_execute("CREATE TABLE IF NOT EXISTS kabacus_schema (version integer NOT NULL)")
            .await?;
        let found: i32 = match tx
            .query_opt("SELECT version FROM kabacus_schema", &[])
            .await?
        {
            Some(row) => row.get(0),
            None => {
                tx.execute("INSERT INTO kabacus_schema (version) VALUES (0)", &[])
                    .await?;
                0
            }
        };
        let found = found as usize;
        if found > MIGRATIONS.len() {
            return Err(crate::sqlite::schema::SchemaError::Newer {
                found,
                known: MIGRATIONS.len(),
            }
            .into());
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(found) {
            tx.batch_execute(sql).await?;
            tx.execute(
                "UPDATE kabacus_schema SET version = $1",
                &[&((i + 1) as i32)],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Run `f` in one database transaction that sees `scope`.
    async fn with<T, F>(&self, scope: Scope, isolation: IsolationLevel, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(
                &'a PgTx<'a>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>,
            > + Send,
        T: Send,
    {
        let mut client = self.pool.get().await.map_err(pool_error)?;
        let tx = client
            .build_transaction()
            .isolation_level(isolation)
            .start()
            .await?;
        match scope {
            Scope::Ledger(ledger) => {
                tx.execute(
                    "SELECT set_config('kabacus.ledger', $1, true)",
                    &[&ledger.to_string()],
                )
                .await?
            }
            Scope::All => {
                tx.execute("SELECT set_config('kabacus.scope', 'all', true)", &[])
                    .await?
            }
        };
        let out = f(&tx).await?;
        tx.commit().await?;
        Ok(out)
    }
}

fn not_found(ledger: LedgerId) -> StoreError {
    StoreError::NotFound(format!("ledger {ledger}"))
}

async fn ledger_revision(tx: &PgTx<'_>, ledger: LedgerId) -> Result<(Revision, u64)> {
    let row = tx
        .query_opt(
            "SELECT revision, next_transaction_id FROM ledgers WHERE id = $1",
            &[&ledger],
        )
        .await?
        .ok_or_else(|| not_found(ledger))?;
    Ok((row.get(0), row.get::<_, i64>(1) as u64))
}

async fn read_accounts(tx: &PgTx<'_>, ledger: LedgerId) -> Result<Vec<AccountRow>> {
    Ok(tx
        .query(
            "SELECT name, declared_tags FROM accounts WHERE ledger_id = $1 ORDER BY name",
            &[&ledger],
        )
        .await?
        .iter()
        .map(|r| AccountRow {
            name: r.get(0),
            declared_tags: r.get(1),
        })
        .collect())
}

async fn read_commodities(tx: &PgTx<'_>, ledger: LedgerId) -> Result<Vec<CommodityRow>> {
    Ok(tx
        .query(
            "SELECT symbol, style_side, style_space, decimal_mark, group_mark, group_sizes, places
             FROM commodities WHERE ledger_id = $1 ORDER BY symbol",
            &[&ledger],
        )
        .await?
        .iter()
        .map(|r| CommodityRow {
            symbol: r.get(0),
            style_side: r.get(1),
            style_space: r.get(2),
            decimal_mark: r.get(3),
            group_mark: r.get(4),
            group_sizes: r.get(5),
            places: r.get::<_, Option<i32>>(6).map(|p| p as u32),
        })
        .collect())
}

async fn read_transactions(
    tx: &PgTx<'_>,
    ledger: LedgerId,
) -> Result<Vec<(TxRow, Revision, Revision)>> {
    let mut txs: Vec<(TxRow, Revision, Revision)> = tx
        .query(
            "SELECT id, to_char(date, 'YYYY-MM-DD'), status, code, description, comment,
                    revision_created, revision_changed
             FROM transactions WHERE ledger_id = $1 ORDER BY id",
            &[&ledger],
        )
        .await?
        .iter()
        .map(|r| {
            (
                TxRow {
                    id: r.get::<_, i64>(0) as u64,
                    date: r.get(1),
                    status: r.get(2),
                    code: r.get(3),
                    description: r.get(4),
                    comment: r.get(5),
                    postings: Vec::new(),
                },
                r.get(6),
                r.get(7),
            )
        })
        .collect();
    let index: BTreeMap<u64, usize> = txs
        .iter()
        .enumerate()
        .map(|(i, (t, _, _))| (t.id, i))
        .collect();
    let postings = tx
        .query(
            "SELECT transaction_id, account, kind, status, mantissa::text, places, commodity,
                    cost_kind, cost_mantissa::text, cost_places, cost_commodity,
                    assert_op, assert_mantissa::text, assert_places, assert_commodity, comment
             FROM postings WHERE ledger_id = $1 ORDER BY transaction_id, position",
            &[&ledger],
        )
        .await?;
    for r in postings {
        let id = r.get::<_, i64>(0) as u64;
        let i = index
            .get(&id)
            .ok_or_else(|| StoreError::Invalid(format!("a posting of missing transaction {id}")))?;
        txs[*i].0.postings.push(PostingRow {
            account: r.get(1),
            kind: r.get(2),
            status: r.get(3),
            mantissa: r.get(4),
            places: r.get::<_, i16>(5) as u32,
            commodity: r.get(6),
            cost_kind: r.get(7),
            cost_mantissa: r.get(8),
            cost_places: r.get::<_, Option<i16>>(9).map(|p| p as u32),
            cost_commodity: r.get(10),
            assert_op: r.get(11),
            assert_mantissa: r.get(12),
            assert_places: r.get::<_, Option<i16>>(13).map(|p| p as u32),
            assert_commodity: r.get(14),
            comment: r.get(15),
        });
    }
    Ok(txs)
}

fn audit_row(r: &tokio_postgres::Row) -> AuditRow {
    AuditRow {
        revision: r.get(0),
        n: r.get::<_, i32>(1) as i64,
        user: r.get(2),
        actor: r.get(3),
        kind: r.get(4),
        summary: r.get(5),
        payload: r.get(6),
    }
}

async fn read_audit(tx: &PgTx<'_>, ledger: LedgerId, after: Revision) -> Result<Vec<AuditRow>> {
    Ok(tx
        .query(
            "SELECT revision, n, user_id, actor, kind, summary, payload::text FROM audit_log
             WHERE ledger_id = $1 AND revision > $2 ORDER BY revision, n",
            &[&ledger, &after],
        )
        .await?
        .iter()
        .map(audit_row)
        .collect())
}

fn book_from(
    accounts: &[AccountRow],
    commodities: &[CommodityRow],
    txs: &[(TxRow, Revision, Revision)],
) -> Result<Book> {
    let mut book = Book::default();
    for a in accounts {
        book.accounts
            .insert(a.name.clone(), crate::rows::account(a));
    }
    for c in commodities {
        book.commodities
            .insert(c.symbol.clone(), crate::rows::commodity(c)?);
    }
    for (t, _, _) in txs {
        book.transactions.push(crate::rows::transaction(t)?);
    }
    Ok(book)
}

async fn insert_postings(tx: &PgTx<'_>, ledger: LedgerId, t: &TxRow) -> Result<()> {
    let stmt = tx
        .prepare(
            "INSERT INTO postings (ledger_id, transaction_id, position, account, kind, status,
                mantissa, places, commodity, cost_kind, cost_mantissa, cost_places, cost_commodity,
                assert_op, assert_mantissa, assert_places, assert_commodity, comment)
             VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8, $9, $10, $11::text::numeric,
                $12, $13, $14, $15::text::numeric, $16, $17, $18)",
        )
        .await?;
    for (position, p) in t.postings.iter().enumerate() {
        let places = p.places as i16;
        let cost_places = p.cost_places.map(|v| v as i16);
        let assert_places = p.assert_places.map(|v| v as i16);
        tx.execute(
            &stmt,
            &[
                &ledger,
                &(t.id as i64),
                &(position as i32),
                &p.account,
                &p.kind,
                &p.status,
                &p.mantissa,
                &places,
                &p.commodity,
                &p.cost_kind,
                &p.cost_mantissa,
                &cost_places,
                &p.cost_commodity,
                &p.assert_op,
                &p.assert_mantissa,
                &assert_places,
                &p.assert_commodity,
                &p.comment,
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_transaction(
    tx: &PgTx<'_>,
    ledger: LedgerId,
    t: &TxRow,
    created: Revision,
    changed: Revision,
) -> Result<()> {
    tx.execute(
        "INSERT INTO transactions (ledger_id, id, date, status, code, description, comment,
            revision_created, revision_changed)
         VALUES ($1, $2, $3::text::date, $4, $5, $6, $7, $8, $9)",
        &[
            &ledger,
            &(t.id as i64),
            &t.date,
            &t.status,
            &t.code,
            &t.description,
            &t.comment,
            &created,
            &changed,
        ],
    )
    .await?;
    insert_postings(tx, ledger, t).await
}

async fn upsert_commodity(
    tx: &PgTx<'_>,
    ledger: LedgerId,
    c: &CommodityRow,
    insert: bool,
) -> Result<()> {
    let sql = if insert {
        "INSERT INTO commodities (ledger_id, symbol, style_side, style_space, decimal_mark,
            group_mark, group_sizes, places) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    } else {
        "UPDATE commodities SET style_side = $3, style_space = $4, decimal_mark = $5,
            group_mark = $6, group_sizes = $7, places = $8 WHERE ledger_id = $1 AND symbol = $2"
    };
    let places = c.places.map(|p| p as i32);
    tx.execute(
        sql,
        &[
            &ledger,
            &c.symbol,
            &c.style_side,
            &c.style_space,
            &c.decimal_mark,
            &c.group_mark,
            &c.group_sizes,
            &places,
        ],
    )
    .await?;
    Ok(())
}

async fn apply_change(
    tx: &PgTx<'_>,
    ledger: LedgerId,
    rev: Revision,
    change: &RowChange,
) -> Result<()> {
    match (&change.before, &change.after) {
        (None, Some(Row::Transaction(t))) => insert_transaction(tx, ledger, t, rev, rev).await?,
        (Some(Row::Transaction(_)), Some(Row::Transaction(t))) => {
            let n = tx
                .execute(
                    "UPDATE transactions SET date = $3::text::date, status = $4, code = $5,
                        description = $6, comment = $7, revision_changed = $8
                     WHERE ledger_id = $1 AND id = $2",
                    &[
                        &ledger,
                        &(t.id as i64),
                        &t.date,
                        &t.status,
                        &t.code,
                        &t.description,
                        &t.comment,
                        &rev,
                    ],
                )
                .await?;
            if n != 1 {
                return Err(StoreError::NotFound(format!("transaction {}", t.id)));
            }
            tx.execute(
                "DELETE FROM postings WHERE ledger_id = $1 AND transaction_id = $2",
                &[&ledger, &(t.id as i64)],
            )
            .await?;
            insert_postings(tx, ledger, t).await?;
        }
        (Some(Row::Transaction(t)), None) => {
            let n = tx
                .execute(
                    "DELETE FROM transactions WHERE ledger_id = $1 AND id = $2",
                    &[&ledger, &(t.id as i64)],
                )
                .await?;
            if n != 1 {
                return Err(StoreError::NotFound(format!("transaction {}", t.id)));
            }
        }
        (None, Some(Row::Account(a))) => {
            tx.execute(
                "INSERT INTO accounts (ledger_id, name, declared_tags) VALUES ($1, $2, $3)",
                &[&ledger, &a.name, &a.declared_tags],
            )
            .await?;
        }
        (Some(Row::Account(_)), Some(Row::Account(a))) => {
            tx.execute(
                "UPDATE accounts SET declared_tags = $3 WHERE ledger_id = $1 AND name = $2",
                &[&ledger, &a.name, &a.declared_tags],
            )
            .await?;
        }
        (Some(Row::Account(a)), None) => {
            tx.execute(
                "DELETE FROM accounts WHERE ledger_id = $1 AND name = $2",
                &[&ledger, &a.name],
            )
            .await?;
        }
        (None, Some(Row::Commodity(c))) => upsert_commodity(tx, ledger, c, true).await?,
        (Some(Row::Commodity(_)), Some(Row::Commodity(c))) => {
            upsert_commodity(tx, ledger, c, false).await?
        }
        (Some(Row::Commodity(c)), None) => {
            tx.execute(
                "DELETE FROM commodities WHERE ledger_id = $1 AND symbol = $2",
                &[&ledger, &c.symbol],
            )
            .await?;
        }
        _ => {
            return Err(StoreError::Invalid(
                "a change must keep one row of one table".into(),
            ))
        }
    }
    Ok(())
}

async fn insert_audit(tx: &PgTx<'_>, ledger: LedgerId, row: &AuditRow) -> Result<()> {
    tx.execute(
        "INSERT INTO audit_log (ledger_id, revision, n, user_id, actor, kind, summary, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::json)",
        &[
            &ledger,
            &row.revision,
            &(row.n as i32),
            &row.user,
            &row.actor,
            &row.kind,
            &row.summary,
            &row.payload,
        ],
    )
    .await?;
    Ok(())
}

fn change_info(rows: &[(AuditRow, String)]) -> Result<ChangeInfo> {
    let (first, user) = &rows[0];
    let payload: AuditPayload = serde_json::from_str(&first.payload)?;
    Ok(ChangeInfo {
        revision: first.revision,
        at: payload.at,
        user: user.clone(),
        actor: first.actor.clone(),
        kind: first.kind.clone(),
        summary: first.summary.clone(),
        details: payload.who,
        rows: rows
            .iter()
            .filter(|(r, _)| {
                serde_json::from_str::<AuditPayload>(&r.payload).is_ok_and(|p| p.change.is_some())
            })
            .count(),
    })
}

async fn audit_with_users(
    tx: &PgTx<'_>,
    ledger: LedgerId,
    from: Revision,
    to: Revision,
) -> Result<Vec<(AuditRow, String)>> {
    Ok(tx
        .query(
            "SELECT a.revision, a.n, a.user_id, a.actor, a.kind, a.summary, a.payload::text,
                    u.display_name
             FROM audit_log a JOIN users u ON u.id = a.user_id
             WHERE a.ledger_id = $1 AND a.revision BETWEEN $2 AND $3
             ORDER BY a.revision DESC, a.n",
            &[&ledger, &from, &to],
        )
        .await?
        .iter()
        .map(|r| (audit_row(r), r.get(7)))
        .collect())
}

fn group_by_revision(rows: Vec<(AuditRow, String)>) -> Vec<Vec<(AuditRow, String)>> {
    let mut groups: Vec<Vec<(AuditRow, String)>> = Vec::new();
    for row in rows {
        match groups.last_mut() {
            Some(g) if g[0].0.revision == row.0.revision => g.push(row),
            _ => groups.push(vec![row]),
        }
    }
    groups
}

async fn dump(tx: &PgTx<'_>, ledger: LedgerId) -> Result<LedgerDump> {
    let row = tx
        .query_opt(
            "SELECT name, created_at, revision, next_transaction_id FROM ledgers WHERE id = $1",
            &[&ledger],
        )
        .await?
        .ok_or_else(|| not_found(ledger))?;
    let users = tx
        .query(
            "SELECT DISTINCT u.id, u.display_name FROM users u
             JOIN audit_log a ON a.user_id = u.id WHERE a.ledger_id = $1 ORDER BY u.id",
            &[&ledger],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    Ok(LedgerDump {
        name: row.get(0),
        created_at: row.get(1),
        revision: row.get(2),
        next_transaction_id: row.get::<_, i64>(3) as u64,
        accounts: read_accounts(tx, ledger).await?,
        commodities: read_commodities(tx, ledger).await?,
        transactions: read_transactions(tx, ledger).await?,
        audit: read_audit(tx, ledger, 0).await?,
        users,
    })
}

impl BookStore for PostgresStore {
    async fn create_ledger(&self, name: &str, owner: UserId) -> Result<LedgerId> {
        let name = name.to_string();
        self.with(Scope::All, IsolationLevel::ReadCommitted, move |tx| {
            Box::pin(async move {
                let at = now();
                let id: i64 = tx
                    .query_one(
                        "INSERT INTO ledgers (name, revision, created_at) VALUES ($1, 1, $2)
                         RETURNING id",
                        &[&name, &at],
                    )
                    .await?
                    .get(0);
                tx.execute(
                    "INSERT INTO memberships (ledger_id, user_id, role) VALUES ($1, $2, 'rewrite')",
                    &[&id, &owner],
                )
                .await?;
                let payload = AuditPayload {
                    at,
                    who: serde_json::Value::Null,
                    change: None,
                    ledger: Some(name.clone()),
                };
                insert_audit(
                    tx,
                    id,
                    &AuditRow {
                        revision: 1,
                        n: 0,
                        user: owner,
                        actor: "kabacus".into(),
                        kind: "create".into(),
                        summary: format!("create ledger: {name}"),
                        payload: serde_json::to_string(&payload)?,
                    },
                )
                .await?;
                Ok(id)
            })
        })
        .await
    }

    async fn ledgers(&self) -> Result<Vec<LedgerInfo>> {
        self.with(Scope::All, IsolationLevel::ReadCommitted, |tx| {
            Box::pin(async move {
                Ok(tx
                    .query(
                        "SELECT id, name, revision, created_at FROM ledgers ORDER BY id",
                        &[],
                    )
                    .await?
                    .iter()
                    .map(|r| LedgerInfo {
                        id: r.get(0),
                        name: r.get(1),
                        revision: r.get(2),
                        created_at: r.get(3),
                    })
                    .collect())
            })
        })
        .await
    }

    async fn revision(&self, ledger: LedgerId) -> Result<Revision> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::ReadCommitted,
            move |tx| Box::pin(async move { Ok(ledger_revision(tx, ledger).await?.0) }),
        )
        .await
    }

    async fn load(&self, ledger: LedgerId, at: Option<Revision>) -> Result<Loaded> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::RepeatableRead,
            move |tx| {
                Box::pin(async move {
                    let (revision, next_id) = ledger_revision(tx, ledger).await?;
                    let mut book = book_from(
                        &read_accounts(tx, ledger).await?,
                        &read_commodities(tx, ledger).await?,
                        &read_transactions(tx, ledger).await?,
                    )?;
                    let revision = match at {
                        Some(at) if at < revision => {
                            if at < 1 {
                                return Err(StoreError::NotFound(format!("revision {at}")));
                            }
                            for row in read_audit(tx, ledger, at).await?.iter().rev() {
                                let payload: AuditPayload = serde_json::from_str(&row.payload)?;
                                if let Some(change) = payload.change {
                                    crate::rows::apply(&mut book, &change.reversed())?;
                                }
                            }
                            at
                        }
                        Some(at) if at > revision => {
                            return Err(StoreError::NotFound(format!("revision {at}")));
                        }
                        _ => revision,
                    };
                    Ok(Loaded {
                        book,
                        revision,
                        next_id,
                    })
                })
            },
        )
        .await
    }

    async fn commit(
        &self,
        ledger: LedgerId,
        expected: Revision,
        changes: ChangeSet,
        who: &Who,
    ) -> Result<Revision> {
        if changes.is_empty() {
            return Err(StoreError::Invalid("a write must change something".into()));
        }
        let who = who.clone();
        self.with(Scope::Ledger(ledger), IsolationLevel::ReadCommitted, move |tx| {
            Box::pin(async move {
                let updated = tx
                    .execute(
                        "UPDATE ledgers SET revision = revision + 1 WHERE id = $1 AND revision = $2",
                        &[&ledger, &expected],
                    )
                    .await?;
                if updated == 0 {
                    let (current, _) = ledger_revision(tx, ledger).await?;
                    return Err(StoreError::Conflict { expected, current });
                }
                let rev = expected + 1;
                let (_, next) = ledger_revision(tx, ledger).await?;
                let next = next_id_after(next, &changes)?;
                let at = now();
                for (n, change) in changes.changes.iter().enumerate() {
                    apply_change(tx, ledger, rev, change).await?;
                    let payload = AuditPayload {
                        at: at.clone(),
                        who: who.details.clone(),
                        change: Some(change.clone()),
                        ledger: None,
                    };
                    insert_audit(
                        tx,
                        ledger,
                        &AuditRow {
                            revision: rev,
                            n: n as i64,
                            user: who.user,
                            actor: who.actor.clone(),
                            kind: who.kind.clone(),
                            summary: who.summary.clone(),
                            payload: serde_json::to_string(&payload)?,
                        },
                    )
                    .await?;
                }
                tx.execute(
                    "UPDATE ledgers SET next_transaction_id = $2 WHERE id = $1",
                    &[&ledger, &(next as i64)],
                )
                .await?;
                Ok(rev)
            })
        })
        .await
    }

    async fn history(&self, ledger: LedgerId, limit: usize) -> Result<Vec<ChangeInfo>> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::RepeatableRead,
            move |tx| {
                Box::pin(async move {
                    let (revision, _) = ledger_revision(tx, ledger).await?;
                    let from = (revision - limit as i64 + 1).max(1);
                    group_by_revision(audit_with_users(tx, ledger, from, revision).await?)
                        .iter()
                        .map(|g| change_info(g))
                        .collect()
                })
            },
        )
        .await
    }

    async fn transaction_history(&self, ledger: LedgerId, id: u64) -> Result<Vec<ChangeInfo>> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::RepeatableRead,
            move |tx| {
                Box::pin(async move {
                    ledger_revision(tx, ledger).await?;
                    let id = id.to_string();
                    let revisions: Vec<i64> = tx
                        .query(
                            "SELECT DISTINCT revision FROM audit_log WHERE ledger_id = $1 AND (
                            (payload->'change'->'before'->>'table' = 'transaction'
                             AND payload->'change'->'before'->>'id' = $2)
                         OR (payload->'change'->'after'->>'table' = 'transaction'
                             AND payload->'change'->'after'->>'id' = $2))
                         ORDER BY revision DESC",
                            &[&ledger, &id],
                        )
                        .await?
                        .iter()
                        .map(|r| r.get(0))
                        .collect();
                    let mut out = Vec::new();
                    for rev in revisions {
                        out.push(change_info(&audit_with_users(tx, ledger, rev, rev).await?)?);
                    }
                    Ok(out)
                })
            },
        )
        .await
    }

    async fn change(&self, ledger: LedgerId, revision: Revision) -> Result<ChangeRecord> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::RepeatableRead,
            move |tx| {
                Box::pin(async move {
                    ledger_revision(tx, ledger).await?;
                    let mut rows = audit_with_users(tx, ledger, revision, revision).await?;
                    if rows.is_empty() {
                        return Err(StoreError::NotFound(format!("revision {revision}")));
                    }
                    let info = change_info(&rows)?;
                    rows.sort_by_key(|(r, _)| r.n);
                    let mut changes = Vec::new();
                    for (row, _) in rows {
                        let payload: AuditPayload = serde_json::from_str(&row.payload)?;
                        changes.extend(payload.change);
                    }
                    Ok(ChangeRecord { info, changes })
                })
            },
        )
        .await
    }

    async fn user_for_identity(
        &self,
        provider: &str,
        subject: &str,
        display_name: &str,
    ) -> Result<UserId> {
        let (provider, subject, display_name) = (
            provider.to_string(),
            subject.to_string(),
            display_name.to_string(),
        );
        self.with(Scope::All, IsolationLevel::Serializable, move |tx| {
            Box::pin(async move {
                if let Some(row) = tx
                    .query_opt(
                        "SELECT user_id FROM identities WHERE provider = $1 AND subject = $2",
                        &[&provider, &subject],
                    )
                    .await?
                {
                    return Ok(row.get(0));
                }
                let id: i64 = tx
                    .query_one(
                        "INSERT INTO users (display_name, created_at) VALUES ($1, $2) RETURNING id",
                        &[&display_name, &now()],
                    )
                    .await?
                    .get(0);
                tx.execute(
                    "INSERT INTO identities (provider, subject, user_id) VALUES ($1, $2, $3)",
                    &[&provider, &subject, &id],
                )
                .await?;
                Ok(id)
            })
        })
        .await
    }

    async fn local_user(&self, display_name: &str) -> Result<UserId> {
        let display_name = display_name.to_string();
        self.with(Scope::All, IsolationLevel::Serializable, move |tx| {
            Box::pin(async move {
                if let Some(row) = tx
                    .query_opt(
                        "SELECT id FROM users WHERE display_name = $1 AND password_hash IS NULL
                         AND NOT EXISTS (SELECT 1 FROM identities i WHERE i.user_id = users.id)
                         ORDER BY id LIMIT 1",
                        &[&display_name],
                    )
                    .await?
                {
                    return Ok(row.get(0));
                }
                Ok(tx
                    .query_one(
                        "INSERT INTO users (display_name, created_at) VALUES ($1, $2) RETURNING id",
                        &[&display_name, &now()],
                    )
                    .await?
                    .get(0))
            })
        })
        .await
    }

    async fn memberships(&self, user: UserId) -> Result<Vec<(LedgerId, Role)>> {
        self.with(Scope::All, IsolationLevel::ReadCommitted, move |tx| {
            Box::pin(async move {
                Ok(tx
                    .query(
                        "SELECT ledger_id, role FROM memberships WHERE user_id = $1
                         ORDER BY ledger_id",
                        &[&user],
                    )
                    .await?
                    .iter()
                    .filter_map(|r| Role::parse(r.get(1)).map(|role| (r.get(0), role)))
                    .collect())
            })
        })
        .await
    }

    async fn set_membership(
        &self,
        ledger: LedgerId,
        user: UserId,
        role: Option<Role>,
    ) -> Result<()> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::ReadCommitted,
            move |tx| {
                Box::pin(async move {
                    match role {
                    Some(role) => {
                        tx.execute(
                            "INSERT INTO memberships (ledger_id, user_id, role) VALUES ($1, $2, $3)
                             ON CONFLICT (ledger_id, user_id) DO UPDATE SET role = excluded.role",
                            &[&ledger, &user, &role.as_str()],
                        )
                        .await?
                    }
                    None => {
                        tx.execute(
                            "DELETE FROM memberships WHERE ledger_id = $1 AND user_id = $2",
                            &[&ledger, &user],
                        )
                        .await?
                    }
                };
                    Ok(())
                })
            },
        )
        .await
    }

    async fn dump(&self, ledger: LedgerId) -> Result<LedgerDump> {
        self.with(
            Scope::Ledger(ledger),
            IsolationLevel::RepeatableRead,
            move |tx| Box::pin(async move { dump(tx, ledger).await }),
        )
        .await
    }

    async fn restore(&self, dump: LedgerDump, owner: UserId) -> Result<LedgerId> {
        self.with(Scope::All, IsolationLevel::ReadCommitted, move |tx| {
            Box::pin(async move {
                let ledger: i64 = tx
                    .query_one(
                        "INSERT INTO ledgers (name, revision, created_at, next_transaction_id)
                         VALUES ($1, $2, $3, $4) RETURNING id",
                        &[
                            &dump.name,
                            &dump.revision,
                            &dump.created_at,
                            &(dump.next_transaction_id as i64),
                        ],
                    )
                    .await?
                    .get(0);
                let at = now();
                let mut users = BTreeMap::new();
                for (old, name) in &dump.users {
                    let id: i64 = tx
                        .query_one(
                            "INSERT INTO users (display_name, created_at) VALUES ($1, $2)
                             RETURNING id",
                            &[name, &at],
                        )
                        .await?
                        .get(0);
                    users.insert(*old, id);
                }
                tx.execute(
                    "INSERT INTO memberships (ledger_id, user_id, role) VALUES ($1, $2, 'rewrite')",
                    &[&ledger, &owner],
                )
                .await?;
                for a in &dump.accounts {
                    tx.execute(
                        "INSERT INTO accounts (ledger_id, name, declared_tags) VALUES ($1, $2, $3)",
                        &[&ledger, &a.name, &a.declared_tags],
                    )
                    .await?;
                }
                for c in &dump.commodities {
                    upsert_commodity(tx, ledger, c, true).await?;
                }
                for (t, created, changed) in &dump.transactions {
                    insert_transaction(tx, ledger, t, *created, *changed).await?;
                }
                for row in &dump.audit {
                    let user = *users.get(&row.user).ok_or_else(|| {
                        StoreError::Invalid(format!("audit user {} is missing", row.user))
                    })?;
                    insert_audit(
                        tx,
                        ledger,
                        &AuditRow {
                            user,
                            ..row.clone()
                        },
                    )
                    .await?;
                }
                Ok(ledger)
            })
        })
        .await
    }
}
