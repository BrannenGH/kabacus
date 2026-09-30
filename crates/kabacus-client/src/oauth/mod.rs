// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OAuth 2.0 / OIDC token acquisition for the app.
//! Provider-agnostic: every endpoint comes from the issuer's discovery
//! document. (MCP clients sign in on their own, against the metadata
//! kabacus-server's `/mcp` publishes.)
//!
//! A [`TokenSource`] hands [`crate::KabacusClient`] a currently valid access
//! token for each request, fetching or refreshing it as needed, so the app
//! keeps working with short-lived tokens (five minutes or
//! less is a good lifetime).

pub mod app;

use crate::ClientError;
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::Instant;

pub type TokenFuture<'a> = Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>>;

/// Supplies the bearer token for each request.
pub trait TokenSource: Send + Sync {
    fn token(&self) -> TokenFuture<'_>;
}

/// A fixed token (tests, or a caller that manages tokens itself).
pub struct StaticToken(pub String);

impl TokenSource for StaticToken {
    fn token(&self) -> TokenFuture<'_> {
        let token = self.0.clone();
        Box::pin(async move { Ok(token) })
    }
}

/// The parts of `<issuer>/.well-known/openid-configuration` a client needs.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
}

impl Discovery {
    /// OIDC Discovery §4.3: the document must be for the issuer that was
    /// asked for, or its endpoints can't be trusted.
    pub fn for_issuer(self, requested: &str) -> Result<Discovery, ClientError> {
        if self.issuer.trim_end_matches('/') != requested.trim_end_matches('/') {
            return Err(ClientError::Auth(format!(
                "discovery for {requested} names a different issuer, {}",
                self.issuer
            )));
        }
        Ok(self)
    }

    /// A required endpoint, held to the same https rule as the issuer:
    /// the token endpoint receives client secrets.
    pub(crate) fn endpoint(what: &str, value: &Option<String>) -> Result<String, ClientError> {
        let url = value.clone().ok_or_else(|| {
            ClientError::Auth(format!("the provider's discovery document has no {what}"))
        })?;
        check_url(what, &url)?;
        Ok(url)
    }
}

/// The issuer must be HTTPS. Plain HTTP is allowed only to a loopback
/// address, where nothing crosses a network (local test providers).
pub fn check_issuer_url(issuer: &str) -> Result<(), ClientError> {
    check_url("issuer", issuer)
}

fn check_url(what: &str, url: &str) -> Result<(), ClientError> {
    let issuer = url;
    if issuer.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = issuer.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if host == "localhost" || host == "127.0.0.1" || rest.starts_with("[::1]") {
            return Ok(());
        }
    }
    Err(ClientError::Auth(format!(
        "{what} {issuer} must use https:// (plain http is only allowed on loopback)"
    )))
}

pub async fn discover(issuer: &str) -> Result<Discovery, ClientError> {
    check_issuer_url(issuer)?;
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let response = crate::builder()
        .timeout(crate::REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ClientError::Auth(e.to_string()))?
        .get(&url)
        .send()
        .await
        .map_err(|e| ClientError::Auth(format!("discovery at {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(ClientError::Auth(format!(
            "discovery at {url} returned {}",
            response.status()
        )));
    }
    let document: Discovery = response
        .json()
        .await
        .map_err(|e| ClientError::Auth(format!("discovery at {url}: {e}")))?;
    document.for_issuer(issuer)
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthErrorBody {
    pub(crate) error: String,
    pub(crate) error_description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedToken {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    /// When to stop using it: some margin before it actually expires.
    pub(crate) use_until: Instant,
}

impl CachedToken {
    pub(crate) fn from_response(response: TokenResponse) -> Self {
        // Without `expires_in`, assume the recommended short lifetime.
        let lifetime = Duration::from_secs(response.expires_in.unwrap_or(300));
        let margin = (lifetime / 4).min(Duration::from_secs(30));
        CachedToken {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            use_until: Instant::now() + lifetime - margin,
        }
    }

    pub(crate) fn is_fresh(&self) -> bool {
        Instant::now() < self.use_until
    }
}

/// POST a form to the token endpoint, returning the token or the provider's
/// OAuth error code (e.g. `authorization_pending`).
pub(crate) async fn token_request(
    http: &reqwest::Client,
    token_endpoint: &str,
    basic_auth: Option<(&str, &str)>,
    form: &[(&str, &str)],
) -> Result<Result<TokenResponse, OAuthErrorBody>, ClientError> {
    let mut request = http.post(token_endpoint).form(form);
    if let Some((id, secret)) = basic_auth {
        request = request.basic_auth(id, Some(secret));
    }
    let response = request
        .send()
        .await
        .map_err(|e| ClientError::Auth(format!("token endpoint: {e}")))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| ClientError::Auth(format!("token endpoint: {e}")))?;
    if status.is_success() {
        let token = serde_json::from_slice(&body)
            .map_err(|e| ClientError::Auth(format!("token endpoint response: {e}")))?;
        return Ok(Ok(token));
    }
    match serde_json::from_slice::<OAuthErrorBody>(&body) {
        Ok(error) => Ok(Err(error)),
        Err(_) => Err(ClientError::Auth(format!(
            "token endpoint returned {status}: {}",
            String::from_utf8_lossy(&body)
        ))),
    }
}

pub(crate) fn describe(error: &OAuthErrorBody) -> String {
    match &error.error_description {
        Some(description) => format!("{}: {description}", error.error),
        None => error.error.clone(),
    }
}

/// RFC 6749 §6: trade a refresh token for a new access token. The outer
/// error is a network failure (worth retrying); the inner one is the
/// provider refusing the refresh token (sign in again).
pub(crate) async fn refresh_grant(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<Result<CachedToken, String>, ClientError> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    Ok(
        match token_request(http, token_endpoint, None, &form).await? {
            Ok(token) => {
                let mut fresh = CachedToken::from_response(token);
                // The provider may keep the old refresh token.
                if fresh.refresh_token.is_none() {
                    fresh.refresh_token = Some(refresh_token.to_string());
                }
                Ok(fresh)
            }
            Err(error) => Err(describe(&error)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issuer_must_be_https_except_on_loopback() {
        assert!(check_issuer_url("https://id.example.lan/realms/home").is_ok());
        assert!(check_issuer_url("http://127.0.0.1:8080").is_ok());
        assert!(check_issuer_url("http://localhost:8080/x").is_ok());
        assert!(check_issuer_url("http://id.example.lan").is_err());
        assert!(check_issuer_url("http://localhost.evil.example").is_err());
    }

    fn discovery(issuer: &str, token_endpoint: &str) -> Discovery {
        Discovery {
            issuer: issuer.into(),
            authorization_endpoint: None,
            token_endpoint: Some(token_endpoint.into()),
            revocation_endpoint: None,
        }
    }
    #[test]
    fn discovery_must_name_the_requested_issuer() {
        let doc = discovery("https://id.example.lan/", "https://id.example.lan/token");
        assert!(doc.clone().for_issuer("https://id.example.lan").is_ok());
        assert!(doc.for_issuer("https://other.example.lan").is_err());
    }
    #[tokio::test]
    async fn cached_tokens_are_replaced_before_they_expire() {
        let short = CachedToken::from_response(TokenResponse {
            access_token: "t".into(),
            expires_in: Some(0),
            refresh_token: None,
        });
        assert!(!short.is_fresh());
        let normal = CachedToken::from_response(TokenResponse {
            access_token: "t".into(),
            expires_in: Some(300),
            refresh_token: None,
        });
        assert!(normal.is_fresh());
        // Used until 30 s before expiry, not up to the last moment.
        assert!(normal.use_until <= Instant::now() + Duration::from_secs(270));
    }
}
