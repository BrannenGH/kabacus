// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A stand-in OAuth 2.0 / OIDC provider, served in-process over loopback
//! HTTP. It implements only what Kabacus uses — discovery, JWKS, the
//! authorization code grant with PKCE (the browser side is automatic: it
//! signs `sign_in_as` straight in), client credentials,
//! refresh tokens and revocation (RFC 7009) — and signs RS256 tokens with the fixed test key in kabacus-server's
//! fixtures. Its claim shapes are ordinary (`preferred_username`, `azp`, a
//! top-level `roles` list); auth is also tested against
//! Keycloak where a container runtime is available.

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const KEY_PEM: &str = include_str!("../../kabacus-server/tests/fixtures/jwt_test_key.pem");
const JWKS: &str = include_str!("../../kabacus-server/tests/fixtures/jwt_test_jwks.json");
const KID: &str = "testkit-key";

#[derive(Clone)]
pub struct ProviderOptions {
    pub audience: String,
    /// Confidential clients allowed the client credentials grant:
    /// `(client_id, secret)`.
    pub confidential_clients: Vec<(String, String)>,
    /// The `roles` claim put in every token.
    pub roles: Vec<String>,
    pub token_lifetime_secs: i64,
    /// Whether discovery lists `client_credentials` in
    /// `grant_types_supported`.
    pub advertise_client_credentials: bool,
    /// Who the browser sign-in (`/authorize`) signs in, straight away;
    /// `None` means the user refuses (`access_denied`).
    pub sign_in_as: Option<String>,
}

impl Default for ProviderOptions {
    fn default() -> Self {
        ProviderOptions {
            audience: "kabacus-server".to_string(),
            confidential_clients: vec![("kabacus-mcp".to_string(), "test-secret".to_string())],
            roles: vec!["ledger-read".to_string(), "ledger-write".to_string()],
            token_lifetime_secs: 300,
            advertise_client_credentials: true,
            sign_in_as: Some("alex".to_string()),
        }
    }
}

/// An authorization code waiting to be redeemed.
struct CodeEntry {
    username: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
}

struct ProviderState {
    issuer: String,
    options: ProviderOptions,
    codes: HashMap<String, CodeEntry>,
    revoked: Vec<String>,
    /// refresh token -> (username, client id)
    refresh_tokens: HashMap<String, (String, String)>,
    tokens_issued: usize,
}

#[derive(Clone)]
pub struct FakeProvider {
    pub issuer: String,
    state: Arc<Mutex<ProviderState>>,
}

impl FakeProvider {
    pub async fn start(options: ProviderOptions) -> FakeProvider {
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", tcp.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ProviderState {
            issuer: issuer.clone(),
            options,
            codes: HashMap::new(),
            revoked: Vec::new(),
            refresh_tokens: HashMap::new(),
            tokens_issued: 0,
        }));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/revoke", post(revoke))
            .route("/token", post(token))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(tcp, app).await.unwrap() });
        FakeProvider { issuer, state }
    }

    /// The session ends at the provider (an admin logs it out): every
    /// refresh token it issued stops working.
    pub fn end_all_sessions(&self) {
        self.state.lock().unwrap().refresh_tokens.clear();
    }

    /// The tokens revoked through `/revoke`, in order.
    pub fn revoked(&self) -> Vec<String> {
        self.state.lock().unwrap().revoked.clone()
    }

    /// Whether `refresh_token` would still be accepted.
    pub fn refresh_token_is_live(&self, refresh_token: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .refresh_tokens
            .contains_key(refresh_token)
    }

    /// How many access tokens the token endpoint has issued.
    pub fn tokens_issued(&self) -> usize {
        self.state.lock().unwrap().tokens_issued
    }

    /// Mint an access token directly, as if `client_id` had obtained it
    /// for `username`.
    pub fn mint(&self, username: &str, client_id: &str) -> String {
        mint(&self.state.lock().unwrap(), username, client_id)
    }
}

fn mint(state: &ProviderState, username: &str, client_id: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": state.issuer,
        "aud": state.options.audience,
        "sub": format!("sub-{username}"),
        "preferred_username": username,
        "azp": client_id,
        "roles": state.options.roles,
        "iat": now,
        "exp": now + state.options.token_lifetime_secs,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

type Shared = State<Arc<Mutex<ProviderState>>>;

async fn discovery(State(state): Shared) -> Json<Value> {
    let state = state.lock().unwrap();
    let issuer = &state.issuer;
    let mut grants = vec!["authorization_code", "refresh_token"];
    if state.options.advertise_client_credentials {
        grants.push("client_credentials");
    }
    Json(json!({
        "issuer": issuer,
        "jwks_uri": format!("{issuer}/jwks"),
        "authorization_endpoint": format!("{issuer}/authorize"),
        "revocation_endpoint": format!("{issuer}/revoke"),
        "token_endpoint": format!("{issuer}/token"),
        "grant_types_supported": grants,
    }))
}

async fn jwks() -> Json<Value> {
    Json(serde_json::from_str(&JWKS.replace("__KID__", KID)).unwrap())
}

async fn authorize(State(state): Shared, Query(query): Query<HashMap<String, String>>) -> Response {
    let mut state = state.lock().unwrap();
    let get = |key: &str| query.get(key).cloned().unwrap_or_default();
    let redirect_uri = get("redirect_uri");
    // RFC 8252 §7.3: loopback redirects only, as the app uses.
    if !redirect_uri.starts_with("http://127.0.0.1:") {
        return (StatusCode::BAD_REQUEST, "redirect_uri must be loopback").into_response();
    }
    let back = |params: &[(&str, &str)]| {
        let mut url = redirect_uri.clone();
        for (i, (key, value)) in params.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str(&format!("{key}={}", urlencode(value)));
        }
        (StatusCode::FOUND, [("location", url)]).into_response()
    };
    let client_state = get("state");
    if get("response_type") != "code" || get("code_challenge_method") != "S256" {
        return back(&[("error", "invalid_request"), ("state", &client_state)]);
    }
    let Some(username) = state.options.sign_in_as.clone() else {
        return back(&[("error", "access_denied"), ("state", &client_state)]);
    };
    let code = uuid::Uuid::new_v4().to_string();
    state.codes.insert(
        code.clone(),
        CodeEntry {
            username,
            client_id: get("client_id"),
            redirect_uri: redirect_uri.clone(),
            code_challenge: get("code_challenge"),
        },
    );
    back(&[("code", &code), ("state", &client_state)])
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// RFC 7009. Always 200, as the RFC asks, even for unknown tokens.
async fn revoke(State(state): Shared, Form(form): Form<HashMap<String, String>>) -> Response {
    let mut state = state.lock().unwrap();
    let token = form.get("token").cloned().unwrap_or_default();
    state.refresh_tokens.remove(&token);
    state.revoked.push(token);
    StatusCode::OK.into_response()
}

fn oauth_error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

fn issue(state: &mut ProviderState, username: &str, client_id: &str, refresh: bool) -> Response {
    state.tokens_issued += 1;
    let access_token = mint(state, username, client_id);
    let mut body = json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": state.options.token_lifetime_secs,
    });
    if refresh {
        let refresh_token = uuid::Uuid::new_v4().to_string();
        state
            .refresh_tokens
            .insert(refresh_token.clone(), (username.into(), client_id.into()));
        body["refresh_token"] = json!(refresh_token);
    }
    Json(body).into_response()
}

/// `client_secret_basic` credentials from the Authorization header.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let (id, secret) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}

async fn token(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut state = state.lock().unwrap();
    match form.get("grant_type").map(String::as_str) {
        Some("client_credentials") => {
            let Some((id, secret)) = basic_credentials(&headers) else {
                return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
            };
            let known = state
                .options
                .confidential_clients
                .iter()
                .any(|(i, s)| *i == id && *s == secret);
            if !known {
                return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
            }
            // A service account's username is its own client id.
            issue(&mut state, &format!("service-account-{id}"), &id, false)
        }
        Some("authorization_code") => {
            let code = form.get("code").cloned().unwrap_or_default();
            let Some(entry) = state.codes.remove(&code) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            };
            let verifier = form.get("code_verifier").cloned().unwrap_or_default();
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()),
            );
            let matches = form.get("client_id") == Some(&entry.client_id)
                && form.get("redirect_uri") == Some(&entry.redirect_uri)
                && challenge == entry.code_challenge;
            if !matches {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            issue(&mut state, &entry.username, &entry.client_id, true)
        }
        Some("refresh_token") => {
            let presented = form.get("refresh_token").cloned().unwrap_or_default();
            let Some((username, client_id)) = state.refresh_tokens.remove(&presented) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            };
            issue(&mut state, &username, &client_id, true)
        }
        _ => oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}
