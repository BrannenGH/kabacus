// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The store on SQLite: one `.kabacus` file, one writer at a time, for
//! local mode and small self-hosted servers.

pub mod schema;

use crate::rows::{AccountRow, CommodityRow, PostingRow, Row, RowChange, TxRow};
use crate::{
    now, AuditPayload, AuditRow, BookStore, ChangeInfo, ChangeRecord, ChangeSet, LedgerDump,
    LedgerId, LedgerInfo, Loaded, Result, Revision, Role, StoreError, UserId, Who,
};
use kabacus_ledger::Book;
use rusqlite::{params, Connection, OptionalExtension, Transaction as SqlTx, TransactionBehavior};
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The SQLite store. Calls run on the blocking pool, one at a time.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Open (or create) the `.kabacus` file at `path`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: &Path) -> Result<SqliteStore> {
        let conn = schema::open_file(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        Self::with_connection(conn)
    }

    /// A store in memory, for tests.
    pub fn in_memory() -> Result<SqliteStore> {
        Self::with_connection(schema::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<SqliteStore> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(SqliteStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        crate::file::read_bytes(bytes)?;
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize_read_exact("main", bytes, bytes.len(), false)?;
        schema::migrate(&mut conn)?;
        Self::with_connection(conn)
    }

    pub async fn to_bytes(&self) -> Result<Vec<u8>> {
        self.call(|conn| Ok(conn.serialize("main")?.to_vec())).await
    }

    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let conn = self.conn.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            tokio::task::spawn_blocking(move || {
                let mut conn = conn.lock().unwrap_or_else(|p| p.into_inner());
                f(&mut conn)
            })
            .await
            .map_err(|e| StoreError::Invalid(e.to_string()))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut conn)
        }
    }
}

fn not_found(ledger: LedgerId) -> StoreError {
    StoreError::NotFound(format!("ledger {ledger}"))
}

fn ledger_revision(conn: &Connection, ledger: LedgerId) -> Result<(Revision, u64)> {
    conn.query_row(
        "SELECT revision, next_transaction_id FROM ledgers WHERE id = ?1",
        [ledger],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as u64)),
    )
    .optional()?
    .ok_or_else(|| not_found(ledger))
}

fn read_accounts(conn: &Connection, ledger: LedgerId) -> Result<Vec<AccountRow>> {
    let mut stmt = conn
        .prepare("SELECT name, declared_tags FROM accounts WHERE ledger_id = ?1 ORDER BY name")?;
    let rows = stmt
        .query_map([ledger], |r| {
            Ok(AccountRow {
                name: r.get(0)?,
                declared_tags: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_commodities(conn: &Connection, ledger: LedgerId) -> Result<Vec<CommodityRow>> {
    let mut stmt = conn.prepare(
        "SELECT symbol, style_side, style_space, decimal_mark, group_mark, group_sizes, places
         FROM commodities WHERE ledger_id = ?1 ORDER BY symbol",
    )?;
    let rows = stmt
        .query_map([ledger], |r| {
            Ok(CommodityRow {
                symbol: r.get(0)?,
                style_side: r.get(1)?,
                style_space: r.get::<_, i64>(2)? != 0,
                decimal_mark: r.get(3)?,
                group_mark: r.get(4)?,
                group_sizes: r.get(5)?,
                places: r.get::<_, Option<i64>>(6)?.map(|p| p as u32),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every transaction with its postings and revision columns, by id.
fn read_transactions(
    conn: &Connection,
    ledger: LedgerId,
) -> Result<Vec<(TxRow, Revision, Revision)>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, status, code, description, comment, revision_created, revision_changed
         FROM transactions WHERE ledger_id = ?1 ORDER BY id",
    )?;
    let mut txs: Vec<(TxRow, Revision, Revision)> = stmt
        .query_map([ledger], |r| {
            Ok((
                TxRow {
                    id: r.get::<_, i64>(0)? as u64,
                    date: r.get(1)?,
                    status: r.get(2)?,
                    code: r.get(3)?,
                    description: r.get(4)?,
                    comment: r.get(5)?,
                    postings: Vec::new(),
                },
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let index: BTreeMap<u64, usize> = txs
        .iter()
        .enumerate()
        .map(|(i, (t, _, _))| (t.id, i))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT transaction_id, account, kind, status, mantissa, places, commodity,
                cost_kind, cost_mantissa, cost_places, cost_commodity,
                assert_op, assert_mantissa, assert_places, assert_commodity, comment
         FROM postings WHERE ledger_id = ?1 ORDER BY transaction_id, position",
    )?;
    let postings = stmt.query_map([ledger], |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            PostingRow {
                account: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                mantissa: r.get(4)?,
                places: r.get::<_, i64>(5)? as u32,
                commodity: r.get(6)?,
                cost_kind: r.get(7)?,
                cost_mantissa: r.get(8)?,
                cost_places: r.get::<_, Option<i64>>(9)?.map(|p| p as u32),
                cost_commodity: r.get(10)?,
                assert_op: r.get(11)?,
                assert_mantissa: r.get(12)?,
                assert_places: r.get::<_, Option<i64>>(13)?.map(|p| p as u32),
                assert_commodity: r.get(14)?,
                comment: r.get(15)?,
            },
        ))
    })?;
    for p in postings {
        let (id, posting) = p?;
        let i = index
            .get(&id)
            .ok_or_else(|| StoreError::Invalid(format!("a posting of missing transaction {id}")))?;
        txs[*i].0.postings.push(posting);
    }
    Ok(txs)
}

fn read_audit(conn: &Connection, ledger: LedgerId, after: Revision) -> Result<Vec<AuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT revision, n, user_id, actor, kind, summary, payload FROM audit_log
         WHERE ledger_id = ?1 AND revision > ?2 ORDER BY revision, n",
    )?;
    let rows = stmt
        .query_map(params![ledger, after], |r| {
            Ok(AuditRow {
                revision: r.get(0)?,
                n: r.get(1)?,
                user: r.get(2)?,
                actor: r.get(3)?,
                kind: r.get(4)?,
                summary: r.get(5)?,
                payload: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
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

fn load(conn: &mut Connection, ledger: LedgerId, at: Option<Revision>) -> Result<Loaded> {
    let tx = conn.transaction()?;
    let (revision, next_id) = ledger_revision(&tx, ledger)?;
    let mut book = book_from(
        &read_accounts(&tx, ledger)?,
        &read_commodities(&tx, ledger)?,
        &read_transactions(&tx, ledger)?,
    )?;
    let revision = match at {
        Some(at) if at < revision => {
            if at < 1 {
                return Err(StoreError::NotFound(format!("revision {at}")));
            }
            let audit = read_audit(&tx, ledger, at)?;
            for row in audit.iter().rev() {
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
    tx.finish()?;
    Ok(Loaded {
        book,
        revision,
        next_id,
    })
}

fn insert_postings(tx: &SqlTx, ledger: LedgerId, t: &TxRow) -> Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO postings (ledger_id, transaction_id, position, account, kind, status,
            mantissa, places, commodity, cost_kind, cost_mantissa, cost_places, cost_commodity,
            assert_op, assert_mantissa, assert_places, assert_commodity, comment)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
    )?;
    for (position, p) in t.postings.iter().enumerate() {
        stmt.execute(params![
            ledger,
            t.id as i64,
            position as i64,
            p.account,
            p.kind,
            p.status,
            p.mantissa,
            p.places as i64,
            p.commodity,
            p.cost_kind,
            p.cost_mantissa,
            p.cost_places.map(i64::from),
            p.cost_commodity,
            p.assert_op,
            p.assert_mantissa,
            p.assert_places.map(i64::from),
            p.assert_commodity,
            p.comment,
        ])?;
    }
    Ok(())
}

fn insert_transaction(
    tx: &SqlTx,
    ledger: LedgerId,
    t: &TxRow,
    created: Revision,
    changed: Revision,
) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO transactions (ledger_id, id, date, status, code, description, comment,
            revision_created, revision_changed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        ledger,
        t.id as i64,
        t.date,
        t.status,
        t.code,
        t.description,
        t.comment,
        created,
        changed
    ])?;
    insert_postings(tx, ledger, t)
}

fn insert_account(tx: &SqlTx, ledger: LedgerId, a: &AccountRow) -> Result<()> {
    tx.execute(
        "INSERT INTO accounts (ledger_id, name, declared_tags) VALUES (?1, ?2, ?3)",
        params![ledger, a.name, a.declared_tags],
    )?;
    Ok(())
}

fn upsert_commodity(tx: &SqlTx, ledger: LedgerId, c: &CommodityRow, insert: bool) -> Result<()> {
    let sql = if insert {
        "INSERT INTO commodities (ledger_id, symbol, style_side, style_space, decimal_mark,
            group_mark, group_sizes, places) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
    } else {
        "UPDATE commodities SET style_side = ?3, style_space = ?4, decimal_mark = ?5,
            group_mark = ?6, group_sizes = ?7, places = ?8 WHERE ledger_id = ?1 AND symbol = ?2"
    };
    tx.execute(
        sql,
        params![
            ledger,
            c.symbol,
            c.style_side,
            c.style_space as i64,
            c.decimal_mark,
            c.group_mark,
            c.group_sizes,
            c.places.map(i64::from)
        ],
    )?;
    Ok(())
}

fn apply_change(tx: &SqlTx, ledger: LedgerId, rev: Revision, change: &RowChange) -> Result<()> {
    match (&change.before, &change.after) {
        (None, Some(Row::Transaction(t))) => insert_transaction(tx, ledger, t, rev, rev)?,
        (Some(Row::Transaction(_)), Some(Row::Transaction(t))) => {
            let n = tx.execute(
                "UPDATE transactions SET date = ?3, status = ?4, code = ?5, description = ?6,
                    comment = ?7, revision_changed = ?8 WHERE ledger_id = ?1 AND id = ?2",
                params![
                    ledger,
                    t.id as i64,
                    t.date,
                    t.status,
                    t.code,
                    t.description,
                    t.comment,
                    rev
                ],
            )?;
            if n != 1 {
                return Err(StoreError::NotFound(format!("transaction {}", t.id)));
            }
            tx.execute(
                "DELETE FROM postings WHERE ledger_id = ?1 AND transaction_id = ?2",
                params![ledger, t.id as i64],
            )?;
            insert_postings(tx, ledger, t)?;
        }
        (Some(Row::Transaction(t)), None) => {
            let n = tx.execute(
                "DELETE FROM transactions WHERE ledger_id = ?1 AND id = ?2",
                params![ledger, t.id as i64],
            )?;
            if n != 1 {
                return Err(StoreError::NotFound(format!("transaction {}", t.id)));
            }
        }
        (None, Some(Row::Account(a))) => insert_account(tx, ledger, a)?,
        (Some(Row::Account(_)), Some(Row::Account(a))) => {
            tx.execute(
                "UPDATE accounts SET declared_tags = ?3 WHERE ledger_id = ?1 AND name = ?2",
                params![ledger, a.name, a.declared_tags],
            )?;
        }
        (Some(Row::Account(a)), None) => {
            tx.execute(
                "DELETE FROM accounts WHERE ledger_id = ?1 AND name = ?2",
                params![ledger, a.name],
            )?;
        }
        (None, Some(Row::Commodity(c))) => upsert_commodity(tx, ledger, c, true)?,
        (Some(Row::Commodity(_)), Some(Row::Commodity(c))) => {
            upsert_commodity(tx, ledger, c, false)?
        }
        (Some(Row::Commodity(c)), None) => {
            tx.execute(
                "DELETE FROM commodities WHERE ledger_id = ?1 AND symbol = ?2",
                params![ledger, c.symbol],
            )?;
        }
        _ => {
            return Err(StoreError::Invalid(
                "a change must keep one row of one table".into(),
            ))
        }
    }
    Ok(())
}

fn insert_audit(tx: &SqlTx, ledger: LedgerId, row: &AuditRow) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO audit_log (ledger_id, revision, n, user_id, actor, kind, summary, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        ledger,
        row.revision,
        row.n,
        row.user,
        row.actor,
        row.kind,
        row.summary,
        row.payload
    ])?;
    Ok(())
}

/// The id the next new transaction may take after `changes`.
pub(crate) fn next_id_after(next: u64, changes: &ChangeSet) -> Result<u64> {
    let mut next = next;
    for c in &changes.changes {
        if let (None, Some(Row::Transaction(t))) = (&c.before, &c.after) {
            if t.id < next {
                return Err(StoreError::Invalid(format!(
                    "transaction id {} is taken; the next is {next}",
                    t.id
                )));
            }
            next = t.id + 1;
        }
    }
    Ok(next)
}

fn commit(
    conn: &mut Connection,
    ledger: LedgerId,
    expected: Revision,
    changes: &ChangeSet,
    who: &Who,
) -> Result<Revision> {
    if changes.is_empty() {
        return Err(StoreError::Invalid("a write must change something".into()));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = tx.execute(
        "UPDATE ledgers SET revision = revision + 1 WHERE id = ?1 AND revision = ?2",
        params![ledger, expected],
    )?;
    if updated == 0 {
        let (current, _) = ledger_revision(&tx, ledger)?;
        return Err(StoreError::Conflict { expected, current });
    }
    let rev = expected + 1;
    let (_, next) = ledger_revision(&tx, ledger)?;
    let next = next_id_after(next, changes)?;
    let at = now();
    for (n, change) in changes.changes.iter().enumerate() {
        apply_change(&tx, ledger, rev, change)?;
        let payload = AuditPayload {
            at: at.clone(),
            who: who.details.clone(),
            change: Some(change.clone()),
            ledger: None,
        };
        insert_audit(
            &tx,
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
        )?;
    }
    tx.execute(
        "UPDATE ledgers SET next_transaction_id = ?2 WHERE id = ?1",
        params![ledger, next as i64],
    )?;
    tx.commit()?;
    Ok(rev)
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

fn audit_with_users(
    conn: &Connection,
    ledger: LedgerId,
    from: Revision,
    to: Revision,
) -> Result<Vec<(AuditRow, String)>> {
    let mut stmt = conn.prepare(
        "SELECT a.revision, a.n, a.user_id, a.actor, a.kind, a.summary, a.payload, u.display_name
         FROM audit_log a JOIN users u ON u.id = a.user_id
         WHERE a.ledger_id = ?1 AND a.revision BETWEEN ?2 AND ?3 ORDER BY a.revision DESC, a.n",
    )?;
    let rows = stmt
        .query_map(params![ledger, from, to], |r| {
            Ok((
                AuditRow {
                    revision: r.get(0)?,
                    n: r.get(1)?,
                    user: r.get(2)?,
                    actor: r.get(3)?,
                    kind: r.get(4)?,
                    summary: r.get(5)?,
                    payload: r.get(6)?,
                },
                r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
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

fn create_ledger(conn: &mut Connection, name: &str, owner: UserId) -> Result<LedgerId> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let at = now();
    tx.execute(
        "INSERT INTO ledgers (name, revision, created_at) VALUES (?1, 1, ?2)",
        params![name, at],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO memberships (ledger_id, user_id, role) VALUES (?1, ?2, 'rewrite')",
        params![id, owner],
    )?;
    let payload = AuditPayload {
        at,
        who: serde_json::Value::Null,
        change: None,
        ledger: Some(name.to_string()),
    };
    insert_audit(
        &tx,
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
    )?;
    tx.commit()?;
    Ok(id)
}

fn dump(conn: &mut Connection, ledger: LedgerId) -> Result<LedgerDump> {
    let tx = conn.transaction()?;
    let (name, created_at, revision, next): (String, String, i64, i64) = tx
        .query_row(
            "SELECT name, created_at, revision, next_transaction_id FROM ledgers WHERE id = ?1",
            [ledger],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| not_found(ledger))?;
    let audit = read_audit(&tx, ledger, 0)?;
    let mut users = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT u.id, u.display_name FROM users u
             JOIN audit_log a ON a.user_id = u.id WHERE a.ledger_id = ?1 ORDER BY u.id",
        )?;
        for row in stmt.query_map([ledger], |r| Ok((r.get(0)?, r.get(1)?)))? {
            users.push(row?);
        }
    }
    let out = LedgerDump {
        name,
        created_at,
        revision,
        next_transaction_id: next as u64,
        accounts: read_accounts(&tx, ledger)?,
        commodities: read_commodities(&tx, ledger)?,
        transactions: read_transactions(&tx, ledger)?,
        audit,
        users,
    };
    tx.finish()?;
    Ok(out)
}

/// The ledger of `dump` and everything in it, its audit rows pointing at
/// `users` (the dump's user ids to this store's).
fn insert_ledger(
    tx: &SqlTx,
    dump: &LedgerDump,
    users: &BTreeMap<UserId, UserId>,
) -> Result<LedgerId> {
    tx.execute(
        "INSERT INTO ledgers (name, revision, created_at, next_transaction_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            dump.name,
            dump.revision,
            dump.created_at,
            dump.next_transaction_id as i64
        ],
    )?;
    let ledger = tx.last_insert_rowid();
    for a in &dump.accounts {
        insert_account(tx, ledger, a)?;
    }
    for c in &dump.commodities {
        upsert_commodity(tx, ledger, c, true)?;
    }
    for (t, created, changed) in &dump.transactions {
        insert_transaction(tx, ledger, t, *created, *changed)?;
    }
    for row in &dump.audit {
        let user = *users
            .get(&row.user)
            .ok_or_else(|| StoreError::Invalid(format!("audit user {} is missing", row.user)))?;
        insert_audit(
            tx,
            ledger,
            &AuditRow {
                user,
                ..row.clone()
            },
        )?;
    }
    Ok(ledger)
}

fn restore(conn: &mut Connection, dump: &LedgerDump, owner: UserId) -> Result<LedgerId> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let at = now();
    let mut users = BTreeMap::new();
    for (old, name) in &dump.users {
        tx.execute(
            "INSERT INTO users (display_name, created_at) VALUES (?1, ?2)",
            params![name, at],
        )?;
        users.insert(*old, tx.last_insert_rowid());
    }
    let ledger = insert_ledger(&tx, dump, &users)?;
    tx.execute(
        "INSERT INTO memberships (ledger_id, user_id, role) VALUES (?1, ?2, 'rewrite')",
        params![ledger, owner],
    )?;
    tx.commit()?;
    Ok(ledger)
}

impl BookStore for SqliteStore {
    async fn create_ledger(&self, name: &str, owner: UserId) -> Result<LedgerId> {
        let name = name.to_string();
        self.call(move |c| create_ledger(c, &name, owner)).await
    }

    async fn ledgers(&self) -> Result<Vec<LedgerInfo>> {
        self.call(|c| {
            let mut stmt =
                c.prepare("SELECT id, name, revision, created_at FROM ledgers ORDER BY id")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(LedgerInfo {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        revision: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn revision(&self, ledger: LedgerId) -> Result<Revision> {
        self.call(move |c| Ok(ledger_revision(c, ledger)?.0)).await
    }

    async fn load(&self, ledger: LedgerId, at: Option<Revision>) -> Result<Loaded> {
        self.call(move |c| load(c, ledger, at)).await
    }

    async fn commit(
        &self,
        ledger: LedgerId,
        expected: Revision,
        changes: ChangeSet,
        who: &Who,
    ) -> Result<Revision> {
        let who = who.clone();
        self.call(move |c| commit(c, ledger, expected, &changes, &who))
            .await
    }

    async fn history(&self, ledger: LedgerId, limit: usize) -> Result<Vec<ChangeInfo>> {
        self.call(move |c| {
            let (revision, _) = ledger_revision(c, ledger)?;
            let from = (revision - limit as i64 + 1).max(1);
            group_by_revision(audit_with_users(c, ledger, from, revision)?)
                .iter()
                .map(|g| change_info(g))
                .collect()
        })
        .await
    }

    async fn transaction_history(&self, ledger: LedgerId, id: u64) -> Result<Vec<ChangeInfo>> {
        self.call(move |c| {
            ledger_revision(c, ledger)?;
            let revisions: Vec<i64> = {
                let mut stmt = c.prepare(
                    "SELECT DISTINCT revision FROM audit_log WHERE ledger_id = ?1 AND (
                        (json_extract(payload, '$.change.before.table') = 'transaction'
                         AND json_extract(payload, '$.change.before.id') = ?2)
                     OR (json_extract(payload, '$.change.after.table') = 'transaction'
                         AND json_extract(payload, '$.change.after.id') = ?2))
                     ORDER BY revision DESC",
                )?;
                let rows = stmt
                    .query_map(params![ledger, id as i64], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            revisions
                .into_iter()
                .map(|rev| change_info(&audit_with_users(c, ledger, rev, rev)?))
                .collect()
        })
        .await
    }

    async fn change(&self, ledger: LedgerId, revision: Revision) -> Result<ChangeRecord> {
        self.call(move |c| {
            ledger_revision(c, ledger)?;
            let rows = audit_with_users(c, ledger, revision, revision)?;
            if rows.is_empty() {
                return Err(StoreError::NotFound(format!("revision {revision}")));
            }
            let info = change_info(&rows)?;
            let mut changes = Vec::new();
            let mut ordered = rows;
            ordered.sort_by_key(|(r, _)| r.n);
            for (row, _) in ordered {
                let payload: AuditPayload = serde_json::from_str(&row.payload)?;
                changes.extend(payload.change);
            }
            Ok(ChangeRecord { info, changes })
        })
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
        self.call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let found: Option<i64> = tx
                .query_row(
                    "SELECT user_id FROM identities WHERE provider = ?1 AND subject = ?2",
                    params![provider, subject],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match found {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO users (display_name, created_at) VALUES (?1, ?2)",
                        params![display_name, now()],
                    )?;
                    let id = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO identities (provider, subject, user_id) VALUES (?1, ?2, ?3)",
                        params![provider, subject, id],
                    )?;
                    id
                }
            };
            tx.commit()?;
            Ok(id)
        })
        .await
    }

    async fn local_user(&self, display_name: &str) -> Result<UserId> {
        let display_name = display_name.to_string();
        self.call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let found: Option<i64> = tx
                .query_row(
                    "SELECT id FROM users WHERE display_name = ?1 AND password_hash IS NULL
                     AND NOT EXISTS (SELECT 1 FROM identities i WHERE i.user_id = users.id)
                     ORDER BY id LIMIT 1",
                    [&display_name],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match found {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO users (display_name, created_at) VALUES (?1, ?2)",
                        params![display_name, now()],
                    )?;
                    tx.last_insert_rowid()
                }
            };
            tx.commit()?;
            Ok(id)
        })
        .await
    }

    async fn memberships(&self, user: UserId) -> Result<Vec<(LedgerId, Role)>> {
        self.call(move |c| {
            let mut stmt = c.prepare(
                "SELECT ledger_id, role FROM memberships WHERE user_id = ?1 ORDER BY ledger_id",
            )?;
            let rows = stmt
                .query_map([user], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows
                .into_iter()
                .filter_map(|(l, r)| Role::parse(&r).map(|r| (l, r)))
                .collect())
        })
        .await
    }

    async fn set_membership(
        &self,
        ledger: LedgerId,
        user: UserId,
        role: Option<Role>,
    ) -> Result<()> {
        self.call(move |c| {
            match role {
                Some(role) => c.execute(
                    "INSERT INTO memberships (ledger_id, user_id, role) VALUES (?1, ?2, ?3)
                     ON CONFLICT (ledger_id, user_id) DO UPDATE SET role = excluded.role",
                    params![ledger, user, role.as_str()],
                )?,
                None => c.execute(
                    "DELETE FROM memberships WHERE ledger_id = ?1 AND user_id = ?2",
                    params![ledger, user],
                )?,
            };
            Ok(())
        })
        .await
    }

    async fn dump(&self, ledger: LedgerId) -> Result<LedgerDump> {
        self.call(move |c| dump(c, ledger)).await
    }

    async fn restore(&self, dump: LedgerDump, owner: UserId) -> Result<LedgerId> {
        self.call(move |c| restore(c, &dump, owner)).await
    }
}

/// Rows of a book as stored, for writing a `.kabacus` file directly.
pub(crate) fn write_dump(conn: &mut Connection, dump: &LedgerDump) -> Result<LedgerId> {
    let tx = conn.transaction()?;
    for (id, name) in &dump.users {
        tx.execute(
            "INSERT INTO users (id, display_name, created_at) VALUES (?1, ?2, ?3)",
            params![id, name, dump.created_at],
        )?;
    }
    tx.commit()?;
    // The users are the file's own; the ledger keeps their ids.
    let identity: BTreeMap<UserId, UserId> = dump.users.iter().map(|(id, _)| (*id, *id)).collect();
    let tx = conn.transaction()?;
    let ledger = insert_ledger(&tx, dump, &identity)?;
    tx.commit()?;
    Ok(ledger)
}

/// Every ledger's rows in an open `.kabacus` file, read with fixed
/// queries.
pub(crate) fn read_dumps(conn: &mut Connection) -> Result<Vec<LedgerDump>> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM ledgers ORDER BY id")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids
    };
    ids.into_iter().map(|id| dump(conn, id)).collect()
}
