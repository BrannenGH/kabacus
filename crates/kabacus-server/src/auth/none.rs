// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--auth=none`, the explicit test/local mode. Every request is
//! treated as one local principal; the client id comes from a
//! client-supplied header, for the audit trail.

use super::roles::{Channel, Principal, Role};
use std::collections::HashSet;

pub const DEFAULT_ROLES: &[&str] = &["ledger-read", "ledger-write"];

pub fn no_auth_principal(configured_roles: &[String], client_header: Option<&str>) -> Principal {
    let role_names: Vec<&str> = if configured_roles.is_empty() {
        DEFAULT_ROLES.to_vec()
    } else {
        configured_roles.iter().map(String::as_str).collect()
    };
    let roles: HashSet<Role> = role_names
        .iter()
        .filter_map(|r| Role::from_canonical_name(r))
        .collect();
    let client = client_header.unwrap_or("unknown").to_string();

    Principal {
        subject: "local".to_string(),
        username: "local".to_string(),
        client,
        session: None,
        issuer: "none".to_string(),
        roles,
        auth_none: true,
        channel: Channel::Api,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_roles_are_read_and_write() {
        let p = no_auth_principal(&[], None);
        assert!(p.has_role(Role::Read));
        assert!(p.has_role(Role::Write));
        assert!(!p.has_role(Role::Rewrite));
        assert_eq!(p.username, "local");
        assert!(p.auth_none);
    }
    #[test]
    fn no_auth_roles_flag_can_grant_rewrite() {
        let p = no_auth_principal(
            &[
                "ledger-read".to_string(),
                "ledger-write".to_string(),
                "ledger-rewrite".to_string(),
            ],
            Some("app"),
        );
        assert!(p.has_role(Role::Rewrite));
    }
}
