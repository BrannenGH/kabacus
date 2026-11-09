// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus-core`: no I/O, no UI.
//!
//! Everything here is deterministic domain logic shared by `kabacus-server`
//! (which enforces it server-side for both its API and its MCP
//! endpoint) and the app (which uses it for live validation and identical
//! counterparty/statement behaviour). Money is always exact; nothing in this
//! crate ever constructs a `Money` from a float.

pub mod accounts;
pub mod comment;
pub mod correction;
pub mod counterparty;
pub mod domain;
pub mod format;
pub mod funds;
pub mod impact;
pub mod kinds;
pub mod money;
pub mod payee;
pub mod reports;
pub mod statement_export;
pub mod validate;

pub use accounts::{
    parse_account_declarations, rank_accounts, render_account_declaration, AccountDeclaration,
    PickerEntry,
};
pub use comment::{declared_tags, reconcile_tags};
pub use correction::{build_reversal, reversal_note};
pub use counterparty::{
    build_statement, list_counterparties, select_counterparty_sides, CounterpartyNet,
    FlowDirection, Statement, StatementLine,
};
pub use domain::{
    AccountName, BalanceAssertion, Cost, CostKind, Posting, PostingKind, Status, Tag, Transaction,
};
pub use format::format_money;
pub use funds::{
    card_reserves, cash_on_hand, check_new_fund, new_fund_account, plan_allocation,
    plan_card_payment, plan_card_purchase, unreserved_flags, validation_warnings, Balances,
    CardLine, CardReserve, CashOnHand, Chart, FundsError, MissingReserve, PlannedTransaction,
    PurchaseSource, ReserveSource, Warning, DEFAULT_RESERVE_TEMPLATE, RESERVE_SPLIT_RULE,
};
pub use impact::{balance_impact, BalanceChange};
pub use kinds::{check_kind_rules, default_kind_rules, AccountKind, ClassifiedAccount, KindRule};
pub use money::{parse_amount, Amount, AmountError, Money};
pub use num_bigint::BigInt;
pub use payee::{known_payees, last_transaction_for, KnownPayee};
pub use statement_export::{statement_csv, statement_filename, statement_pdf, StatementHeader};
pub use validate::{
    carry_over, carry_over_with, check_accounts_declared, check_balance_assertions,
    check_description, parse_date, resolve_amounts, resolve_amounts_by_kind, AssertionEdit,
    BalanceAssertionFailure, CarriedOver, CostEdit, DraftPosting, ValidationError,
};
