// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Configurable OAuth 2.0 / OIDC authentication and role
//! authorization, or the explicit `--auth=none` mode.

pub mod claims;
pub mod none;
pub mod oauth;
pub mod roles;

use crate::config::{AuthConfig, AuthMode, TokenType};
use oauth::{AuthError, JwksCache};
use roles::{resolve_roles, Channel, Principal};
use std::time::Duration;

pub struct AuthContext {
    config: AuthConfig,
    http: reqwest::Client,
    jwks: Option<JwksCache>,
    introspection_endpoint: Option<String>,
}

impl AuthContext {
    /// Resolve everything from the issuer's discovery document
    /// (unless the mode is `none`, in which case nothing is fetched).
    pub async fn init(config: AuthConfig) -> anyhow::Result<AuthContext> {
        let http = reqwest::Client::new();
        if config.mode == AuthMode::None {
            return Ok(AuthContext {
                config,
                http,
                jwks: None,
                introspection_endpoint: None,
            });
        }
        let issuer = config
            .issuer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("auth.mode = \"oauth\" requires auth.issuer"))?;
        let discovery = oauth::discover(&http, &issuer).await?;
        let jwks = JwksCache::new(http.clone(), discovery.jwks_uri.clone());
        Ok(AuthContext {
            config,
            http,
            jwks: Some(jwks),
            introspection_endpoint: discovery.introspection_endpoint,
        })
    }

    pub fn mode(&self) -> &AuthMode {
        &self.config.mode
    }

    pub fn issuer(&self) -> Option<&str> {
        self.config.issuer.as_deref()
    }

    /// Turn a bearer token (or, in no-auth mode, a
    /// client-supplied hint) into a [`Principal`]. `client_header` is the
    /// `Kabacus-Client-Hint` header the app sends identifying itself (and
    /// `mcp` for `/mcp`), used for the no-auth-mode audit trail;
    /// in OAuth mode the client comes from the token alone.
    pub async fn authenticate(
        &self,
        authorization_header: Option<&str>,
        client_header: Option<&str>,
    ) -> Result<Principal, AuthError> {
        if self.config.mode == AuthMode::None {
            return Ok(none::no_auth_principal(
                &self.config.no_auth_roles,
                client_header,
            ));
        }

        let token = oauth::bearer_token(authorization_header)?;
        let claims = match self.config.token_type {
            TokenType::Jwt => {
                let jwks = self
                    .jwks
                    .as_ref()
                    .expect("oauth mode always has a jwks cache");
                let issuer = self.config.issuer.as_deref().unwrap_or_default();
                let audience = self.config.audience.as_deref().unwrap_or_default();
                let skew = Duration::from_secs(self.config.clock_skew_seconds);
                jwks.verify(token, issuer, audience, skew).await?
            }
            TokenType::Introspection => {
                let endpoint = self.introspection_endpoint.as_deref().ok_or_else(|| {
                    AuthError::Provider("no introspection_endpoint from discovery".into())
                })?;
                let client_id = self
                    .config
                    .introspection_client_id
                    .as_deref()
                    .unwrap_or_default();
                let client_secret = self
                    .config
                    .introspection_client_secret
                    .as_deref()
                    .unwrap_or_default();
                oauth::introspect(&self.http, endpoint, token, client_id, client_secret).await?
            }
        };

        let subject = claims::get_string(&claims, "sub")
            .unwrap_or_default()
            .to_string();
        let username = claims::get_string(&claims, &self.config.claims.username)
            .or_else(|| claims::get_string(&claims, "sub"))
            .unwrap_or("unknown")
            .to_string();
        let client = claims::get_string(&claims, &self.config.claims.client)
            .unwrap_or("unknown")
            .to_string();
        let session = self
            .config
            .claims
            .session
            .as_ref()
            .and_then(|path| claims::get_string(&claims, path))
            .map(str::to_string);
        let raw_roles = claims::get_string_list(&claims, &self.config.claims.roles);
        let roles = resolve_roles(&raw_roles, &self.config.role_map);

        Ok(Principal {
            subject: if subject.is_empty() {
                username.clone()
            } else {
                subject
            },
            username,
            client,
            session,
            issuer: self.config.issuer.clone().unwrap_or_default(),
            roles,
            auth_none: false,
            channel: Channel::Api,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    #[tokio::test]
    async fn none_mode_never_hits_the_network() {
        let ctx = AuthContext::init(AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        })
        .await
        .unwrap();
        let principal = ctx.authenticate(None, Some("app")).await.unwrap();
        assert!(principal.has_role(super::roles::Role::Read));
    }
}
