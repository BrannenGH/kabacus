// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `kabacus-server`: the gateway daemon. Only this crate touches
//! the books: the app talks to its HTTP API, and MCP clients to
//! the MCP endpoint it serves alongside.

pub mod account_edit;
pub mod api;
pub mod auth;
pub mod batch;
pub mod books;
pub mod cache;
pub mod config;
pub mod diff;
pub mod import;
pub mod mcp;
pub mod ops;
pub mod remote;
pub mod reports;
pub mod search;
pub mod service;
pub mod split;
pub mod state;
pub mod web;
