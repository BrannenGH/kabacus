// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Signing the app in, independent of Qt. The app follows the
//! server: `GET /version` says whether there's OAuth at all,
//! and names the issuer and the app's client id, so a device needs only
//! the server's address.
//!
//! The refresh token lives only in the [`SecretStore`]; KConfig never sees
//! it.

use crate::model::ModelError;
use crate::secrets::{SecretAccount, SecretStore};
use crate::{i18n, i18nc};
use kabacus_client::oauth::app::{BrowserSignIn, UserSession};
use kabacus_client::oauth::{self, Discovery, TokenSource};
use kabacus_client::{ClientError, KabacusClient};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// The server runs `--auth=none`: no sign-in, and a persistent
    /// "Authentication off (local mode)" indicator.
    Off,
    SignedOut,
    SignedIn,
    /// Refreshing was refused: the "Sign in again" banner.
    Expired,
}

/// What the UI shows about the sign-in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuthView {
    pub status: AuthStatus,
    /// Empty unless signed in (and, offline, until the first token).
    pub username: String,
    /// The provider, for "Sign in with …"; empty when auth is off.
    pub issuer: String,
    /// Something about the secret store the user should know; empty if
    /// nothing.
    pub warning: String,
}

#[derive(Clone)]
struct Provider {
    discovery: Discovery,
    client_id: String,
    account: SecretAccount,
}

/// The sign-in state for one server.
pub struct Auth {
    store: Arc<dyn SecretStore>,
    /// `None` when the server runs without auth.
    provider: Option<Provider>,
    session: Option<UserSession>,
    status: AuthStatus,
    username: String,
    warning: Option<String>,
}

/// Keep the secret store current as the provider rotates refresh tokens.
fn keep_stored(session: &UserSession, store: &Arc<dyn SecretStore>, account: &SecretAccount) {
    let (store, account) = (store.clone(), account.clone());
    session.on_new_refresh_token(move |token| {
        let (store, account, token) = (store.clone(), account.clone(), token.to_string());
        tokio::spawn(async move {
            if let Err(e) = store.save(&account, &token).await {
                log::warn!("couldn't store the new refresh token: {e}");
            }
        });
    });
}

impl Auth {
    /// Ask the server at `base` how it authenticates and pick up a session
    /// the secret store kept. `server` identifies the server in the store.
    pub async fn connect(
        base: &KabacusClient,
        server: &str,
        store: Arc<dyn SecretStore>,
    ) -> Result<Auth, ModelError> {
        let version = base.version().await?;
        let mut auth = Auth {
            store,
            provider: None,
            session: None,
            status: AuthStatus::Off,
            username: String::new(),
            warning: None,
        };
        if version.auth.mode == "none" {
            return Ok(auth);
        }
        let issuer = version.auth.issuer.ok_or_else(|| {
            ModelError::Invalid(i18n!("The server uses OAuth but didn't say which provider"))
        })?;
        let client_id = version.auth.app_client_id.ok_or_else(|| {
            ModelError::Invalid(i18n!(
                "The server uses OAuth but has no client id for the app"
            ))
        })?;
        let discovery = oauth::discover(&issuer).await?;
        let provider = Provider {
            discovery,
            client_id,
            account: SecretAccount {
                server: server.to_string(),
                issuer,
            },
        };
        auth.status = AuthStatus::SignedOut;
        // No secret store (none running, or locked and dismissed): treat
        // it as nothing stored, and say why the app will ask to sign in.
        let stored = match auth.store.load(&provider.account).await {
            Ok(stored) => stored,
            Err(e) => {
                auth.warning = Some(i18nc!(
                    "%1 is the error",
                    "Can't read the secret store: %1",
                    e
                ));
                None
            }
        };
        if let Some(refresh_token) = stored {
            let session =
                UserSession::resume(&provider.discovery, &provider.client_id, &refresh_token)?;
            keep_stored(&session, &auth.store, &provider.account);
            match session.token().await {
                Ok(_) => {
                    auth.status = AuthStatus::SignedIn;
                    auth.username = session.username().await.unwrap_or_default();
                    auth.session = Some(session);
                }
                Err(ClientError::SessionExpired(_)) => {
                    // A refresh token the provider refuses is no use to anyone.
                    let _ = auth.store.clear(&provider.account).await;
                    auth.status = AuthStatus::Expired;
                }
                // Can't reach the provider: still signed in, just offline.
                Err(_) => {
                    auth.status = AuthStatus::SignedIn;
                    auth.session = Some(session);
                }
            }
        }
        auth.provider = Some(provider);
        Ok(auth)
    }

    pub fn view(&self) -> AuthView {
        AuthView {
            status: self.status,
            username: self.username.clone(),
            issuer: self
                .provider
                .as_ref()
                .map(|p| p.account.issuer.clone())
                .unwrap_or_default(),
            warning: self.warning.clone().unwrap_or_default(),
        }
    }

    /// `base`, sending this session's tokens if there is one.
    pub fn client(&self, base: KabacusClient) -> KabacusClient {
        match &self.session {
            Some(session) => base.with_token_source(Arc::new(session.clone())),
            None => base,
        }
    }

    /// Start a browser sign-in. Open [`PendingSignIn::authorize_url`] in
    /// the system browser, await [`PendingSignIn::finish`], then pass the
    /// result to [`Auth::signed_in`].
    pub async fn begin_sign_in(&self) -> Result<PendingSignIn, ModelError> {
        let provider = self.provider.clone().ok_or_else(|| {
            ModelError::Invalid(i18n!(
                "This server doesn't use sign-in (authentication is off)"
            ))
        })?;
        let sign_in = BrowserSignIn::start(&provider.discovery, &provider.client_id, None).await?;
        Ok(PendingSignIn {
            authorize_url: sign_in.authorize_url().to_string(),
            sign_in,
            store: self.store.clone(),
            account: provider.account,
        })
    }

    pub fn signed_in(&mut self, done: SignedIn) {
        self.session = Some(done.session);
        self.username = done.username;
        self.warning = done.warning;
        self.status = AuthStatus::SignedIn;
    }

    /// Sign out: revoke at the provider if it can, and always
    /// clear the secret store. An error means revoking failed; the app is
    /// signed out either way.
    pub async fn sign_out(&mut self) -> Result<(), ModelError> {
        let revoked = match self.session.take() {
            Some(session) => session.sign_out().await,
            None => Ok(()),
        };
        let mut cleared = Ok(());
        if let Some(provider) = &self.provider {
            cleared = self.store.clear(&provider.account).await;
            self.status = AuthStatus::SignedOut;
        }
        self.username.clear();
        self.warning = None;
        if let Err(e) = cleared {
            return Err(ModelError::Invalid(i18nc!(
                "%1 is the error",
                "Signed out, but couldn't clear the secret store: %1",
                e
            )));
        }
        revoked.map_err(|e| {
            ModelError::Invalid(i18nc!(
                "%1 is the error",
                "Signed out, but the provider couldn't revoke the session: %1",
                e
            ))
        })
    }

    /// Take note of a failed request. A signed-in session the provider or
    /// server no longer accepts becomes [`AuthStatus::Expired`]; returns
    /// whether the status changed.
    pub fn note_error(&mut self, error: &ModelError) -> bool {
        let rejected = matches!(
            error,
            ModelError::Client(ClientError::SessionExpired(_) | ClientError::Unauthorized)
        );
        if rejected && self.status == AuthStatus::SignedIn {
            self.status = AuthStatus::Expired;
            self.username.clear();
            return true;
        }
        false
    }
}

/// A browser sign-in under way.
pub struct PendingSignIn {
    pub authorize_url: String,
    sign_in: BrowserSignIn,
    store: Arc<dyn SecretStore>,
    account: SecretAccount,
}

/// A finished sign-in, ready for [`Auth::signed_in`].
pub struct SignedIn {
    session: UserSession,
    username: String,
    /// Set when the secret store couldn't keep the sign-in: it works now,
    /// but the next start asks again. It never falls back to a file.
    pub warning: Option<String>,
}

impl PendingSignIn {
    /// Wait for the user to finish in the browser, then keep the refresh
    /// token in the secret store.
    pub async fn finish(self) -> Result<SignedIn, ModelError> {
        let session = self.sign_in.finish().await?;
        let mut warning = None;
        if let Some(refresh_token) = session.refresh_token().await {
            match self.store.save(&self.account, &refresh_token).await {
                Ok(()) => keep_stored(&session, &self.store, &self.account),
                Err(e) => {
                    warning = Some(i18nc!(
                        "%1 is the error",
                        "Signed in, but the secret store couldn't keep it (%1); you'll be asked again next time",
                        e
                    ))
                }
            }
        }
        let username = session.username().await.unwrap_or_default();
        Ok(SignedIn {
            session,
            username,
            warning,
        })
    }
}
