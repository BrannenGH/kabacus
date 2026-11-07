// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared MCP tools for the server, headless local books and attached apps.

pub mod skills;
pub mod tools;
use kabacus_service::caller::Caller;
use std::{future::Future, pin::Pin, sync::Arc};
pub use tools::{tool_list_hash, KabacusMcp, REWRITE_TOOLS};

#[derive(Clone)]
pub struct CallerHandle(pub Arc<dyn Caller>);

pub trait Metadata: Send + Sync {
    fn version(&self) -> Pin<Box<dyn Future<Output = serde_json::Value> + Send + '_>>;
}

/// The tool list each client (user and client id) last listed, kept in
/// memory, so a client that comes back after the tools changed is told at
/// once; and a signal to every open `subscriptions/listen` when they
/// change.
#[derive(Default)]
pub struct ToolLists {
    seen: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    changed: std::sync::OnceLock<tokio::sync::watch::Sender<u64>>,
}

impl ToolLists {
    pub fn new() -> ToolLists {
        ToolLists::default()
    }

    fn changed(&self) -> &tokio::sync::watch::Sender<u64> {
        self.changed
            .get_or_init(|| tokio::sync::watch::channel(0).0)
    }

    fn key(principal: &dyn kabacus_service::caller::Caller) -> String {
        {
            let details = principal.details();
            format!(
                "{}\u{1f}{}",
                details["user"].as_str().unwrap_or_default(),
                details["client"].as_str().unwrap_or_default()
            )
        }
    }

    /// The hash of the tool list `principal`'s client last listed.
    pub fn seen(&self, principal: &dyn kabacus_service::caller::Caller) -> Option<String> {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&Self::key(principal))
            .cloned()
    }

    pub fn set_seen(&self, principal: &dyn kabacus_service::caller::Caller, hash: &str) {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(Self::key(principal), hash.to_string());
    }

    /// Tell every open subscription the tools changed.
    pub fn notify_all(&self) {
        self.changed().send_modify(|n| *n += 1);
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed().subscribe()
    }
}

/// Adds the connected agent to audit metadata while preserving authenticated identity and permissions.
pub struct AgentCaller {
    pub caller: Arc<dyn Caller>,
    pub agent: String,
}
impl Caller for AgentCaller {
    fn require(
        &self,
        permission: kabacus_service::caller::Permission,
    ) -> Result<(), kabacus_service::error::Error> {
        self.caller.require(permission)
    }
    fn user<'a>(
        &'a self,
        store: &'a kabacus_store::AnyStore,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<kabacus_store::UserId>> + Send + 'a>> {
        self.caller.user(store)
    }
    fn actor(&self, _: Option<&str>) -> String {
        "mcp".into()
    }
    fn details(&self) -> serde_json::Value {
        let mut details = self.caller.details();
        details["agent"] = serde_json::json!(self.agent);
        details
    }
}
