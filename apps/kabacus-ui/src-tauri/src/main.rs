// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kabacus_client::{
    oauth::{
        self,
        app::{BrowserSignIn, UserSession},
        TokenSource,
    },
    KabacusClient,
};
use kabacus_kde::secrets::{SecretAccount, SecretServiceStore, SecretStore};
use std::sync::Arc;
use tauri::Manager;

fn server_url(value: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(value).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("KABACUS_SERVER_URL must be an HTTPS origin".into());
    }
    url.set_path("/app/");
    Ok(url)
}

struct NativeSession {
    session: UserSession,
    account: SecretAccount,
    store: Arc<SecretServiceStore>,
}
impl NativeSession {
    async fn token(&self) -> Result<String, kabacus_client::ClientError> {
        self.session.token().await
    }
    async fn sign_out(&self) {
        let _ = self.session.sign_out().await;
        let _ = self.store.clear(&self.account).await;
    }
}
async fn sign_in(origin: &str) -> Result<NativeSession, Box<dyn std::error::Error + Send + Sync>> {
    let client = KabacusClient::connect_https(origin, None)?;
    let version = client.version().await?;
    let issuer = version
        .auth
        .issuer
        .ok_or("The server uses OAuth but didn't say which provider")?;
    let client_id = version
        .auth
        .app_client_id
        .ok_or("The server uses OAuth but has no client id for the app")?;
    let discovery = oauth::discover(&issuer).await?;
    let account = SecretAccount {
        server: origin.to_string(),
        issuer,
    };
    let store = Arc::new(SecretServiceStore);
    let mut restored = None;
    if let Ok(Some(refresh)) = store.load(&account).await {
        let candidate = UserSession::resume(&discovery, &client_id, &refresh)?;
        match candidate.token().await {
            Ok(_) => restored = Some(candidate),
            Err(kabacus_client::ClientError::SessionExpired(_)) => {
                let _ = store.clear(&account).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let session = match restored {
        Some(session) => session,
        None => {
            let pending = BrowserSignIn::start(&discovery, &client_id, None).await?;
            open::that(pending.authorize_url())?;
            pending.finish().await?
        }
    };
    if let Some(refresh) = session.refresh_token().await {
        if let Err(error) = store.save(&account, &refresh).await {
            eprintln!("{error}");
        }
    }
    let hook_store = store.clone();
    let hook_account = account.clone();
    session.on_new_refresh_token(move |token| {
        let store = hook_store.clone();
        let account = hook_account.clone();
        let token = token.to_string();
        tauri::async_runtime::spawn(async move {
            let _ = store.save(&account, &token).await;
        });
    });
    Ok(NativeSession {
        session,
        account,
        store,
    })
}

fn trusted_local_url(url: &url::Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}

fn token_script(origin: &str, token: Option<&str>, username: &str) -> String {
    let origin = serde_json::to_string(origin).unwrap();
    let token = serde_json::to_string(&token).unwrap();
    let username = serde_json::to_string(username).unwrap();
    // Recheck the origin inside the page as navigation can race an eval.
    format!("if(location.origin === {origin}) {{ const token = {token}; sessionStorage.setItem('kabacus.native-user',{username}); if(token) sessionStorage.setItem('kabacus.native-token',token); else sessionStorage.removeItem('kabacus.native-token'); window.dispatchEvent(new Event('kabacus-native-ready')); }}")
}

#[tauri::command]
async fn connect_server(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    origin: String,
) -> Result<(), String> {
    // Only our bundled connection window may start a native session.
    let caller = window.url().map_err(|e| e.to_string())?;
    if window.label() != "connection" || !trusted_local_url(&caller) {
        return Err("Untrusted connection window".into());
    }
    let url = server_url(&origin)?;
    let origin = url.origin().ascii_serialization();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
    let session = Arc::new(sign_in(&origin).await.map_err(|e| e.to_string())?);
    let app_handle = app.clone();
    let nav_origin = origin.clone();
    let nav_session = session.clone();
    let load_origin = origin.clone();
    let load_session = session.clone();
    let marker = format!("if(location.origin === {}) {{ window.__KABACUS_NATIVE__=true; sessionStorage.removeItem('kabacus.native-token'); }}", serde_json::to_string(&origin).unwrap());
    let window =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url.clone()))
            .title("Kabacus")
            .inner_size(1440.0, 960.0)
            .min_inner_size(960.0, 640.0)
            .incognito(true)
            .initialization_script(&marker)
            .on_navigation(move |target| {
                if target.origin().ascii_serialization() != nav_origin {
                    return false;
                }
                if target.path() == "/app/" && target.query() == Some("native-settings=1") {
                    if let Some(window) = app_handle.get_webview_window("connection") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    return false;
                }
                if target.path() == "/app/" && target.query() == Some("native-signout=1") {
                    let session = nav_session.clone();
                    let app = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = session.sign_out().await;
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.close();
                        }
                    });
                    return false;
                }
                true
            })
            .on_page_load(move |window, payload| {
                if payload.event() != tauri::webview::PageLoadEvent::Finished {
                    return;
                }
                let session = load_session.clone();
                let origin = load_origin.clone();
                let window = window.clone();
                tauri::async_runtime::spawn(async move {
                    let token = session.token().await.ok();
                    let username = session.session.username().await.unwrap_or_default();
                    let _ = window.eval(&token_script(&origin, token.as_deref(), &username));
                });
            })
            .build()
            .map_err(|e| e.to_string())?;
    // Only short-lived access tokens reach the hosted UI; refresh tokens stay in memory.
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let token = session.token().await.ok();
            let username = session.session.username().await.unwrap_or_default();
            if window
                .eval(&token_script(&origin, token.as_deref(), &username))
                .is_err()
            {
                break;
            }
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![connect_server])
        .setup(|app| {
            let default_server = std::env::var("KABACUS_SERVER_URL").unwrap_or_default();
            let marker = format!(
                "window.__KABACUS_LAUNCHER__=true; window.__KABACUS_DEFAULT_SERVER__={};",
                serde_json::to_string(&default_server).unwrap()
            );
            tauri::WebviewWindowBuilder::new(
                app,
                "connection",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Kabacus")
            .inner_size(640.0, 560.0)
            .initialization_script(&marker)
            .on_navigation(trusted_local_url)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Unable to start Kabacus");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_commands_only_accept_bundled_origins() {
        for value in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
        ] {
            assert!(trusted_local_url(&url::Url::parse(value).unwrap()));
        }
        for value in [
            "https://localhost/index.html",
            "https://books.example.org/app/",
            "https://tauri.localhost.evil.example/",
            "file:///index.html",
        ] {
            assert!(!trusted_local_url(&url::Url::parse(value).unwrap()));
        }
    }
    #[test]
    fn requires_a_secure_origin_without_credentials_or_paths() {
        assert_eq!(
            server_url("https://books.example.org:8443")
                .unwrap()
                .as_str(),
            "https://books.example.org:8443/app/"
        );
        for value in [
            "http://books.example.org",
            "file:///books",
            "https://user:secret@books.example.org",
            "https://books.example.org/other",
            "https://books.example.org/?token=secret",
        ] {
            assert!(server_url(value).is_err(), "{value}");
        }
    }
}
