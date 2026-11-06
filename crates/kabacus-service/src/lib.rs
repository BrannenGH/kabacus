// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared reads, reports and validated writes over a book store.

pub mod account_edit;
pub mod batch;
pub mod books;
pub mod cache;
pub mod caller;
pub mod commands;
pub mod diff;
pub mod error;
pub mod import;
pub mod ops;
pub mod reports;
pub mod search;
pub mod service;
pub mod split;
pub mod state;

pub use state::Service;

pub mod catalog;
pub mod dispatch;
pub mod local;

pub mod script;

pub mod setup;
