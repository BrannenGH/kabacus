// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
use kabacus_client::{
    oauth::{
        self,
        app::{BrowserSignIn, UserSession},
        TokenSource,
    },
    KabacusClient,
};
#[cfg(target_os = "linux")]
use kabacus_kde::secrets::{SecretAccount, SecretServiceStore, SecretStore};
use std::sync::Arc;
use tauri::Manager;

// ponytail: only Linux has a keychain (Secret Service), so elsewhere the
// refresh token is kept for the session and the app signs in on each launch.
// Add the platform keychains when those builds are released.
#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
#[allow(dead_code)]
struct SecretAccount {
    server: String,
    issuer: String,
}
#[cfg(not(target_os = "linux"))]
struct SecretServiceStore;
#[cfg(not(target_os = "linux"))]
impl SecretServiceStore {
    async fn load(&self, _: &SecretAccount) -> Result<Option<String>, String> {
        Ok(None)
    }
    async fn save(&self, _: &SecretAccount, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn clear(&self, _: &SecretAccount) -> Result<(), String> {
        Ok(())
    }
}

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

#[derive(Clone)]
struct NativeSession {
    session: UserSession,
    account: SecretAccount,
    store: Arc<SecretServiceStore>,
}
impl NativeSession {
    async fn sign_out(&self) {
        let _ = self.session.sign_out().await;
        let _ = self.store.clear(&self.account).await;
    }
}
async fn sign_in(
    origin: &str,
    private_ca: Option<&[u8]>,
    interactive: bool,
) -> Result<NativeSession, Box<dyn std::error::Error + Send + Sync>> {
    let client = KabacusClient::connect_https(origin, private_ca)?;
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
            if !interactive {
                return Err("Sign in to this server".into());
            }
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

use kabacus_client::{
    books::computer_user,
    control::ControlChannel,
    workspace::{Source, WindowState, Workspace},
    BooksRequest, BooksResponse, LocalBooks,
};
use kabacus_service::local::LocalCaller;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::Mutex;

struct WindowBooks {
    client: KabacusClient,
    source: Source,
    control: Option<ControlChannel>,
    session: Option<NativeSession>,
}
struct NativeState {
    windows: Mutex<HashMap<String, WindowBooks>>,
    quitting: Mutex<Option<std::collections::HashSet<String>>>,
    workspace: Mutex<Workspace>,
    path: PathBuf,
}
fn trust(window: &tauri::WebviewWindow) -> Result<(), String> {
    if trusted_local_url(&window.url().map_err(|e| e.to_string())?) {
        Ok(())
    } else {
        Err("Untrusted window".into())
    }
}
fn metadata(source: &Source) -> Value {
    match source {
        Source::Local { path, name } => {
            json!({"id":source.key(),"name":name,"filename":path.file_name().unwrap_or_default().to_string_lossy(),"location":path.parent().unwrap_or(std::path::Path::new("")).display().to_string(),"local":true,"source":source})
        }
        Source::Remote { url, name, .. } => {
            json!({"id":source.key(),"name":name,"filename":url,"location":url,"local":false,"source":source})
        }
    }
}
fn private_ca(path: &str) -> Result<Option<Vec<u8>>, String> {
    if path.trim().is_empty() {
        Ok(None)
    } else {
        std::fs::read(path).map(Some).map_err(|e| e.to_string())
    }
}
async fn install(
    window: &tauri::WebviewWindow,
    app: &tauri::AppHandle,
    state: &NativeState,
    source: Source,
    client: KabacusClient,
) -> Result<Value, String> {
    let mut windows = state.windows.lock().await;
    if let Some((id, _)) = windows
        .iter()
        .find(|(id, b)| id.as_str() != window.label() && b.source.key() == source.key())
    {
        if let Some(other) = app.get_webview_window(id) {
            other.show().map_err(|e| e.to_string())?;
            other.set_focus().map_err(|e| e.to_string())?;
        }
        return Ok(json!({"focused":true}));
    }
    window
        .set_title(&format!("{} — Kabacus", source.name()))
        .map_err(|e| e.to_string())?;
    windows.insert(
        window.label().into(),
        WindowBooks {
            client,
            source: source.clone(),
            control: None,
            session: None,
        },
    );
    drop(windows);
    let mut workspace = state.workspace.lock().await;
    let args: Vec<String> = std::env::args().collect();
    let requested = args
        .iter()
        .position(|arg| arg == "--control")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u16>().ok());
    let preferences = workspace.windows.iter().find(|w| w.id == window.label());
    if preferences.is_some_and(|w| w.automation) || requested.is_some() {
        let port = requested.unwrap_or(preferences.map_or(7979, |w| w.control_port));
        if let Some(books) = state.windows.lock().await.get_mut(window.label()) {
            books.control = Some(
                ControlChannel::for_client(books.client.clone(), port)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    let mut record = workspace
        .windows
        .iter()
        .find(|w| w.id == window.label())
        .cloned()
        .unwrap_or_else(|| WindowState::welcome(window.label()));
    record.source = Some(source.clone());
    workspace.remember(record);
    workspace.save(&state.path).map_err(|e| e.to_string())?;
    Ok(metadata(&source))
}
#[tauri::command]
async fn configure_menu(
    window: tauri::WebviewWindow,
    labels: HashMap<String, String>,
) -> Result<(), String> {
    trust(&window)?;
    #[cfg(desktop)]
    {
        use tauri::menu::{IsMenuItem, Menu, MenuItem, Submenu};
        let definitions = [
            ("new-window", Some("CmdOrCtrl+Shift+N")),
            ("new", None),
            ("open", Some("CmdOrCtrl+O")),
            ("recent", None),
            ("save-copy", Some("CmdOrCtrl+Shift+S")),
            ("close", Some("CmdOrCtrl+Shift+W")),
            ("connect", None),
            ("quit", Some("CmdOrCtrl+Q")),
        ];
        let items = definitions
            .iter()
            .map(|(id, key)| {
                MenuItem::with_id(
                    &window,
                    *id,
                    labels.get(*id).map(String::as_str).unwrap_or(id),
                    true,
                    *key,
                )
            })
            .collect::<tauri::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let refs = items
            .iter()
            .map(|item| item as &dyn IsMenuItem<_>)
            .collect::<Vec<_>>();
        let submenu = Submenu::with_items(
            &window,
            labels.get("file").map(String::as_str).unwrap_or("File"),
            true,
            &refs,
        )
        .map_err(|e| e.to_string())?;
        let menu = Menu::with_items(&window, &[&submenu]).map_err(|e| e.to_string())?;
        window.set_menu(menu).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
async fn choose_books(window: tauri::WebviewWindow, save: bool) -> Result<Option<String>, String> {
    trust(&window)?;
    #[cfg(not(desktop))]
    {
        let _ = save;
        return Err("File dialogs require the desktop app".into());
    }
    #[cfg(desktop)]
    {
        let dialog = rfd::AsyncFileDialog::new().add_filter("Kabacus", &["kabacus"]);
        let selected = if save {
            dialog.set_file_name("Household.kabacus").save_file().await
        } else {
            dialog.pick_file().await
        };
        Ok(selected.map(|p| p.path().to_string_lossy().into_owned()))
    }
}
#[tauri::command]
async fn open_books(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
    source: Source,
) -> Result<Value, String> {
    trust(&window)?;
    let (source, client) = match source {
        Source::Local { path, .. } => {
            let books = LocalBooks::open(path, LocalCaller::for_app(computer_user()))
                .await
                .map_err(|e| e.to_string())?;
            let name = books.name().await.map_err(|e| e.to_string())?;
            (
                Source::Local {
                    path: books.path.clone().unwrap_or_default(),
                    name,
                },
                KabacusClient::from_local(books),
            )
        }
        Source::Remote {
            url,
            name,
            private_ca: ca_path,
        } => {
            let origin = server_url(&url)?.origin().ascii_serialization();
            let ca = private_ca(&ca_path)?;
            let mut client =
                KabacusClient::connect_https(&origin, ca.as_deref()).map_err(|e| e.to_string())?;
            let oauth = client.version().await.is_ok_and(|v| v.auth.mode == "oauth");
            let session = if oauth {
                sign_in(&origin, ca.as_deref(), false).await.ok()
            } else {
                None
            };
            if let Some(session) = &session {
                client = client.with_token_source(Arc::new(session.session.clone()));
            }
            let source = Source::Remote {
                url: origin,
                name,
                private_ca: ca_path,
            };
            let mut metadata = install(&window, &app, &state, source, client).await?;
            if metadata["focused"] == true {
                return Ok(metadata);
            }
            metadata["needs_sign_in"] = json!(oauth && session.is_none());
            metadata["oauth"] = json!(oauth);
            if let Some(books) = state.windows.lock().await.get_mut(window.label()) {
                books.session = session;
            }
            return Ok(metadata);
        }
    };
    install(&window, &app, &state, source, client).await
}
#[tauri::command]
async fn new_books(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
    path: String,
    name: String,
    currency: String,
    account: String,
) -> Result<Value, String> {
    trust(&window)?;
    let books = LocalBooks::create(
        path,
        &name,
        &currency,
        &account,
        LocalCaller::for_app(computer_user()),
    )
    .await
    .map_err(|e| e.to_string())?;
    let source = Source::Local {
        path: books.path.clone().unwrap_or_default(),
        name,
    };
    install(
        &window,
        &app,
        &state,
        source,
        KabacusClient::from_local(books),
    )
    .await
}
#[tauri::command]
async fn connect_server(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
    origin: String,
    private_ca: String,
) -> Result<Value, String> {
    trust(&window)?;
    let url = server_url(&origin)?;
    let origin = url.origin().ascii_serialization();
    let ca = crate::private_ca(&private_ca)?;
    let mut client =
        KabacusClient::connect_https(&origin, ca.as_deref()).map_err(|e| e.to_string())?;
    let session = if client.version().await.map_err(|e| e.to_string())?.auth.mode == "oauth" {
        Some(
            sign_in(&origin, ca.as_deref(), true)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    if let Some(session) = &session {
        client = client.with_token_source(Arc::new(session.session.clone()));
    }
    let mut metadata = install(
        &window,
        &app,
        &state,
        Source::Remote {
            url: origin,
            name: url.host_str().unwrap_or("Server").into(),
            private_ca,
        },
        client,
    )
    .await?;
    if metadata["focused"] == true {
        return Ok(metadata);
    }
    metadata["oauth"] = json!(session.is_some());
    if let Some(books) = state.windows.lock().await.get_mut(window.label()) {
        books.session = session;
    }
    Ok(metadata)
}
#[tauri::command]
async fn sign_out(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<(), String> {
    trust(&window)?;
    let session = state
        .windows
        .lock()
        .await
        .get_mut(window.label())
        .and_then(|books| {
            books.control = None;
            books.session.take()
        });
    if let Some(session) = session {
        session.sign_out().await;
    }
    Ok(())
}
#[tauri::command]
async fn test_server(
    window: tauri::WebviewWindow,
    origin: String,
    private_ca: String,
) -> Result<Value, String> {
    trust(&window)?;
    let url = server_url(&origin)?;
    let ca = crate::private_ca(&private_ca)?;
    let client = KabacusClient::connect_https(url.origin().ascii_serialization(), ca.as_deref())
        .map_err(|e| e.to_string())?;
    let version = tokio::time::timeout(std::time::Duration::from_secs(10), client.version())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    serde_json::to_value(version).map_err(|e| e.to_string())
}
#[tauri::command]
async fn books_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
    request: BooksRequest,
) -> Result<BooksResponse, String> {
    trust(&window)?;
    let client = state
        .windows
        .lock()
        .await
        .get(window.label())
        .ok_or("No books open")?
        .client
        .clone();
    client
        .request_books(request)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn save_copy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<(), String> {
    trust(&window)?;
    let client = state
        .windows
        .lock()
        .await
        .get(window.label())
        .ok_or("No books open")?
        .client
        .clone();
    #[cfg(not(desktop))]
    {
        let _ = client;
        return Err("Save a copy requires the desktop app".into());
    }
    #[cfg(desktop)]
    {
        let Some(path) = rfd::AsyncFileDialog::new()
            .add_filter("Kabacus", &["kabacus"])
            .set_file_name("Books copy.kabacus")
            .save_file()
            .await
        else {
            return Ok(());
        };
        if let Some(books) = client.local_books().await.map_err(|e| e.to_string())? {
            return books
                .save_copy(path.path())
                .await
                .map_err(|e| e.to_string());
        }
        let ledger = client.version().await.map_err(|e| e.to_string())?.ledger;
        let response = client
            .request_books(BooksRequest {
                method: "GET".into(),
                path: format!("/ledgers/{ledger}/export"),
                head: None,
                body: Value::Null,
            })
            .await
            .map_err(|e| e.to_string())?;
        if response.status >= 400 {
            return Err(String::from_utf8_lossy(&response.body).into_owned());
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.path())
            .map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &response.body).map_err(|e| e.to_string())
    }
}
#[tauri::command]
async fn workspace_state(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<Value, String> {
    trust(&window)?;
    let mut workspace = state.workspace.lock().await.clone();
    let preferences = workspace.windows.iter().find(|w| w.id == window.label());
    let automation = preferences.is_some_and(|w| w.automation);
    let port = preferences.map_or(7979, |w| w.control_port);
    workspace.automation = automation;
    workspace.control_port = port;
    serde_json::to_value(workspace).map_err(|e| e.to_string())
}
#[tauri::command]
async fn remember_window(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
    mut record: WindowState,
) -> Result<(), String> {
    trust(&window)?;
    record.id = window.label().into();
    if let Some(books) = state.windows.lock().await.get(window.label()) {
        record.source = Some(books.source.clone());
    }
    let mut workspace = state.workspace.lock().await;
    workspace.remember(record);
    workspace.save(&state.path).map_err(|e| e.to_string())
}
#[tauri::command]
async fn restore_windows(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<Option<WindowState>, String> {
    trust(&window)?;
    Ok(state
        .workspace
        .lock()
        .await
        .windows
        .iter()
        .find(|w| w.id == window.label())
        .cloned())
}
#[tauri::command]
async fn save_workspace_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
    settings: Workspace,
) -> Result<(), String> {
    trust(&window)?;
    let mut windows = state.windows.lock().await;
    if let Some(books) = windows.get_mut(window.label()) {
        if !settings.automation {
            books.control = None;
        } else if books
            .control
            .as_ref()
            .is_none_or(|c| c.port != settings.control_port)
        {
            books.control = None;
            tokio::task::yield_now().await;
            books.control = Some(
                ControlChannel::for_client(books.client.clone(), settings.control_port)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
    } else if settings.automation {
        return Err("Open books before enabling automation".into());
    }
    drop(windows);
    let mut workspace = state.workspace.lock().await;
    workspace.reopen = settings.reopen;
    if let Some(window) = workspace
        .windows
        .iter_mut()
        .find(|w| w.id == window.label())
    {
        window.automation = settings.automation;
        window.control_port = settings.control_port;
    }
    workspace.save(&state.path).map_err(|e| e.to_string())
}
#[tauri::command]
async fn agent_activity(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<Value, String> {
    trust(&window)?;
    Ok(state
        .windows
        .lock()
        .await
        .get(window.label())
        .and_then(|w| w.control.as_ref())
        .map(|c| json!(c.activity()))
        .unwrap_or(Value::Null))
}
fn make_window(app: &tauri::AppHandle, state: &WindowState) -> Result<(), String> {
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        &state.id,
        tauri::WebviewUrl::App("index.html".into()),
    );
    #[cfg(desktop)]
    let builder = builder
        .inner_size(state.width as f64, state.height as f64)
        .min_inner_size(900.0, 600.0);
    let window = builder.title(state.source.as_ref().map(|s|format!("{} — Kabacus",s.name())).unwrap_or_else(||"Kabacus".into()))
 .initialization_script(&format!("window.__KABACUS_LAUNCHER__=true;window.__KABACUS_NATIVE__=true;window.__KABACUS_WINDOW_ID__={};",json!(state.id)))
 .on_navigation(trusted_local_url).build().map_err(|e|e.to_string())?;
    #[cfg(desktop)]
    window.on_menu_event(|window, event| {
        if let Some(webview) = window.get_webview_window(window.label()) {
            let _ = webview.eval(&format!(
                "window.dispatchEvent(new CustomEvent('kabacus-file-action',{{detail:{}}}))",
                json!(event.id().as_ref())
            ));
        }
    });
    let webview = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = webview.eval(
                "window.dispatchEvent(new CustomEvent('kabacus-file-action',{detail:'close'}))",
            );
        }
    });
    Ok(())
}
#[tauri::command]
async fn new_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
    source: Option<Source>,
) -> Result<(), String> {
    trust(&window)?;
    let id = format!(
        "window-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut record = WindowState::welcome(id);
    record.source = source;
    {
        let mut workspace = state.workspace.lock().await;
        workspace.remember(record.clone());
        workspace.save(&state.path).map_err(|e| e.to_string())?;
    }
    make_window(&app, &record)
}
#[tauri::command]
async fn close_window(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeState>,
) -> Result<(), String> {
    trust(&window)?;
    state.windows.lock().await.remove(window.label());
    let mut workspace = state.workspace.lock().await;
    workspace.close(window.label());
    workspace.save(&state.path).map_err(|e| e.to_string())?;
    window.destroy().map_err(|e| e.to_string())
}
#[tauri::command]
async fn quit(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
) -> Result<(), String> {
    trust(&window)?;
    let windows = app.webview_windows();
    *state.quitting.lock().await = Some(windows.keys().cloned().collect());
    for window in windows.values() {
        window
            .eval("window.dispatchEvent(new CustomEvent('kabacus-quit-check'))")
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
async fn approve_quit(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeState>,
    approved: bool,
) -> Result<(), String> {
    trust(&window)?;
    let mut pending = state.quitting.lock().await;
    if !approved {
        *pending = None;
        return Ok(());
    }
    if let Some(windows) = pending.as_mut() {
        windows.remove(window.label());
        if windows.is_empty() {
            app.exit(0);
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            approve_quit,
            configure_menu,
            sign_out,
            choose_books,
            open_books,
            new_books,
            connect_server,
            test_server,
            books_request,
            save_copy,
            workspace_state,
            remember_window,
            restore_windows,
            save_workspace_settings,
            agent_activity,
            new_window,
            close_window,
            quit
        ])
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("windows.json");
            let mut workspace = Workspace::load(&path);
            let chosen = std::env::args()
                .skip(1)
                .find(|arg| arg.ends_with(".kabacus"));
            let restored = if let Some(path) = chosen {
                let mut record = WindowState::welcome("window-main");
                let path = std::fs::canonicalize(path)?;
                record.source = Some(Source::Local {
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    path,
                });
                vec![record]
            } else if workspace.reopen && !workspace.windows.is_empty() {
                workspace.windows.clone()
            } else {
                vec![WindowState::welcome("window-main")]
            };
            for window in &restored {
                workspace.remember(window.clone());
            }
            workspace.save(&path)?;
            app.manage(NativeState {
                windows: Mutex::new(HashMap::new()),
                quitting: Mutex::new(None),
                workspace: Mutex::new(workspace),
                path,
            });
            for window in restored {
                make_window(app.handle(), &window).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Unable to start Kabacus");
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_windows_cannot_navigate_to_untrusted_origins() {
        assert!(trusted_local_url(
            &url::Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(!trusted_local_url(
            &url::Url::parse("https://books.example.org/app/").unwrap()
        ));
    }
    #[test]
    fn server_addresses_are_https_origins() {
        assert!(server_url("https://books.example.org:8443").is_ok());
        for url in [
            "http://books.example.org",
            "https://user:password@books.example.org",
            "https://books.example.org/path",
        ] {
            assert!(server_url(url).is_err());
        }
    }
}
