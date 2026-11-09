// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The books as one in-memory working copy, which every write applies its
//! change to. A single write is a batch of one: it loads the working copy
//! at the caller's `head`, applies its operation, and either checks the
//! result (a dry run) or commits the rows it changed. A batch applies many
//! operations in order, each seeing the ones before it, and checks once.

use crate::cache::Snapshot;
use crate::ops::OpError as BooksError;
use crate::state::{KindTable, Service};
use kabacus_core::{AccountName, Chart, Status, Tag, Transaction};
use kabacus_ledger::{render_transaction, Account, Book, Styles};
use kabacus_store::{ChangeSet, Revision};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// One transaction's (or declaration's) change, for a compact preview:
/// its text before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The transaction's id; none for a declaration.
    pub id: Option<u64>,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub struct WorkingCopy {
    /// The books as loaded.
    pub base: Arc<Snapshot>,
    /// The books with every operation so far applied.
    pub book: Book,
    pub next_id: u64,
    pub kinds: KindTable,
    /// The revision the working copy was loaded at, as callers write it.
    pub head: String,
    pub revision: Revision,
    /// The books at the last [`WorkingCopy::checkpoint`].
    checkpoint: Book,
}

/// `head` as a revision; anything else matches no revision.
pub fn parse_head(head: &str) -> Option<Revision> {
    head.trim().trim_matches('"').parse().ok()
}

/// `comment` with `tags` declared in it, as the comment stores them:
/// declarations of other tags are kept as they are written.
pub fn with_tags(comment: &str, tags: &[Tag]) -> String {
    let (kept, missing) = kabacus_core::reconcile_tags(comment, tags);
    let mut lines: Vec<String> = kept.split('\n').map(str::to_string).collect();
    if !missing.is_empty() {
        let added = missing
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    format!("{k}:")
                } else {
                    format!("{k}:{v}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        if lines[0].trim().is_empty() {
            lines[0] = added;
        } else {
            lines.push(added);
        }
    }
    let joined = lines.join("\n");
    joined.trim_end_matches('\n').to_string()
}

/// `tx` as it is stored: its tags and its postings' tags written into
/// their comments, and the tags read back from them.
pub fn normalized(mut tx: Transaction) -> Transaction {
    tx.comment = with_tags(&tx.comment, &tx.tags);
    tx.tags = kabacus_core::declared_tags(&tx.comment);
    for p in &mut tx.postings {
        p.comment = with_tags(&p.comment, &p.tags);
        p.tags = kabacus_core::declared_tags(&p.comment);
    }
    tx
}

pub fn primary_commodity(book: &kabacus_ledger::Book, chart: &kabacus_core::Chart) -> String {
    book.transactions
        .iter()
        .flat_map(|t| &t.postings)
        .find(|p| chart.kind_of(&p.account) == kabacus_core::AccountKind::Money)
        .map(|p| p.amount.commodity.clone())
        .or_else(|| {
            book.transactions
                .iter()
                .flat_map(|t| &t.postings)
                .map(|p| p.amount.commodity.clone())
                .min()
        })
        .or_else(|| book.commodities.keys().next().cloned())
        .unwrap_or_else(|| "USD".to_string())
}

impl WorkingCopy {
    /// The books at `expected_head`; books that have moved on are a
    /// conflict, for a dry run as for a commit.
    pub async fn load(state: &Service, expected_head: &str) -> Result<WorkingCopy, BooksError> {
        let base = state.books_snapshot().await?;
        if parse_head(expected_head) != Some(base.revision) {
            return Err(BooksError::Conflict {
                expected: expected_head.to_string(),
                current: base.key.clone(),
            });
        }
        Ok(WorkingCopy::from_snapshot(base, state.kind_table()))
    }

    pub fn from_snapshot(base: Arc<Snapshot>, kinds: KindTable) -> WorkingCopy {
        WorkingCopy {
            book: base.book.clone(),
            checkpoint: base.book.clone(),
            next_id: base.next_id,
            head: base.key.clone(),
            revision: base.revision,
            kinds,
            base,
        }
    }

    pub fn styles(&self) -> Styles {
        self.book.styles()
    }

    /// The cash commodity, falling back to the first used symbol.
    pub fn commodity(&self) -> String {
        primary_commodity(&self.book, &self.rules_chart())
    }

    /// The decimal places `commodity` shows: its declared places, or the
    /// most any of its amounts has.
    pub fn places(&self, commodity: &str) -> u32 {
        self.book
            .commodities
            .get(commodity)
            .and_then(|c| c.style.places)
            .unwrap_or_else(|| {
                self.book
                    .transactions
                    .iter()
                    .flat_map(|t| &t.postings)
                    .filter(|p| p.amount.commodity == commodity)
                    .map(|p| p.amount.money.places())
                    .max()
                    .unwrap_or(2)
            })
    }

    pub fn transactions(&self) -> Vec<Transaction> {
        self.book.transactions.clone()
    }

    pub fn declared(&self) -> BTreeSet<AccountName> {
        self.book.declared_accounts().cloned().collect()
    }

    pub fn is_declared(&self, account: &str) -> bool {
        self.book.is_declared(account)
    }

    /// A chart that only knows the kind rules.
    pub fn rules_chart(&self) -> Chart {
        Chart::from_rules(
            self.kinds.rules.clone(),
            self.kinds.reserve_template.clone(),
        )
    }

    /// Every account in the working copy, classified.
    pub fn chart(&self) -> Chart {
        Chart::from_books(
            self.kinds.rules.clone(),
            self.kinds.reserve_template.clone(),
            self.declared(),
            &self.book.transactions,
            &self.friendly_names(),
        )
        .with_account_types(&crate::state::account_types(&self.book))
    }

    pub fn friendly_names(&self) -> HashMap<String, String> {
        crate::state::friendly_names(&self.book)
    }

    /// Which transaction has `id`.
    pub fn find(&self, id: u64) -> Option<usize> {
        self.book.transactions.iter().position(|t| t.id == id)
    }

    pub fn tx(&self, i: usize) -> &Transaction {
        &self.book.transactions[i]
    }

    fn text(&self, tx: &Transaction) -> String {
        render_transaction(tx, &self.styles())
    }

    // --- transactions -------------------------------------------------------

    /// Add `tx` with the next id. Returns the change and the id.
    pub fn add(&mut self, tx: Transaction) -> Result<Change, BooksError> {
        let tx = Transaction {
            id: self.next_id,
            ..normalized(tx)
        };
        self.next_id += 1;
        let change = Change {
            id: Some(tx.id),
            before: None,
            after: Some(self.text(&tx)),
        };
        self.book.transactions.push(tx);
        Ok(change)
    }

    /// Write `new` over transaction `i`, keeping its id.
    pub fn replace(&mut self, i: usize, new: Transaction) -> Result<Change, BooksError> {
        let old = self.book.transactions[i].clone();
        let new = Transaction {
            id: old.id,
            ..normalized(new)
        };
        let change = Change {
            id: Some(old.id),
            before: Some(self.text(&old)),
            after: Some(self.text(&new)),
        };
        self.book.transactions[i] = new;
        Ok(change)
    }

    pub fn delete(&mut self, i: usize) -> Result<Change, BooksError> {
        let old = self.book.transactions.remove(i);
        Ok(Change {
            id: Some(old.id),
            before: Some(self.text(&old)),
            after: None,
        })
    }

    /// Set the status mark of transaction `i`'s first posting to `account`,
    /// and nothing else.
    pub fn set_status(
        &mut self,
        i: usize,
        account: &AccountName,
        status: Status,
    ) -> Result<Change, BooksError> {
        let mut tx = self.book.transactions[i].clone();
        let posting = tx
            .postings
            .iter_mut()
            .find(|p| &p.account == account)
            .ok_or_else(|| {
                BooksError::invalid(
                    "account",
                    format!("this transaction has no posting to {account}"),
                )
            })?;
        posting.status = status;
        self.replace(i, tx)
    }

    // --- declarations ---------------------------------------------------------

    /// Declare `account` with `tags` (the declaration's comment).
    pub fn declare(&mut self, account: &str, tags: &str) -> Change {
        let declaration = Account {
            name: AccountName::new(account),
            tags: tags.to_string(),
        };
        let after = declaration_text(&declaration);
        let before = self.book.accounts.get(account).map(declaration_text);
        self.book.accounts.insert(account.to_string(), declaration);
        Change {
            id: None,
            before,
            after: Some(after),
        }
    }

    /// Take away `account`'s declaration.
    pub fn undeclare(&mut self, account: &str) -> Option<Change> {
        let old = self.book.accounts.remove(account)?;
        Some(Change {
            id: None,
            before: Some(declaration_text(&old)),
            after: None,
        })
    }

    // --- results -------------------------------------------------------------------

    /// Every row changed since the books were loaded.
    pub fn changes(&self) -> ChangeSet {
        ChangeSet::between(&self.base.book, &self.book)
    }

    /// The rows changed since the last checkpoint, and a new checkpoint:
    /// one commit's worth, for a batch committed per operation.
    pub fn checkpoint(&mut self) -> ChangeSet {
        let changes = ChangeSet::between(&self.checkpoint, &self.book);
        self.checkpoint = self.book.clone();
        changes
    }

    /// A unified diff of every change, as journal text.
    pub fn diff(&self) -> String {
        diff_books(&self.base.book, &self.book)
    }

    /// Each account's balance (one commodity) over every transaction.
    pub fn balances(
        &self,
        accounts: &BTreeSet<(String, String)>,
    ) -> BTreeMap<(String, String), kabacus_core::Money> {
        let mut out: BTreeMap<(String, String), kabacus_core::Money> = accounts
            .iter()
            .map(|k| (k.clone(), kabacus_core::Money::zero()))
            .collect();
        for tx in &self.book.transactions {
            for p in &tx.postings {
                let key = (p.account.as_str().to_string(), p.amount.commodity.clone());
                if let Some(total) = out.get_mut(&key) {
                    *total = total.clone() + p.amount.money.clone();
                }
            }
        }
        out
    }
}

/// An account declaration as journal text.
pub fn declaration_text(account: &Account) -> String {
    let mut line = format!("account {}", account.name);
    let mut lines = account.tags.split('\n');
    if let Some(first) = lines.next().filter(|l| !l.trim().is_empty()) {
        line.push_str("  ; ");
        line.push_str(first.trim());
    }
    for rest in lines {
        line.push_str("\n    ; ");
        line.push_str(rest.trim());
    }
    line.push('\n');
    line
}

/// What changed from `before` to `after`, as a unified diff of journal
/// text: one hunk per transaction or declaration.
pub fn diff_books(before: &Book, after: &Book) -> String {
    diff_changes(before, after, &ChangeSet::between(before, after))
}

/// `changes` (between `before` and `after`) as a unified diff.
pub fn diff_changes(before: &Book, after: &Book, changes: &ChangeSet) -> String {
    let (old_styles, new_styles) = (before.styles(), after.styles());
    let text = |row: &kabacus_store::Row, styles: &Styles| -> (String, String) {
        match row {
            kabacus_store::Row::Transaction(t) => (
                format!("transaction #{}", t.id),
                kabacus_store::rows::transaction(t)
                    .map(|tx| render_transaction(&tx, styles))
                    .unwrap_or_default(),
            ),
            kabacus_store::Row::Account(a) => (
                format!("account {}", a.name),
                declaration_text(&kabacus_store::rows::account(a)),
            ),
            kabacus_store::Row::Commodity(c) => (
                format!("commodity {}", c.symbol),
                kabacus_store::rows::commodity(c)
                    .map(|c| {
                        format!(
                            "commodity {}\n",
                            kabacus_ledger::amount::render_example(&c.symbol, &c.style)
                        )
                    })
                    .unwrap_or_default(),
            ),
        }
    };
    changes
        .changes
        .iter()
        .map(|c| {
            let old = c.before.as_ref().map(|r| text(r, &old_styles));
            let new = c.after.as_ref().map(|r| text(r, &new_styles));
            let name = new
                .as_ref()
                .or(old.as_ref())
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            crate::diff::unified_diff(
                &name,
                old.as_ref().map(|(_, t)| t.as_str()).unwrap_or(""),
                new.as_ref().map(|(_, t)| t.as_str()).unwrap_or(""),
            )
        })
        .collect()
}
