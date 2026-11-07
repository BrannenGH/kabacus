// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus-ledger`: Kabacus's own ledger engine. It reads journals into a
//! [`Book`], fills in omitted amounts, checks a book strictly (balancing,
//! balance assertions at the end of each day, declared commodities and
//! accounts) and renders journal text. Pure functions on in-memory values;
//! the only I/O is reading journals through an [`ImportSource`].
//!
//! Implements the engine implementation specification, section 2. Every
//! module is written from the specification's ledger-spec pages only, never
//! from hledger's source or tests; its header names what it implements.

pub mod amount;
pub mod balance;
pub mod book;
pub mod check;
pub mod import;
pub mod render;

pub use amount::{parse_amount, render_amount, style_from_example, Side, Style, MAX_PLACES};
pub use balance::{infer, Unbalanced, Written};
pub use book::{style_in, Account, Book, Commodity, Styles};
pub use check::{
    check, check_change, places_needed, Change, CheckFailure, FailureKind, PlacesNeeded, Position,
};
pub use import::{
    files, import, read, FsSource, ImportError, ImportOutcome, ImportReport, ImportSource,
    MemorySource, Noted, ReportEntry,
};
pub use render::{render_book, render_money, render_posting, render_transaction};

/// The version of the whole app: `KABACUS_VERSION` when the build sets it
/// (scripts/package-version), else the crate's.
pub const VERSION: &str = match option_env!("KABACUS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
