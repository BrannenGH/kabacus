// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! HTTP, MCP and sign-in around the shared book service.

pub mod account_edit;
pub mod api;
pub mod auth;
pub mod batch;
pub mod books;
pub mod cache;
mod caller;
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
