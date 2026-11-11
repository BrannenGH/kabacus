// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's view model: what the sidebar, register and editor show and
//! do, as plain async Rust over [`KabacusClient`]. The Qt bridge only
//! marshals these into QML, so the behaviour is tested here without Qt.

use crate::{i18n, i18nc};
use kabacus_client::types::{AddTransactionBody, AmountView, DraftAmount, DraftPostingBody};
use kabacus_client::{ClientError, KabacusClient};
use kabacus_core::Money;
use serde::Serialize;

impl ModelError {
    /// What the app shows: one plain sentence saying what to do.
    /// `server` is the address it's connected to, if known. The raw error is for
    /// the log and the Details view.
    pub fn user_message(&self, server: &str) -> String {
        match self {
            ModelError::Invalid(message) => message.clone(),
            ModelError::Client(e) => match e {
                ClientError::Request(_) if server.is_empty() => {
                    i18n!("Can't reach kabacus-server. Is it running?")
                }
                ClientError::Request(_) => i18nc!(
                    "%1 is the server's address",
                    "Can't reach kabacus-server at %1. Is it running?",
                    server
                ),
                ClientError::Timeout => i18n!("The server is taking too long to answer."),
                ClientError::Unauthorized | ClientError::SessionExpired(_) => {
                    i18n!("Your session ended. Sign in again.")
                }
                ClientError::Auth(_) => i18n!("Signing in didn't work. Try again."),
                ClientError::Forbidden(_) => {
                    i18n!("You can view these books but not change them.")
                }
                ClientError::NotFound(_) => i18n!("That isn't in the books any more."),
                ClientError::Conflict { .. } => {
                    i18n!("The books changed while you were editing. Review again.")
                }
                // The server's own words, which it doesn't translate (yet).
                ClientError::Invalid { message, .. } => message.clone(),
                ClientError::Server(_) | ClientError::Json(_) => {
                    i18n!("The server had a problem.")
                }
            },
        }
    }
}

/// The sidebar's pseudo-account for the all-accounts register.
pub const ALL_TRANSACTIONS: &str = "";

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SidebarEntry {
    pub account: String,
    pub name: String,
    pub balance: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegisterRow {
    pub id: String,
    pub date: String,
    pub payee: String,
    /// The other side of the transaction.
    pub account: String,
    pub memo: String,
    pub outflow: String,
    pub inflow: String,
    pub balance: String,
    pub status: String,
}

/// The editor's simple mode: money moves from one account to
/// another.
#[derive(Debug, Clone, Default)]
pub struct SimpleDraft {
    pub date: String,
    pub payee: String,
    pub memo: String,
    pub from_account: String,
    pub to_account: String,
    pub amount: String,
}

/// Between the items of a list ("12.00 USD, 3.00 EUR"): the comma is
/// the language's (Arabic writes "،", Chinese "、").
pub(crate) fn list_separator() -> String {
    i18nc!(
        "between the items of a list, such as amounts or account names",
        ", "
    )
}

fn join_balances(balances: &[AmountView]) -> String {
    if balances.is_empty() {
        return "0".to_string();
    }
    balances
        .iter()
        .map(|b| b.formatted.as_str())
        .collect::<Vec<_>>()
        .join(&list_separator())
}

fn split_flow(amount: &AmountView) -> (String, String) {
    let text = amount.formatted.trim_start_matches('-').to_string();
    if amount.exact.starts_with('-') {
        (text, String::new())
    } else {
        (String::new(), text)
    }
}

/// "All transactions" first, then every account with its balance.
pub async fn sidebar(client: &KabacusClient) -> Result<Vec<SidebarEntry>, ModelError> {
    let mut entries = vec![SidebarEntry {
        account: ALL_TRANSACTIONS.to_string(),
        name: i18n!("All transactions"),
        balance: String::new(),
    }];
    for a in client.list_accounts(None).await? {
        entries.push(SidebarEntry {
            name: a.friendly_name.clone().unwrap_or_else(|| a.account.clone()),
            balance: join_balances(&a.balances),
            account: a.account,
        });
    }
    Ok(entries)
}

/// One register for any account, newest first.
pub async fn register(
    client: &KabacusClient,
    account: &str,
) -> Result<Vec<RegisterRow>, ModelError> {
    let mut rows: Vec<RegisterRow> = if account == ALL_TRANSACTIONS {
        client
            .list_transactions()
            .await?
            .into_iter()
            .map(|t| {
                // Source → destination by sign, not by posting order (the
                // journal writes them either way round).
                let side = |outgoing: bool| {
                    t.postings
                        .iter()
                        .filter(|p| p.amount.exact.starts_with('-') == outgoing)
                        .map(|p| p.account.as_str())
                        .collect::<Vec<_>>()
                        .join(&list_separator())
                };
                RegisterRow {
                    id: t.id.to_string(),
                    date: t.date.clone(),
                    payee: t.payee.clone(),
                    // The arrow is the translator's: a right-to-left
                    // language points it the other way.
                    account: i18nc!(
                        "%1 is the account money came from, %2 the account it went to",
                        "%1 → %2",
                        side(true),
                        side(false)
                    ),
                    memo: t.memo.clone(),
                    outflow: String::new(),
                    inflow: String::new(),
                    balance: String::new(),
                    status: t.status.clone(),
                }
            })
            .collect()
    } else {
        client
            .get_register(account, None, None)
            .await?
            .into_iter()
            .map(|l| {
                let (outflow, inflow) = split_flow(&l.amount);
                RegisterRow {
                    id: l.id.to_string(),
                    date: l.date,
                    payee: l.payee,
                    account: l.other_account,
                    memo: l.memo,
                    outflow,
                    inflow,
                    balance: l.balance.formatted,
                    status: l.status,
                }
            })
            .collect()
    };
    rows.reverse();
    Ok(rows)
}

impl SimpleDraft {
    /// The postings to send, or why the draft can't be saved yet. The
    /// server validates again in full; this only catches what the
    /// form itself can see.
    pub fn to_body(&self, commodity: &str) -> Result<AddTransactionBody, ModelError> {
        let invalid = |m: String| Err(ModelError::Invalid(m));
        if kabacus_core::validate::parse_date(self.date.trim()).is_err() {
            return invalid(i18n!("Date must be YYYY-MM-DD"));
        }
        if self.payee.trim().is_empty() {
            return invalid(i18nc!(
                "%1 is a form field",
                "%1 is required",
                i18n!("Payee")
            ));
        }
        if self.from_account.is_empty() || self.to_account.is_empty() {
            return invalid(i18n!("Choose both accounts"));
        }
        if self.from_account == self.to_account {
            return invalid(i18n!("From and To must differ"));
        }
        let amount: Money = self
            .amount
            .parse()
            .map_err(|e: kabacus_core::AmountError| ModelError::Invalid(e.to_string()))?;
        if !amount.is_positive() {
            return invalid(i18n!("Amount must be more than zero"));
        }
        let posting = |account: &str, money: Money| DraftPostingBody {
            account: account.to_string(),
            amount: Some(DraftAmount::from_money(commodity, money)),
            status: String::new(),
            balance_assertion: None,
            clear_balance_assertion: false,
        };
        let description = if self.memo.trim().is_empty() {
            self.payee.trim().to_string()
        } else {
            format!("{} | {}", self.payee.trim(), self.memo.trim())
        };
        Ok(AddTransactionBody {
            new_accounts: vec![],
            date: self.date.trim().to_string(),
            description,
            postings: vec![
                posting(&self.to_account, amount.clone()),
                posting(&self.from_account, -amount),
            ],
            tags: vec![],
            dry_run: false,
        })
    }
}

/// The simple transaction form: validate, then add against the current HEAD. The
/// commodity is the one the From account already holds (else `USD`).
pub async fn add_simple(client: &KabacusClient, draft: &SimpleDraft) -> Result<String, ModelError> {
    let accounts = client.list_accounts(None).await?;
    let commodity = accounts
        .iter()
        .find(|a| a.account == draft.from_account)
        .and_then(|a| a.balances.first())
        .map(|b| b.commodity.clone())
        .unwrap_or_else(|| "USD".to_string());
    let body = draft.to_body(&commodity)?;
    let head = client
        .version()
        .await?
        .head
        .ok_or_else(|| ModelError::Invalid(i18n!("The books have no revisions yet")))?;
    Ok(client
        .add_transaction(&head, body)
        .await?
        .id
        .map(|id| id.to_string())
        .unwrap_or_default())
}

/// The editor's form for an existing transaction. Only simple
/// (two-posting, one-commodity) transactions can be edited this way yet.
pub async fn edit_draft(client: &KabacusClient, id: &str) -> Result<SimpleDraft, ModelError> {
    let tx = find_transaction(client, id).await?;
    let [a, b] = tx.postings.as_slice() else {
        return Err(ModelError::Invalid(i18n!(
            "Split transactions can't be edited here yet"
        )));
    };
    if a.amount.commodity != b.amount.commodity {
        return Err(ModelError::Invalid(i18n!(
            "Multi-currency transactions can't be edited here yet"
        )));
    }
    let (from, to) = if a.amount.exact.starts_with('-') {
        (a, b)
    } else {
        (b, a)
    };
    Ok(SimpleDraft {
        date: tx.date.clone(),
        payee: tx.payee.clone(),
        memo: tx.memo.clone(),
        from_account: from.account.clone(),
        to_account: to.account.clone(),
        amount: to.amount.exact.trim_start_matches('-').to_string(),
    })
}

/// Save an edited transaction in place. Keeps its tags and, for
/// accounts it still uses, each posting's cleared/pending mark.
pub async fn save_edit(
    client: &KabacusClient,
    id: &str,
    draft: &SimpleDraft,
) -> Result<(), ModelError> {
    let tx = find_transaction(client, id).await?;
    let commodity = tx
        .postings
        .first()
        .map(|p| p.amount.commodity.clone())
        .unwrap_or_else(|| "USD".to_string());
    let mut body = draft.to_body(&commodity)?;
    for posting in &mut body.postings {
        if let Some(old) = tx.postings.iter().find(|p| p.account == posting.account) {
            posting.status = old.status.clone();
        }
    }
    // Keep the journal's posting order: `to_body` writes To first, so flip
    // it when the original had the From side first.
    if tx
        .postings
        .first()
        .is_some_and(|p| p.amount.exact.starts_with('-'))
    {
        body.postings.reverse();
    }
    body.tags = tx.tags.clone();
    let head = client
        .version()
        .await?
        .head
        .ok_or_else(|| ModelError::Invalid(i18n!("The books have no revisions yet")))?;
    client
        .edit_transaction(transaction_id(id)?, &head, body)
        .await?;
    Ok(())
}

pub(crate) async fn find_transaction(
    client: &KabacusClient,
    id: &str,
) -> Result<kabacus_client::types::TransactionView, ModelError> {
    client
        .list_transactions()
        .await?
        .into_iter()
        .find(|t| is_transaction(t, id))
        .ok_or_else(|| {
            ModelError::Invalid(i18nc!(
                "%1 is a transaction id",
                "Transaction %1 wasn't found",
                id
            ))
        })
}

/// `id` names `t`.
pub fn is_transaction(t: &kabacus_client::types::TransactionView, id: &str) -> bool {
    t.id.to_string() == id
}

/// A transaction id as the views hold it, for the client.
pub fn transaction_id(id: &str) -> Result<u64, ModelError> {
    id.parse().map_err(|_| {
        ModelError::Invalid(i18nc!(
            "%1 is a transaction id",
            "Transaction %1 wasn't found",
            id
        ))
    })
}

#[cfg(test)]
mod user_message_tests {
    use super::*;
    #[test]
    fn each_failure_is_one_plain_sentence() {
        let m = |e: ClientError| ModelError::from(e).user_message("/run/k.sock");
        let unreachable = m(ClientError::Request(
            "reqwest::Error { kind: Request, source: Os { code: 2 } }".into(),
        ));
        assert_eq!(
            unreachable,
            "Can't reach kabacus-server at /run/k.sock. Is it running?"
        );
        assert_eq!(
            ModelError::from(ClientError::Request("x".into())).user_message(""),
            "Can't reach kabacus-server. Is it running?"
        );
        assert_eq!(
            m(ClientError::Timeout),
            "The server is taking too long to answer."
        );
        assert_eq!(
            m(ClientError::Conflict {
                expected: "a".into(),
                current: "b".into()
            }),
            "The books changed while you were editing. Review again."
        );
        assert_eq!(
            m(ClientError::Invalid {
                rule: "amount_format".into(),
                message: "Use a decimal point: 23.40".into(),
                failures: Vec::new(),
                details: serde_json::Value::Null,
            }),
            "Use a decimal point: 23.40",
            "no `invalid (rule):` prefix"
        );
        assert_eq!(
            m(ClientError::Server("<html>500</html>".into())),
            "The server had a problem."
        );
        assert_eq!(
            m(ClientError::Forbidden("ledger-write".into())),
            "You can view these books but not change them."
        );
    }
}
