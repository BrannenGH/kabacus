// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The books as recently read, kept in memory by (ledger, revision): a
//! revision's books never change, so an entry is never stale, and the
//! least recently used ones are dropped.

use crate::state::Service;
use kabacus_core::{AccountName, Transaction};
use kabacus_ledger::Book;
use kabacus_store::{BookStore, LedgerId, Revision};
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// How many revisions are kept.
const CAPACITY: usize = 8;

/// One revision of a ledger's books.
#[derive(Debug)]
pub struct Snapshot {
    pub revision: Revision,
    /// The revision as callers see it (`head`).
    pub key: String,
    pub book: Book,
    /// The id the next new transaction takes.
    pub next_id: u64,
    /// The book's transactions, in id order.
    pub transactions: Vec<Transaction>,
    pub declared: HashSet<AccountName>,
    /// The commodities the postings use, sorted (declared but unused ones
    /// aren't).
    pub commodities: Vec<String>,
}

impl Snapshot {
    pub fn new(revision: Revision, book: Book, next_id: u64) -> Snapshot {
        let transactions = book.transactions.clone();
        let declared = book.declared_accounts().cloned().collect();
        let commodities = transactions
            .iter()
            .flat_map(|t| &t.postings)
            .map(|p| p.amount.commodity.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Snapshot {
            revision,
            key: revision.to_string(),
            book,
            next_id,
            transactions,
            declared,
            commodities,
        }
    }
}

#[derive(Debug, Default)]
pub struct BooksCache {
    entries: std::sync::Mutex<VecDeque<(LedgerId, Revision, Arc<Snapshot>)>>,
    loads: AtomicU64,
}

impl BooksCache {
    /// How many times the books have been read from the store.
    pub fn loads(&self) -> u64 {
        self.loads.load(Ordering::Relaxed)
    }

    pub async fn get(&self, ledger: LedgerId, revision: Revision) -> Option<Arc<Snapshot>> {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let at = entries
            .iter()
            .position(|(l, r, _)| *l == ledger && *r == revision)?;
        let entry = entries.remove(at)?;
        let snapshot = entry.2.clone();
        entries.push_front(entry);
        Some(snapshot)
    }

    pub async fn put(&self, ledger: LedgerId, snapshot: Arc<Snapshot>) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|(l, r, _)| !(*l == ledger && *r == snapshot.revision));
        entries.push_front((ledger, snapshot.revision, snapshot));
        entries.truncate(CAPACITY);
    }
}

impl Service {
    /// The books at `revision` (now, when none), from memory when read
    /// before.
    pub async fn books_at(&self, revision: Option<Revision>) -> anyhow::Result<Arc<Snapshot>> {
        let revision = match revision {
            Some(r) => r,
            None => self.store.revision(self.ledger).await?,
        };
        if let Some(snapshot) = self.books_cache.get(self.ledger, revision).await {
            return Ok(snapshot);
        }
        let loaded = self.store.load(self.ledger, Some(revision)).await?;
        self.books_cache.loads.fetch_add(1, Ordering::Relaxed);
        let snapshot = Arc::new(Snapshot::new(loaded.revision, loaded.book, loaded.next_id));
        self.books_cache.put(self.ledger, snapshot.clone()).await;
        Ok(snapshot)
    }

    /// The books as they are now.
    pub async fn books_snapshot(&self) -> anyhow::Result<Arc<Snapshot>> {
        self.books_at(None).await
    }
}
