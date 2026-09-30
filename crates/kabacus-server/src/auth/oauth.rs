// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OIDC discovery, JWKS-backed JWT verification, and
//! RFC 7662 token introspection for opaque tokens. Nothing here is
//! provider-specific — everything comes from the issuer's discovery
//! document or explicit config overrides.

use jsonwebtoken::{DecodingKey, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing or malformed bearer token")]
    MissingToken,
    #[error("token invalid: {0}")]
    InvalidToken(String),
    #[error("token is not active")]
    Inactive,
    #[error("unknown signing key id: {0}")]
    UnknownKid(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// Everything but the issuer URL is discovered from
/// `<issuer>/.well-known/openid-configuration`, unless overridden.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub introspection_endpoint: Option<String>,
}

pub async fn discover(http: &reqwest::Client, issuer: &str) -> Result<Discovery> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let response = http.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(AuthError::Provider(format!(
            "discovery at {url} returned {}",
            response.status()
        )));
    }
    Ok(response.json().await?)
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
    #[serde(default)]
    crv: Option<String>,
}

impl Jwk {
    fn decoding_key(&self) -> Result<DecodingKey> {
        match self.kty.as_str() {
            "RSA" => {
                let (n, e) = (self.n.as_deref(), self.e.as_deref());
                let (n, e) = (
                    n.ok_or_else(|| AuthError::Provider("RSA JWK missing n".into()))?,
                    e.ok_or_else(|| AuthError::Provider("RSA JWK missing e".into()))?,
                );
                DecodingKey::from_rsa_components(n, e)
                    .map_err(|e| AuthError::InvalidToken(e.to_string()))
            }
            "EC" => {
                if self.crv.as_deref() != Some("P-256") {
                    return Err(AuthError::Provider(format!(
                        "unsupported EC curve {:?} (only P-256 is)",
                        self.crv
                    )));
                }
                let (x, y) = (self.x.as_deref(), self.y.as_deref());
                let (x, y) = (
                    x.ok_or_else(|| AuthError::Provider("EC JWK missing x".into()))?,
                    y.ok_or_else(|| AuthError::Provider("EC JWK missing y".into()))?,
                );
                DecodingKey::from_ec_components(x, y)
                    .map_err(|e| AuthError::InvalidToken(e.to_string()))
            }
            other => Err(AuthError::Provider(format!("unsupported key type {other}"))),
        }
    }
}

/// A fetched, ready-to-use set of verification keys by `kid`. Building this
/// is the only network-dependent part; [`verify_jwt`] itself is pure and
/// synchronous, so it's fully unit-testable without a live provider.
#[derive(Default)]
pub struct KeySet(HashMap<String, DecodingKey>);

impl KeySet {
    pub fn from_jwks_json(json: &str) -> Result<KeySet> {
        let set: JwkSet =
            serde_json::from_str(json).map_err(|e| AuthError::Provider(e.to_string()))?;
        let mut keys = HashMap::new();
        for jwk in &set.keys {
            keys.insert(jwk.kid.clone(), jwk.decoding_key()?);
        }
        Ok(KeySet(keys))
    }
}

/// Verify a JWT's signature against `keys`, and check `iss`,
/// `aud`, `exp` and `nbf` with `clock_skew` leeway. Returns the raw claims
/// object on success.
pub fn verify_jwt(
    token: &str,
    keys: &KeySet,
    issuer: &str,
    audience: &str,
    clock_skew: Duration,
) -> Result<serde_json::Value> {
    let header =
        jsonwebtoken::decode_header(token).map_err(|e| AuthError::InvalidToken(e.to_string()))?;
    let kid = header
        .kid
        .clone()
        .ok_or_else(|| AuthError::InvalidToken("token has no kid".into()))?;
    let key = keys.0.get(&kid).ok_or(AuthError::UnknownKid(kid))?;

    let mut validation = Validation::new(header.alg);
    validation.set_audience(&[audience]);
    validation.set_issuer(&[issuer]);
    validation.leeway = clock_skew.as_secs();

    let data = jsonwebtoken::decode::<serde_json::Value>(token, key, &validation)
        .map_err(|e| AuthError::InvalidToken(e.to_string()))?;
    Ok(data.claims)
}

/// A JWKS cache keyed by issuer, refreshing on an unknown `kid`, so
/// the provider can rotate its keys without a server restart.
pub struct JwksCache {
    http: reqwest::Client,
    jwks_uri: String,
    keys: RwLock<KeySet>,
}

impl JwksCache {
    pub fn new(http: reqwest::Client, jwks_uri: String) -> Self {
        JwksCache {
            http,
            jwks_uri,
            keys: RwLock::new(KeySet::default()),
        }
    }

    async fn refresh(&self) -> Result<()> {
        let response = self.http.get(&self.jwks_uri).send().await?;
        if !response.status().is_success() {
            return Err(AuthError::Provider(format!(
                "JWKS fetch from {} returned {}",
                self.jwks_uri,
                response.status()
            )));
        }
        let body = response.text().await?;
        let fresh = KeySet::from_jwks_json(&body)?;
        *self.keys.write().await = fresh;
        Ok(())
    }

    /// Verify against the cached key set, refreshing once (and retrying)
    /// if the token's `kid` isn't known yet — this is what makes JWKS
    /// rotation transparent.
    pub async fn verify(
        &self,
        token: &str,
        issuer: &str,
        audience: &str,
        clock_skew: Duration,
    ) -> Result<serde_json::Value> {
        {
            let keys = self.keys.read().await;
            if !keys.0.is_empty() {
                match verify_jwt(token, &keys, issuer, audience, clock_skew) {
                    Ok(claims) => return Ok(claims),
                    Err(AuthError::UnknownKid(_)) => {} // fall through to refresh
                    Err(other) => return Err(other),
                }
            }
        }
        self.refresh().await?;
        let keys = self.keys.read().await;
        verify_jwt(token, &keys, issuer, audience, clock_skew)
    }
}

/// RFC 7662 introspection for opaque tokens, cached by the caller
/// until the token's own expiry.
pub async fn introspect(
    http: &reqwest::Client,
    endpoint: &str,
    token: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<serde_json::Value> {
    let response = http
        .post(endpoint)
        .basic_auth(client_id, Some(client_secret))
        .form(&[("token", token)])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AuthError::Provider(format!(
            "introspection returned {}",
            response.status()
        )));
    }
    let body: serde_json::Value = response.json().await?;
    if body.get("active").and_then(|v| v.as_bool()) != Some(true) {
        return Err(AuthError::Inactive);
    }
    Ok(body)
}

/// Parse the `Authorization: Bearer <token>` header value.
pub fn bearer_token(header_value: Option<&str>) -> Result<&str> {
    header_value
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(AuthError::MissingToken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::sync::Arc;
    #[test]
    fn missing_bearer_prefix_is_rejected() {
        assert!(matches!(
            bearer_token(Some("Basic abc")),
            Err(AuthError::MissingToken)
        ));
        assert!(matches!(bearer_token(None), Err(AuthError::MissingToken)));
        assert_eq!(
            bearer_token(Some("Bearer abc.def.ghi")).unwrap(),
            "abc.def.ghi"
        );
    }

    fn make_rsa_jwt_and_jwks(kid: &str, claims: serde_json::Value) -> (String, String) {
        // A fixed 2048-bit test keypair (never used for anything but this
        // test) so we don't need `openssl`/`rsa` crates at test time to
        // generate one on the fly.
        let pem = include_str!("../../tests/fixtures/jwt_test_key.pem");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let jwks = include_str!("../../tests/fixtures/jwt_test_jwks.json").replace("__KID__", kid);
        (token, jwks)
    }
    #[test]
    fn verifies_a_valid_token_and_returns_claims() {
        let claims = json!({
            "iss": "https://idp.example",
            "aud": "kabacus-server",
            "sub": "user-1",
            "preferred_username": "alice",
            "exp": (chrono::Utc::now().timestamp() + 3600),
            "iat": chrono::Utc::now().timestamp(),
        });
        let (token, jwks_json) = make_rsa_jwt_and_jwks("test-key-1", claims);
        let keys = KeySet::from_jwks_json(&jwks_json).unwrap();
        let claims = verify_jwt(
            &token,
            &keys,
            "https://idp.example",
            "kabacus-server",
            Duration::from_secs(0),
        )
        .unwrap();
        assert_eq!(claims["preferred_username"], "alice");
    }
    #[test]
    fn rejects_expired_token() {
        let claims = json!({
            "iss": "https://idp.example", "aud": "kabacus-server", "sub": "user-1",
            "exp": (chrono::Utc::now().timestamp() - 3600),
        });
        let (token, jwks_json) = make_rsa_jwt_and_jwks("test-key-1", claims);
        let keys = KeySet::from_jwks_json(&jwks_json).unwrap();
        let result = verify_jwt(
            &token,
            &keys,
            "https://idp.example",
            "kabacus-server",
            Duration::from_secs(0),
        );
        assert!(result.is_err());
    }
    #[test]
    fn rejects_wrong_audience() {
        let claims = json!({
            "iss": "https://idp.example", "aud": "someone-else", "sub": "user-1",
            "exp": (chrono::Utc::now().timestamp() + 3600),
        });
        let (token, jwks_json) = make_rsa_jwt_and_jwks("test-key-1", claims);
        let keys = KeySet::from_jwks_json(&jwks_json).unwrap();
        let result = verify_jwt(
            &token,
            &keys,
            "https://idp.example",
            "kabacus-server",
            Duration::from_secs(0),
        );
        assert!(result.is_err());
    }
    #[test]
    fn rejects_wrong_issuer() {
        let claims = json!({
            "iss": "https://not-my-idp.example", "aud": "kabacus-server", "sub": "user-1",
            "exp": (chrono::Utc::now().timestamp() + 3600),
        });
        let (token, jwks_json) = make_rsa_jwt_and_jwks("test-key-1", claims);
        let keys = KeySet::from_jwks_json(&jwks_json).unwrap();
        let result = verify_jwt(
            &token,
            &keys,
            "https://idp.example",
            "kabacus-server",
            Duration::from_secs(0),
        );
        assert!(result.is_err());
    }
    #[test]
    fn rejects_bad_signature_from_an_unknown_key() {
        let claims = json!({
            "iss": "https://idp.example", "aud": "kabacus-server", "sub": "user-1",
            "exp": (chrono::Utc::now().timestamp() + 3600),
        });
        let (token, _) = make_rsa_jwt_and_jwks("test-key-1", claims.clone());
        // Verify against a JWKS that doesn't contain this kid at all.
        let (_, other_jwks) = make_rsa_jwt_and_jwks("a-different-kid", claims);
        let keys = KeySet::from_jwks_json(&other_jwks).unwrap();
        let result = verify_jwt(
            &token,
            &keys,
            "https://idp.example",
            "kabacus-server",
            Duration::from_secs(0),
        );
        assert!(matches!(result, Err(AuthError::UnknownKid(_))));
    }

    // --- Network-backed discovery / JWKS-cache / introspection, exercised
    // against a small local mock IdP (no external provider or Docker
    // needed). ---

    async fn spawn_mock_idp(
        jwks_json: serde_json::Value,
        introspect_active: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        #[derive(Clone)]
        struct MockState {
            jwks: serde_json::Value,
            active: bool,
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let state = Arc::new(MockState {
            jwks: jwks_json,
            active: introspect_active,
        });

        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get({
                    let base = base.clone();
                    move || {
                        let base = base.clone();
                        async move {
                            Json(json!({
                                "issuer": base,
                                "jwks_uri": format!("{base}/jwks"),
                                "introspection_endpoint": format!("{base}/introspect"),
                            }))
                        }
                    }
                }),
            )
            .route("/jwks", get(|State(s): State<Arc<MockState>>| async move { Json(s.jwks.clone()) }))
            .route(
                "/introspect",
                post(|State(s): State<Arc<MockState>>| async move { Json(json!({ "active": s.active, "sub": "user-1", "preferred_username": "alice" })) }),
            )
            .with_state(state);

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (base, handle)
    }
    #[tokio::test]
    async fn discovers_endpoints_from_well_known_configuration() {
        let (base, _server) = spawn_mock_idp(json!({"keys": []}), true).await;
        let http = reqwest::Client::new();
        let discovery = discover(&http, &base).await.unwrap();
        assert_eq!(discovery.jwks_uri, format!("{base}/jwks"));
        assert_eq!(
            discovery.introspection_endpoint.as_deref(),
            Some(format!("{base}/introspect").as_str())
        );
    }
    #[tokio::test]
    async fn jwks_cache_refreshes_on_unknown_kid_without_a_restart() {
        let claims = json!({
            "iss": "placeholder", "aud": "kabacus-server", "sub": "user-1",
            "exp": (chrono::Utc::now().timestamp() + 3600),
        });
        let (token, jwks_json_str) = make_rsa_jwt_and_jwks("rotated-key", claims);
        let jwks_json: serde_json::Value = serde_json::from_str(&jwks_json_str).unwrap();
        let (base, _server) = spawn_mock_idp(jwks_json, true).await;

        let cache = JwksCache::new(reqwest::Client::new(), format!("{base}/jwks"));
        // Cache starts empty; the first verify() call must trigger a fetch.
        let claims = cache
            .verify(
                &token,
                "placeholder",
                "kabacus-server",
                Duration::from_secs(0),
            )
            .await
            .unwrap();
        assert_eq!(claims["sub"], "user-1");
    }
    #[tokio::test]
    async fn introspection_accepts_an_active_token_and_rejects_inactive() {
        let (base, _server) = spawn_mock_idp(json!({"keys": []}), true).await;
        let http = reqwest::Client::new();
        let claims = introspect(
            &http,
            &format!("{base}/introspect"),
            "opaque-token",
            "kabacus-server",
            "secret",
        )
        .await
        .unwrap();
        assert_eq!(claims["preferred_username"], "alice");

        let (base2, _server2) = spawn_mock_idp(json!({"keys": []}), false).await;
        let result = introspect(
            &http,
            &format!("{base2}/introspect"),
            "opaque-token",
            "kabacus-server",
            "secret",
        )
        .await;
        assert!(matches!(result, Err(AuthError::Inactive)));
    }
}
