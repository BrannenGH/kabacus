// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Extracting configurable claims from a token's JSON payload, including
//! the dot-path roles claim (`resource_access.kabacus-server.roles`) that
//! lets Kabacus read roles wherever a given provider puts them — client
//! roles, realm roles, groups or scopes.

use serde_json::Value;

/// Walk a dot-separated path through nested JSON objects, e.g.
/// `"resource_access.kabacus-server.roles"`.
pub fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |v, key| v.as_object()?.get(key))
}

/// A claim that's either a single string (e.g. a space-separated `scope`
/// claim) or a JSON array of strings (e.g. `roles`, `groups`).
pub fn get_string_list(value: &Value, path: &str) -> Vec<String> {
    match get_path(value, path) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => s.split_whitespace().map(str::to_string).collect(),
        _ => vec![],
    }
}

pub fn get_string<'a>(value: &'a Value, path: &str) -> Option<&'a str> {
    get_path(value, path).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reads_nested_dot_path_roles() {
        let claims = json!({
            "resource_access": { "kabacus-server": { "roles": ["ledger-read", "ledger-write"] } }
        });
        assert_eq!(
            get_string_list(&claims, "resource_access.kabacus-server.roles"),
            vec!["ledger-read", "ledger-write"]
        );
    }
    #[test]
    fn reads_flat_groups_claim() {
        let claims = json!({ "groups": ["ledger-read"] });
        assert_eq!(get_string_list(&claims, "groups"), vec!["ledger-read"]);
    }
    #[test]
    fn reads_space_separated_scope_string() {
        let claims = json!({ "scope": "ledger-read ledger-write" });
        assert_eq!(
            get_string_list(&claims, "scope"),
            vec!["ledger-read", "ledger-write"]
        );
    }
    #[test]
    fn reads_username_and_falls_back_gracefully() {
        let claims = json!({ "preferred_username": "alice", "sub": "abc" });
        assert_eq!(get_string(&claims, "preferred_username"), Some("alice"));
        assert_eq!(get_string(&claims, "missing"), None);
    }

    #[test]
    fn missing_path_segment_returns_none() {
        let claims = json!({ "a": { "b": 1 } });
        assert_eq!(get_path(&claims, "a.b.c"), None);
        assert_eq!(get_path(&claims, "a.x"), None);
    }
}
