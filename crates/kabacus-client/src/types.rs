// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Wire types for the kabacus-server API. Deliberately not shared
//! with kabacus-server's own internal types — the HTTP contract is the
//! only coupling between them.

use kabacus_core::Money;
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmountView {
    pub commodity: String,
    pub exact: String,
    pub formatted: String,
}

impl AmountView {
    /// Parse the exact decimal string back into a [`Money`] (never
    /// through the formatted/float-ish string).
    pub fn money(&self) -> Option<Money> {
        let (whole, frac) = self
            .exact
            .trim_start_matches('-')
            .split_once('.')
            .unwrap_or((self.exact.trim_start_matches('-'), ""));
        let places = frac.len() as u32;
        let digits = format!("{whole}{frac}");
        let mantissa: BigInt = digits.parse().ok()?;
        Some(Money::from_big(
            if self.exact.starts_with('-') {
                -mantissa
            } else {
                mantissa
            },
            places,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostingView {
    pub account: String,
    pub amount: AmountView,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionView {
    /// The transaction's own number in the books.
    pub id: u64,
    pub date: String,
    pub payee: String,
    pub memo: String,
    pub status: String,
    /// The transaction's comment, tags included.
    #[serde(default)]
    pub comment: String,
    pub postings: Vec<PostingView>,
    pub tags: Vec<(String, String)>,
    /// It posts to a card without the matching reserve line.
    #[serde(default)]
    pub unreserved: bool,
    #[serde(default)]
    pub missing_reserve: Vec<MissingReserveView>,
}

/// What a card transaction didn't post to the card's reserve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingReserveView {
    pub card: String,
    pub reserve: String,
    pub amount: AmountView,
}

#[derive(Debug, Clone, Serialize)]
pub struct DraftAmount {
    pub commodity: String,
    #[serde(with = "kabacus_core::money::wire_mantissa")]
    pub mantissa: BigInt,
    pub places: u32,
}

impl DraftAmount {
    pub fn from_money(commodity: impl Into<String>, money: Money) -> Self {
        DraftAmount {
            commodity: commodity.into(),
            mantissa: money.mantissa().clone(),
            places: money.places(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DraftPostingBody {
    pub account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<DraftAmount>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// The balance to assert after this posting. On an edit, `None` (with
    /// `clear_balance_assertion` false) keeps the posting's current one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_assertion: Option<DraftAmount>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub clear_balance_assertion: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddTransactionBody {
    pub date: String,
    pub description: String,
    pub postings: Vec<DraftPostingBody>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<(String, String)>,
    /// New expense or income accounts the postings use.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_accounts: Vec<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorrectionBody {
    pub void: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postings: Option<Vec<DraftPostingBody>>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResultView {
    /// The transaction written; none for an account operation.
    #[serde(default)]
    pub id: Option<u64>,
    pub head: String,
    #[serde(default)]
    pub dry_run: bool,
    /// The (first) transaction written: its text before and after.
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    /// Every transaction written, when there is more than one: the first
    /// few.
    #[serde(default)]
    pub transactions: Vec<ChangeView>,
    /// How many changes the write makes, when there are more than shown.
    #[serde(default)]
    pub changed: Option<usize>,
    pub preview: String,
    /// Each touched account's balance before and after.
    #[serde(default)]
    pub balances: Vec<BalanceChangeView>,
    #[serde(default)]
    pub unreserved: bool,
    #[serde(default)]
    pub missing_reserve: Vec<MissingReserveView>,
    #[serde(default)]
    pub warnings: Vec<serde_json::Value>,
}

/// One transaction's change in a write's preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeView {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceChangeView {
    pub account: String,
    pub before: AmountView,
    pub after: AmountView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitView {
    /// The revision, as `head` takes it.
    pub hash: String,
    pub revision: i64,
    /// When it was written, RFC 3339.
    #[serde(default)]
    pub date: String,
    pub summary: String,
    pub actor: Option<String>,
    pub kind: Option<String>,
    /// Who made it.
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterpartyView {
    pub name: String,
    pub net: AmountView,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementLineView {
    pub date: String,
    pub description: String,
    pub net_amount: AmountView,
    pub running_balance: AmountView,
    pub open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementView {
    pub name: String,
    pub from: String,
    pub to: String,
    pub opening_balance: AmountView,
    pub closing_balance: AmountView,
    pub lines: Vec<StatementLineView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfoView {
    pub mode: String,
    pub issuer: Option<String>,
    pub app_client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionView {
    #[serde(default)]
    pub ledger: i64,
    pub server_version: String,
    /// The exact commit the server was built from, and where its source is.
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub source_url: String,
    pub head: Option<String>,
    /// Moves whenever the books change, hand edits included.
    #[serde(default)]
    pub books: Option<String>,
    pub auth: AuthInfoView,
}

/// The books' commodity and the decimal places it takes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommodityView {
    pub symbol: String,
    pub places: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountView {
    pub account: String,
    /// The YNAB account/envelope name from the declaration, if any.
    #[serde(default)]
    pub friendly_name: Option<String>,
    /// The name to show.
    #[serde(default)]
    pub display_name: String,
    /// `money`, `pre-allocation` (a fund), `debt-reserve` (a card
    /// reserve), `revolving-debt` (a card), `expense`, ...
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default)]
    pub physical_home: Option<String>,
    /// For a fund or card reserve, the money account holding it.
    #[serde(default)]
    pub home: String,
    pub balances: Vec<AmountView>,
    /// Without sub-accounts: a money account's free money.
    #[serde(default)]
    pub balance_exclusive: Vec<AmountView>,
    /// With every sub-account: what the bank shows.
    #[serde(default)]
    pub balance_inclusive: Vec<AmountView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAccountTypeBody {
    pub synthetic: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterLineView {
    pub id: u64,
    pub date: String,
    pub payee: String,
    pub memo: String,
    pub other_account: String,
    pub amount: AmountView,
    pub balance: AmountView,
    pub status: String,
    #[serde(default)]
    pub unreserved: bool,
    #[serde(default)]
    pub missing_reserve: Vec<MissingReserveView>,
}

/// A warning that puts Cash on Hand at risk; never a refusal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningView {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub accounts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalanceView {
    pub account: String,
    pub display_name: String,
    pub balance: AmountView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KindGroupView {
    pub kind: String,
    pub total: AmountView,
    pub accounts: Vec<AccountBalanceView>,
}

/// A money account: its free money, funds and reserves, and what the bank
/// shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeView {
    pub account: String,
    pub display_name: String,
    pub free: AmountView,
    pub funds: AmountView,
    pub reserves: AmountView,
    pub inclusive: AmountView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundView {
    pub account: String,
    pub display_name: String,
    pub home: String,
    pub balance: AmountView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreservedTransactionView {
    pub id: u64,
    pub date: String,
    pub description: String,
    pub missing_reserve: AmountView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardView {
    pub card: String,
    pub display_name: String,
    pub balance: AmountView,
    pub owed: AmountView,
    pub reserve: String,
    pub reserve_exists: bool,
    pub reserve_balance: AmountView,
    /// What the reserve still needs; zero when it covers the card.
    pub difference: AmountView,
    pub unreserved_count: usize,
    #[serde(default)]
    pub unreserved: Option<Vec<UnreservedTransactionView>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashChangeView {
    pub compare_to: String,
    pub money_on_hand: AmountView,
    pub cash_on_hand: AmountView,
}

/// Cash on Hand and its parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashOnHandView {
    pub as_of: Option<String>,
    pub money_on_hand: AmountView,
    pub future_allocations: AmountView,
    pub receivables: AmountView,
    pub cash_on_hand: AmountView,
    #[serde(default)]
    pub change: Option<CashChangeView>,
    pub by_kind: Vec<KindGroupView>,
    pub homes: Vec<HomeView>,
    pub funds: Vec<FundView>,
    pub cards: Vec<CardView>,
    pub unreserved_count: usize,
    pub warnings: Vec<WarningView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveSourceView {
    /// `free`, `fund` or `unattributed`.
    pub source: String,
    #[serde(default)]
    pub fund: Option<String>,
    pub name: String,
    pub amount: AmountView,
}

/// A card, its reserve, and what the reserve holds by source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReserveView {
    #[serde(flatten)]
    pub card: CardView,
    pub matches: bool,
    pub since: Option<String>,
    pub sources: Vec<ReserveSourceView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReservesView {
    pub rule: String,
    pub cards: Vec<CardReserveView>,
}

/// Move money between a home's free money, funds and reserves.
#[derive(Debug, Clone, Serialize)]
pub struct AllocateBody {
    pub from: String,
    pub to: String,
    /// An exact decimal string, e.g. `"100.00"`.
    pub amount: String,
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `to` is a new fund directly in the money account `from`, made by
    /// this write.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub new_fund: bool,
    pub dry_run: bool,
}

/// A purchase on a card with its reserve lines.
#[derive(Debug, Clone, Serialize)]
pub struct CardPurchaseBody {
    pub card: String,
    pub expense_account: String,
    pub amount: String,
    pub date: String,
    pub description: String,
    /// `free`, or the fund paying for it.
    pub source: String,
    pub dry_run: bool,
}

/// Pay a card from its reserve.
#[derive(Debug, Clone, Serialize)]
pub struct CardPaymentBody {
    pub card: String,
    pub amount: String,
    pub date: String,
    pub dry_run: bool,
}

/// Compare an account with a statement.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub dry_run: bool,
    pub account: String,
    pub balance: String,
    pub date: String,
    /// `check` or `assert`.
    pub mode: String,
}

/// One operation in a batch.
#[derive(Debug, Clone, Serialize)]
pub struct BatchOpBody {
    pub tool: String,
    pub args: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_view_parses_exact_string_back_to_money() {
        let v = AmountView {
            commodity: "USD".to_string(),
            exact: "-45.678".to_string(),
            formatted: "-$45.678".to_string(),
        };
        let m = v.money().unwrap();
        assert_eq!(m.to_string(), "-45.678");
    }

    #[test]
    fn amount_view_parses_whole_numbers() {
        let v = AmountView {
            commodity: "USD".to_string(),
            exact: "100".to_string(),
            formatted: "$100".to_string(),
        };
        assert_eq!(v.money().unwrap().to_string(), "100");
    }
}

/// A known payee and the transaction to pre-fill from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayeeView {
    pub payee: String,
    pub count: usize,
    pub last: TransactionView,
}
