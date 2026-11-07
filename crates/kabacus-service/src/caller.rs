// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Caller-supplied authorization and audit identity.

use crate::error::Error;
use kabacus_store::{AnyStore, UserId};
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, Copy)]
pub enum Permission {
    Read,
    Write,
    Rewrite,
}

/// Authorization and attribution supplied by the embedding application.
pub trait Caller: Send + Sync {
    /// Refuse an operation before any book data is returned or changed.
    fn require(&self, permission: Permission) -> Result<(), Error>;
    /// Resolve the user whose id the store records in its audit log.
    fn user<'a>(
        &'a self,
        store: &'a AnyStore,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UserId>> + Send + 'a>>;
    /// The audit actor, using the configured application client hint if needed.
    fn actor(&self, app_client_id: Option<&str>) -> String;
    /// Additional audit metadata; never include credentials.
    fn details(&self) -> serde_json::Value;
}
