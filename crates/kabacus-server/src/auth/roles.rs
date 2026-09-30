// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The three Kabacus roles, resolved from a configurable claim
//! through `role_map`.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Read,
    Write,
    Rewrite,
}

impl Role {
    pub fn from_canonical_name(s: &str) -> Option<Role> {
        match s {
            "ledger-read" => Some(Role::Read),
            "ledger-write" => Some(Role::Write),
            "ledger-rewrite" => Some(Role::Rewrite),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Role::Read => "ledger-read",
            Role::Write => "ledger-write",
            Role::Rewrite => "ledger-rewrite",
        }
    }
}

/// Which of the server's surfaces a request came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    /// The HTTP API.
    #[default]
    Api,
    /// The `/mcp` endpoint.
    Mcp,
}

/// The authenticated (or, in no-auth mode, assumed) caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Who they are at the issuer (`sub`); in no-auth mode, `local`.
    pub subject: String,
    pub username: String,
    /// The raw client id from the token (or a client-supplied header in
    /// no-auth mode) — `"kabacus-app"`, an MCP client's id, etc.
    pub client: String,
    pub session: Option<String>,
    pub issuer: String,
    pub roles: HashSet<Role>,
    pub auth_none: bool,
    /// Set by `/mcp`'s middleware; [`Channel::Api`] otherwise.
    pub channel: Channel,
}

impl Principal {
    pub fn has_role(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }
}

/// Map the provider's raw role/group/scope values through
/// `role_map` into Kabacus's own role set.
pub fn resolve_roles(raw_values: &[String], role_map: &HashMap<String, String>) -> HashSet<Role> {
    raw_values
        .iter()
        .filter_map(|raw| role_map.get(raw))
        .filter_map(|canonical| Role::from_canonical_name(canonical))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_provider_values_through_role_map() {
        let mut map = HashMap::new();
        map.insert("ledger-read".to_string(), "ledger-read".to_string());
        map.insert("finance-write".to_string(), "ledger-write".to_string());
        let roles = resolve_roles(
            &[
                "ledger-read".to_string(),
                "finance-write".to_string(),
                "unrelated".to_string(),
            ],
            &map,
        );
        assert_eq!(roles, HashSet::from([Role::Read, Role::Write]));
    }
}
