// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Identity and permissions for an application using local books.

use crate::{
    caller::{Caller, Permission},
    error::{Error, ErrorBody},
};
use kabacus_store::{AnyStore, BookStore, UserId};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCaller {
    pub username: String,
    pub actor: String,
    pub agent: Option<String>,
    pub read_only: bool,
    pub allow_rewrites: bool,
}
impl LocalCaller {
    pub fn new(username: impl Into<String>, actor: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            actor: actor.into(),
            agent: None,
            read_only: false,
            allow_rewrites: false,
        }
    }
    pub fn for_app(username: impl Into<String>) -> Self {
        let mut caller = Self::new(username, "app");
        caller.allow_rewrites = true;
        caller
    }
    pub fn display_name(&self) -> String {
        match &self.agent {
            Some(name) => format!("{} via {} (agent)", self.username, name),
            None => self.username.clone(),
        }
    }
}
impl Caller for LocalCaller {
    fn require(&self, permission: Permission) -> Result<(), Error> {
        let allowed = match permission {
            Permission::Read => true,
            Permission::Write => !self.read_only,
            Permission::Rewrite => !self.read_only && self.allow_rewrites,
        };
        if allowed {
            return Ok(());
        }
        let message = if self.read_only {
            "These books are open read-only."
        } else {
            "Rewrites are disabled. Enable rewrites before making this change."
        };
        Err(Error::Denied {
            permission,
            body: ErrorBody {
                code: "role_required".into(),
                message: message.into(),
                path: None,
                details: None,
            },
        })
    }
    fn user<'a>(
        &'a self,
        store: &'a AnyStore,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UserId>> + Send + 'a>> {
        Box::pin(async move { Ok(store.local_user(&self.display_name()).await?) })
    }
    fn actor(&self, _: Option<&str>) -> String {
        self.actor.clone()
    }
    fn details(&self) -> serde_json::Value {
        serde_json::json!({"user": self.username, "client": self.actor, "auth": "none", "session": "none", "agent": self.agent})
    }
}
