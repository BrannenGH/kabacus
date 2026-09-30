// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's sign-in. Authorization Code + PKCE (RFC 7636) in the
//! system browser, with the redirect caught on a loopback port (RFC 8252
//! §7.3). There's no embedded web view, and the app never sees the
//! password.
//!
//! [`BrowserSignIn`] runs one sign-in and yields a [`UserSession`], which
//! then keeps the access token fresh from the refresh token. The caller
//! keeps that refresh token in the platform secret store, never in a file,
//! and hears about each new one through
//! [`UserSession::on_new_refresh_token`].

use super::{
    check_url, describe, refresh_grant, token_request, CachedToken, Discovery, TokenFuture,
    TokenSource,
};
use crate::ClientError;
use base64::Engine;
use reqwest::Url;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::Instant;

/// How long the user has to finish signing in in the browser.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// The scope asked for when the server doesn't need another.
pub const DEFAULT_SCOPE: &str = "openid";
const CALLBACK_PATH: &str = "/callback";

/// A PKCE code verifier and its S256 challenge (RFC 7636 §4.1–4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Pkce {
        Pkce::from_verifier(random_token())
    }

    pub fn from_verifier(verifier: String) -> Pkce {
        let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
        Pkce {
            challenge: base64url(digest.as_ref()),
            verifier,
        }
    }
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 256 random bits as base64url (43 characters): a PKCE verifier or a
/// `state` value.
fn random_token() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system's random number generator failed");
    base64url(&bytes)
}

/// The provider endpoints a signed-in session uses.
#[derive(Clone)]
struct Endpoints {
    http: reqwest::Client,
    token: String,
    revocation: Option<String>,
    client_id: String,
}

impl Endpoints {
    fn new(discovery: &Discovery, client_id: &str) -> Result<Endpoints, ClientError> {
        let revocation = match &discovery.revocation_endpoint {
            Some(url) => {
                check_url("revocation_endpoint", url)?;
                Some(url.clone())
            }
            None => None,
        };
        Ok(Endpoints {
            http: crate::builder()
                .timeout(crate::REQUEST_TIMEOUT)
                .build()
                .map_err(|e| ClientError::Auth(e.to_string()))?,
            token: Discovery::endpoint("token_endpoint", &discovery.token_endpoint)?,
            revocation,
            client_id: client_id.to_string(),
        })
    }
}

/// One sign-in in progress: open [`Self::authorize_url`] in the system
/// browser, then await [`Self::finish`].
pub struct BrowserSignIn {
    endpoints: Endpoints,
    listener: TcpListener,
    authorize_url: String,
    redirect_uri: String,
    state: String,
    pkce: Pkce,
}

impl BrowserSignIn {
    /// Listen on a loopback port for the redirect and build the URL that
    /// starts the sign-in. `client_id` is the app's public client (the
    /// server's `app_client_id`).
    pub async fn start(
        discovery: &Discovery,
        client_id: &str,
        scope: Option<&str>,
    ) -> Result<BrowserSignIn, ClientError> {
        let endpoints = Endpoints::new(discovery, client_id)?;
        let authorization =
            Discovery::endpoint("authorization_endpoint", &discovery.authorization_endpoint)?;
        let mut url = Url::parse(&authorization)
            .map_err(|e| ClientError::Auth(format!("authorization_endpoint: {e}")))?;
        // RFC 8252 §7.3, §8.3: the loopback IP literal, not `localhost`,
        // on whatever port is free.
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| {
            ClientError::Auth(format!("can't listen for the sign-in redirect: {e}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|e| ClientError::Auth(format!("can't listen for the sign-in redirect: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let state = random_token();
        let pkce = Pkce::generate();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", scope.unwrap_or(DEFAULT_SCOPE))
            .append_pair("state", &state)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(BrowserSignIn {
            endpoints,
            listener,
            authorize_url: url.into(),
            redirect_uri,
            state,
            pkce,
        })
    }

    /// Where to send the user's browser.
    pub fn authorize_url(&self) -> &str {
        &self.authorize_url
    }

    /// Wait for the browser to come back, then trade the code for tokens.
    /// Gives up after [`SIGN_IN_TIMEOUT`].
    pub async fn finish(self) -> Result<UserSession, ClientError> {
        tokio::time::timeout(SIGN_IN_TIMEOUT, self.wait_and_redeem())
            .await
            .map_err(|_| {
                ClientError::Auth("sign-in timed out: the browser didn't come back".into())
            })?
    }

    async fn wait_and_redeem(self) -> Result<UserSession, ClientError> {
        let code = loop {
            let (mut stream, _) =
                self.listener.accept().await.map_err(|e| {
                    ClientError::Auth(format!("waiting for the sign-in redirect: {e}"))
                })?;
            let Some(target) = read_request_target(&mut stream).await else {
                continue;
            };
            match callback_outcome(&target, &self.state) {
                Callback::NotOurs => {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "Not a Kabacus sign-in",
                        "This page isn't part of the sign-in the app is waiting for.",
                    )
                    .await;
                }
                Callback::Refused(reason) => {
                    respond(
                        &mut stream,
                        "200 OK",
                        "Sign-in didn't finish",
                        "Go back to Kabacus to try again.",
                    )
                    .await;
                    return Err(ClientError::Auth(format!("sign-in was refused: {reason}")));
                }
                Callback::Code(code) => {
                    respond(
                        &mut stream,
                        "200 OK",
                        "Signed in to Kabacus",
                        "You can close this tab and go back to the app.",
                    )
                    .await;
                    break code;
                }
            }
        };
        let endpoints = &self.endpoints;
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("client_id", endpoints.client_id.as_str()),
            ("code_verifier", self.pkce.verifier.as_str()),
        ];
        match token_request(&endpoints.http, &endpoints.token, None, &form).await? {
            Ok(token) => Ok(UserSession::with_token(
                self.endpoints,
                CachedToken::from_response(token),
            )),
            Err(error) => Err(ClientError::Auth(format!(
                "the provider refused the sign-in code: {}",
                describe(&error)
            ))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    /// Some other request: a wrong path, or a `state` that isn't this
    /// sign-in's (RFC 6749 §10.12). Answered and ignored.
    NotOurs,
    /// The provider sent the user back with an error.
    Refused(String),
    Code(String),
}

/// Judge a request that reached the loopback port. `target` is the
/// request line's path and query.
fn callback_outcome(target: &str, expected_state: &str) -> Callback {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::NotOurs;
    };
    if url.path() != CALLBACK_PATH {
        return Callback::NotOurs;
    }
    let get = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    };
    if get("state").as_deref() != Some(expected_state) {
        return Callback::NotOurs;
    }
    if let Some(error) = get("error") {
        return Callback::Refused(match get("error_description") {
            Some(description) => format!("{error}: {description}"),
            None => error,
        });
    }
    match get("code") {
        Some(code) if !code.is_empty() => Callback::Code(code),
        _ => Callback::Refused("the redirect had no code".into()),
    }
}

/// The request target of a `GET`, read from a browser connection.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::with_capacity(1024);
    let read = async {
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            head.extend_from_slice(&chunk[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .ok()??;
    let line = std::str::from_utf8(&head).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    parts.next().map(str::to_string)
}

/// A small page for the browser tab. The text is fixed; nothing from the
/// request is echoed into it.
async fn respond(stream: &mut TcpStream, status: &str, title: &str, text: &str) {
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"font-family:sans-serif;margin:3em\"><h1>{title}</h1><p>{text}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

enum SessionState {
    Active(CachedToken),
    /// Refreshing was refused, or the user signed out: sign in again.
    Ended(String),
}

type RefreshTokenHook = Arc<dyn Fn(&str) + Send + Sync>;

/// A signed-in user. Hands [`crate::KabacusClient`] access tokens and
/// refreshes them silently; once the provider refuses the refresh token,
/// every request fails with [`ClientError::SessionExpired`].
#[derive(Clone)]
pub struct UserSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    endpoints: Endpoints,
    state: Mutex<SessionState>,
    on_new_refresh_token: std::sync::Mutex<Option<RefreshTokenHook>>,
}

impl UserSession {
    fn with_token(endpoints: Endpoints, token: CachedToken) -> UserSession {
        UserSession {
            inner: Arc::new(SessionInner {
                endpoints,
                state: Mutex::new(SessionState::Active(token)),
                on_new_refresh_token: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Resume a session from a refresh token kept in the secret store.
    /// The first request refreshes; if the provider refuses, it fails with
    /// [`ClientError::SessionExpired`].
    pub fn resume(
        discovery: &Discovery,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<UserSession, ClientError> {
        let endpoints = Endpoints::new(discovery, client_id)?;
        Ok(UserSession::with_token(
            endpoints,
            CachedToken {
                access_token: String::new(),
                refresh_token: Some(refresh_token.to_string()),
                use_until: Instant::now(),
            },
        ))
    }

    /// Call `hook` with each refresh token the provider issues from now on
    /// (they rotate), so the caller can keep the secret store current.
    pub fn on_new_refresh_token(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
        *self.inner.on_new_refresh_token.lock().unwrap() = Some(Arc::new(hook));
    }

    /// The current refresh token, for the secret store.
    pub async fn refresh_token(&self) -> Option<String> {
        match &*self.inner.state.lock().await {
            SessionState::Active(token) => token.refresh_token.clone(),
            SessionState::Ended(_) => None,
        }
    }

    /// Who is signed in, from the access token's claims: for display only
    /// (the server does the real validation). `None` before the first
    /// token, or if the token isn't a readable JWT.
    pub async fn username(&self) -> Option<String> {
        match &*self.inner.state.lock().await {
            SessionState::Active(token) => username_from_jwt(&token.access_token),
            SessionState::Ended(_) => None,
        }
    }

    /// End the session: revoke the refresh token at the provider if it has
    /// a revocation endpoint (RFC 7009). The session ends even if revoking
    /// fails; the error says why, for the caller to report.
    pub async fn sign_out(&self) -> Result<(), ClientError> {
        let previous = std::mem::replace(
            &mut *self.inner.state.lock().await,
            SessionState::Ended("signed out".into()),
        );
        let endpoints = &self.inner.endpoints;
        let (SessionState::Active(token), Some(revocation)) = (previous, &endpoints.revocation)
        else {
            return Ok(());
        };
        let (value, hint) = match &token.refresh_token {
            Some(refresh) => (refresh.clone(), "refresh_token"),
            None => (token.access_token, "access_token"),
        };
        if value.is_empty() {
            return Ok(());
        }
        let form = [
            ("token", value.as_str()),
            ("token_type_hint", hint),
            ("client_id", endpoints.client_id.as_str()),
        ];
        let response = endpoints
            .http
            .post(revocation)
            .form(&form)
            .send()
            .await
            .map_err(|e| ClientError::Auth(format!("revocation endpoint: {e}")))?;
        if !response.status().is_success() {
            return Err(ClientError::Auth(format!(
                "revocation endpoint returned {}",
                response.status()
            )));
        }
        Ok(())
    }

    fn announce(&self, refresh_token: &str) {
        let hook = self.inner.on_new_refresh_token.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(refresh_token);
        }
    }
}

impl TokenSource for UserSession {
    fn token(&self) -> TokenFuture<'_> {
        Box::pin(async move {
            let mut state = self.inner.state.lock().await;
            let current = match &*state {
                SessionState::Ended(reason) => {
                    return Err(ClientError::SessionExpired(reason.clone()))
                }
                SessionState::Active(token) if token.is_fresh() => {
                    return Ok(token.access_token.clone())
                }
                SessionState::Active(token) => token.clone(),
            };
            let Some(refresh_token) = current.refresh_token else {
                let reason = "the provider gave no refresh token".to_string();
                *state = SessionState::Ended(reason.clone());
                return Err(ClientError::SessionExpired(reason));
            };
            let endpoints = &self.inner.endpoints;
            let refreshed = refresh_grant(
                &endpoints.http,
                &endpoints.token,
                &endpoints.client_id,
                &refresh_token,
            )
            .await
            // Can't reach the provider: offline, not signed out. The
            // session stays as it is, and the next request tries again.
            .map_err(|e| ClientError::Request(e.to_string()))?;
            match refreshed {
                Ok(fresh) => {
                    let access_token = fresh.access_token.clone();
                    let rotated = fresh
                        .refresh_token
                        .clone()
                        .filter(|new| *new != refresh_token);
                    *state = SessionState::Active(fresh);
                    drop(state);
                    if let Some(new) = rotated {
                        self.announce(&new);
                    }
                    Ok(access_token)
                }
                Err(reason) => {
                    *state = SessionState::Ended(reason.clone());
                    Err(ClientError::SessionExpired(reason))
                }
            }
        })
    }
}

/// `preferred_username`, else `name`, else `sub`, from a JWT's payload,
/// without checking the signature.
fn username_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["preferred_username", "name", "sub"]
        .iter()
        .find_map(|key| claims.get(key)?.as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_challenge_matches_rfc_7636_appendix_b() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn generated_verifiers_are_fresh_and_the_right_length() {
        let (a, b) = (Pkce::generate(), Pkce::generate());
        assert_ne!(a.verifier, b.verifier);
        // RFC 7636 §4.1: 43 to 128 unreserved characters.
        assert_eq!(a.verifier.len(), 43);
        assert!(a
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)));
    }
    #[test]
    fn only_a_redirect_with_this_sign_ins_state_is_accepted() {
        assert_eq!(
            callback_outcome("/callback?code=abc&state=s1", "s1"),
            Callback::Code("abc".into())
        );
        // Another sign-in's state, or none: possible CSRF, ignored.
        assert_eq!(
            callback_outcome("/callback?code=abc&state=s2", "s1"),
            Callback::NotOurs
        );
        assert_eq!(
            callback_outcome("/callback?code=abc", "s1"),
            Callback::NotOurs
        );
        assert_eq!(callback_outcome("/favicon.ico", "s1"), Callback::NotOurs);
        assert_eq!(
            callback_outcome(
                "/callback?error=access_denied&error_description=no%20thanks&state=s1",
                "s1"
            ),
            Callback::Refused("access_denied: no thanks".into())
        );
        assert!(matches!(
            callback_outcome("/callback?state=s1", "s1"),
            Callback::Refused(_)
        ));
    }

    fn discovery() -> Discovery {
        Discovery {
            issuer: "https://id.example.lan".into(),
            authorization_endpoint: Some("https://id.example.lan/auth?realm=home".into()),
            token_endpoint: Some("https://id.example.lan/token".into()),
            revocation_endpoint: None,
        }
    }
    #[tokio::test]
    async fn the_authorize_url_asks_for_a_code_with_s256_and_a_loopback_redirect() {
        let sign_in = BrowserSignIn::start(&discovery(), "kabacus-app", None)
            .await
            .unwrap();
        let url = Url::parse(sign_in.authorize_url()).unwrap();
        let get = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        // The endpoint's own query survives.
        assert_eq!(get("realm").as_deref(), Some("home"));
        assert_eq!(get("response_type").as_deref(), Some("code"));
        assert_eq!(get("client_id").as_deref(), Some("kabacus-app"));
        assert_eq!(get("scope").as_deref(), Some(DEFAULT_SCOPE));
        assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(get("code_challenge"), Some(sign_in.pkce.challenge.clone()));
        assert_eq!(get("state"), Some(sign_in.state.clone()));
        let redirect = get("redirect_uri").unwrap();
        assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
        assert!(redirect.ends_with("/callback"), "{redirect}");
    }
    #[tokio::test]
    async fn sign_in_needs_https_provider_endpoints() {
        let mut doc = discovery();
        doc.token_endpoint = Some("http://id.example.lan/token".into());
        assert!(BrowserSignIn::start(&doc, "kabacus-app", None)
            .await
            .is_err());
        let mut doc = discovery();
        doc.authorization_endpoint = None;
        assert!(BrowserSignIn::start(&doc, "kabacus-app", None)
            .await
            .is_err());
    }
    #[test]
    fn the_username_comes_from_the_token_claims() {
        let jwt = |claims: &str| format!("h.{}.s", base64url(claims.as_bytes()));
        assert_eq!(
            username_from_jwt(&jwt(r#"{"preferred_username":"jordan","sub":"1"}"#)).as_deref(),
            Some("jordan")
        );
        assert_eq!(
            username_from_jwt(&jwt(r#"{"sub":"u-1"}"#)).as_deref(),
            Some("u-1")
        );
        assert_eq!(username_from_jwt("opaque-token"), None);
    }
}
