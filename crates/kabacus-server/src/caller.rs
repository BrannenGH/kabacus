// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Adapt authenticated callers to the shared service boundary.

use crate::{
    api::ApiError,
    auth::roles::{Channel, Principal, Role},
};
use kabacus_service::{
    caller::{Caller, Permission},
    error::Error,
};
use kabacus_store::{AnyStore, BookStore, UserId};
use std::{future::Future, pin::Pin};

impl Caller for Principal {
    fn require(&self, permission: Permission) -> Result<(), Error> {
        let role = match permission {
            Permission::Read => Role::Read,
            Permission::Write => Role::Write,
            Permission::Rewrite => Role::Rewrite,
        };
        if self.has_role(role) {
            Ok(())
        } else {
            Err(Error::Denied {
                permission,
                body: ApiError::MissingRole(role).body(),
            })
        }
    }

    fn user<'a>(
        &'a self,
        store: &'a AnyStore,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UserId>> + Send + 'a>> {
        Box::pin(async move {
            Ok(if self.auth_none {
                store.local_user(&self.username).await?
            } else {
                store
                    .user_for_identity(&self.issuer, &self.subject, &self.username)
                    .await?
            })
        })
    }

    fn actor(&self, app_client_id: Option<&str>) -> String {
        if self.channel == Channel::Mcp {
            "mcp".to_string()
        } else if Some(self.client.as_str()) == app_client_id {
            "app".to_string()
        } else {
            self.client.clone()
        }
    }

    fn details(&self) -> serde_json::Value {
        let mut details = serde_json::json!({
            "user": self.username, "client": self.client,
            "session": self.session.clone().unwrap_or_else(|| "none".to_string()),
        });
        if self.auth_none {
            details["auth"] = serde_json::json!("none");
        } else {
            details["issuer"] = serde_json::json!(self.issuer);
        }
        details
    }
}

impl kabacus_mcp::Metadata for crate::state::AppState {
    fn version(&self) -> Pin<Box<dyn Future<Output = serde_json::Value> + Send + '_>> {
        Box::pin(async move {
            serde_json::to_value(crate::service::version(self).await).unwrap_or_default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn dana() -> Principal {
        Principal {
            subject: "dana-okafor".into(),
            username: "Dana Okafor".into(),
            client: "kabacus-app".into(),
            session: Some("dana-laptop-session".into()),
            issuer: "https://identity.example.org".into(),
            roles: HashSet::from([Role::Read]),
            auth_none: false,
            channel: Channel::Api,
        }
    }

    #[test]
    fn denied_permissions_keep_the_role_error_on_both_surfaces() {
        let principal = dana();
        assert!(principal.require(Permission::Read).is_ok());
        for (permission, role) in [
            (Permission::Write, Role::Write),
            (Permission::Rewrite, Role::Rewrite),
        ] {
            let error = principal.require(permission).unwrap_err();
            let expected = serde_json::to_value(ApiError::MissingRole(role).body()).unwrap();
            assert_eq!(serde_json::to_value(error.body()).unwrap(), expected);
            let error = ApiError::from(error);
            assert_eq!(serde_json::to_value(error.body()).unwrap(), expected);
            assert!(matches!(error, ApiError::MissingRole(r) if r == role));
        }
    }

    #[tokio::test]
    async fn local_and_external_callers_keep_their_audit_identity() {
        let store = AnyStore::Sqlite(kabacus_store::SqliteStore::in_memory().unwrap());
        let mut principal = dana();
        let external = principal.user(&store).await.unwrap();
        assert_eq!(external, principal.user(&store).await.unwrap());
        assert_eq!(principal.actor(Some("kabacus-app")), "app");
        assert_eq!(
            principal.details(),
            serde_json::json!({
                "user": "Dana Okafor", "client": "kabacus-app", "session": "dana-laptop-session",
                "issuer": "https://identity.example.org",
            })
        );
        principal.channel = Channel::Mcp;
        assert_eq!(principal.actor(Some("kabacus-app")), "mcp");
        principal.channel = Channel::Api;
        principal.client = "household-export".into();
        assert_eq!(principal.actor(Some("kabacus-app")), "household-export");
        principal.auth_none = true;
        principal.session = None;
        let local = principal.user(&store).await.unwrap();
        assert_eq!(local, store.local_user("Dana Okafor").await.unwrap());
        assert_eq!(
            principal.details(),
            serde_json::json!({
                "user": "Dana Okafor", "client": "household-export", "session": "none", "auth": "none",
            })
        );
    }
}
