// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's settings: how it reaches kabacus-server, either
//! the local unix socket or a remote HTTPS address. Stored in
//! KConfig's format in `$XDG_CONFIG_HOME/kabacusrc`.

use crate::model::ModelError;
use crate::{i18n, i18nc, i18ncp};
use kabacus_client::{ClientError, KabacusClient};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GROUP: &str = "Connection";

/// How long a connection test waits before giving up.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionKind {
    /// kabacus-server on this computer, over its unix socket.
    #[default]
    Local,
    /// kabacus-server on another computer, over HTTPS.
    Remote,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Settings {
    pub kind: ConnectionKind,
    /// Local: the server's socket; empty means [`default_socket`].
    pub socket_path: String,
    /// Local books opened directly through the shared libraries.
    pub books_path: String,
    /// Remote: the server's `https://` address.
    pub server_url: String,
    /// Remote: a PEM file of extra CAs to trust, e.g. a private CA; empty
    /// means the system trust store only.
    pub private_ca: String,
}

/// The server's default unix socket.
pub fn default_socket() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(runtime_dir).join("kabacus/server.sock")
}

/// `$XDG_CONFIG_HOME/kabacusrc`, where KConfig keeps the app's settings.
pub fn config_path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("kabacusrc")
}

impl Settings {
    /// The settings in `path`; defaults for anything missing or unreadable.
    pub fn load(path: &Path) -> Settings {
        let groups = std::fs::read_to_string(path)
            .map(|text| parse_kconfig(&text))
            .unwrap_or_default();
        let get = |key: &str| {
            groups
                .get(GROUP)
                .and_then(|g| g.get(key))
                .cloned()
                .unwrap_or_default()
        };
        Settings {
            kind: if get("Kind") == "remote" {
                ConnectionKind::Remote
            } else {
                ConnectionKind::Local
            },
            socket_path: get("SocketPath"),
            books_path: get("BooksPath"),
            server_url: get("ServerUrl"),
            private_ca: get("PrivateCa"),
        }
    }

    /// What the app starts with: the saved settings, unless
    /// `KABACUS_SOCKET` names a socket to use instead (scratch servers and
    /// the GUI e2e tests). The override is never saved.
    pub fn for_app() -> Settings {
        match std::env::var("KABACUS_SOCKET") {
            Ok(socket) if !socket.is_empty() => Settings {
                kind: ConnectionKind::Local,
                socket_path: socket,
                books_path: String::new(),
                ..Settings::load(&config_path())
            },
            _ => Settings::default(),
        }
    }

    /// Write these settings to `path`, keeping any other groups and keys.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut groups = std::fs::read_to_string(path)
            .map(|text| parse_kconfig(&text))
            .unwrap_or_default();
        let group = groups.entry(GROUP.to_string()).or_default();
        let kind = match self.kind {
            ConnectionKind::Local => "local",
            ConnectionKind::Remote => "remote",
        };
        for (key, value) in [
            ("Kind", kind),
            ("SocketPath", self.socket_path.trim()),
            ("BooksPath", self.books_path.trim()),
            ("ServerUrl", self.server_url.trim()),
            ("PrivateCa", self.private_ca.trim()),
        ] {
            group.insert(key.to_string(), value.to_string());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, write_kconfig(&groups))?;
        std::fs::rename(tmp, path)
    }

    /// Which server these settings reach, as the secret store knows it:
    /// the remote address, or `unix:<socket>`.
    pub fn server_id(&self) -> String {
        match self.kind {
            ConnectionKind::Remote => self.server_url.trim().trim_end_matches('/').to_string(),
            ConnectionKind::Local => {
                if !self.books_path.is_empty() || self.socket_path.is_empty() {
                    return format!("file:{}", self.books_path);
                }
                let socket = self.socket_path.trim();
                let socket = if socket.is_empty() {
                    default_socket()
                } else {
                    PathBuf::from(socket)
                };
                format!("unix:{}", socket.display())
            }
        }
    }

    /// A client for the configured server. Doesn't contact it yet.
    pub fn connect(&self) -> Result<KabacusClient, ModelError> {
        let client = match self.kind {
            ConnectionKind::Local => {
                if !self.books_path.is_empty() || self.socket_path.is_empty() {
                    return Ok(KabacusClient::open_local(self.books_path.clone()));
                }
                let socket = self.socket_path.trim();
                KabacusClient::connect_unix(if socket.is_empty() {
                    default_socket()
                } else {
                    PathBuf::from(socket)
                })
            }
            ConnectionKind::Remote => {
                let url = self.server_url.trim();
                if url.is_empty() {
                    return Err(invalid(&i18n!("Enter the server's address")));
                }
                let ca = match self.private_ca.trim() {
                    "" => None,
                    file => Some(std::fs::read(file).map_err(|e| {
                        invalid(&i18nc!(
                            "%1 is a file, %2 why it can't be read",
                            "Can't read the private CA %1: %2",
                            file,
                            e
                        ))
                    })?),
                };
                KabacusClient::connect_https(url, ca.as_deref()).map_err(|e| match e {
                    ClientError::Request(m) if m.contains("https://") => {
                        invalid(&i18n!("The server address must start with https://"))
                    }
                    e => invalid(&e.to_string()),
                })?
            }
        };
        Ok(client.with_client_hint("app"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Reached the server and read the books.
    Connected,
    /// Reached the server, but reading the books needs an OAuth sign-in.
    SignInRequired,
}

/// What a connection test found, for the settings page.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionReport {
    pub status: ConnectionStatus,
    pub message: String,
    pub server_version: String,
    pub auth_mode: String,
}

/// Contact the server `settings` points at and try reading the books.
/// Errors are phrased for the settings page.
pub async fn test_connection(settings: &Settings) -> Result<ConnectionReport, ModelError> {
    let client = settings.connect()?;
    let timed_out = || {
        invalid(&i18n!(
            "Can't reach the server: no answer within 10 seconds"
        ))
    };
    let version = tokio::time::timeout(TEST_TIMEOUT, client.version())
        .await
        .map_err(|_| timed_out())?
        .map_err(unreachable)?;
    let accounts = tokio::time::timeout(TEST_TIMEOUT, client.list_accounts(None))
        .await
        .map_err(|_| timed_out())?;
    let (status, message) = match accounts {
        Ok(accounts) => (
            ConnectionStatus::Connected,
            i18ncp!(
                "%2 is the server's version",
                "Connected to kabacus-server %2: %1 account",
                "Connected to kabacus-server %2: %1 accounts",
                accounts.len(),
                version.server_version
            ),
        ),
        Err(ClientError::Unauthorized) => (
            ConnectionStatus::SignInRequired,
            match version.auth.issuer.as_deref() {
                Some(issuer) => i18nc!(
                    "%1 is the server's version, %2 the sign-in provider's address",
                    "Reached kabacus-server %1. It needs you to sign in with %2.",
                    version.server_version,
                    issuer
                ),
                None => i18nc!(
                    "%1 is the server's version",
                    "Reached kabacus-server %1. It needs you to sign in with its OAuth provider.",
                    version.server_version
                ),
            },
        ),
        Err(ClientError::Forbidden(role)) => {
            return Err(invalid(&i18nc!(
                "%1 is a role's name",
                "Reached the server, but you lack the %1 role",
                role
            )))
        }
        Err(e) => return Err(unreachable(e)),
    };
    Ok(ConnectionReport {
        status,
        message,
        server_version: version.server_version,
        auth_mode: version.auth.mode,
    })
}

fn invalid(message: &str) -> ModelError {
    ModelError::Invalid(message.to_string())
}

fn unreachable(e: ClientError) -> ModelError {
    let text = e.to_string();
    if text.contains("UnknownIssuer") || text.contains("certificate") {
        invalid(&i18nc!(
            "%1 is the error",
            "The server's certificate isn't trusted. If it comes from a private CA, choose that CA's file. (%1)",
            text
        ))
    } else {
        invalid(&i18nc!(
            "%1 is the error",
            "Can't reach the server: %1",
            text
        ))
    }
}

type Groups = BTreeMap<String, BTreeMap<String, String>>;

/// KConfig's INI dialect: `[Group]` headers, `Key=Value`, `#` comments,
/// and backslash escapes in values.
fn parse_kconfig(text: &str) -> Groups {
    let mut groups = Groups::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
        } else if let Some((key, value)) = line.split_once('=') {
            groups
                .entry(current.clone())
                .or_default()
                .insert(key.trim().to_string(), unescape(value.trim()));
        }
    }
    groups
}

fn write_kconfig(groups: &Groups) -> String {
    let mut out = String::new();
    for (name, entries) in groups {
        if !out.is_empty() {
            out.push('\n');
        }
        if !name.is_empty() {
            out.push_str(&format!("[{name}]\n"));
        }
        for (key, value) in entries {
            out.push_str(&format!("{key}={}\n", escape(value)));
        }
    }
    out
}

fn escape(value: &str) -> String {
    let mut out = String::new();
    for (i, c) in value.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            // KConfig trims unescaped leading and trailing spaces.
            ' ' if i == 0 || i == value.chars().count() - 1 => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('s') => out.push(' '),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_through_a_kconfig_file_keeping_other_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kabacusrc");
        std::fs::write(&path, "[General]\nAllowRewrite=false\n").unwrap();
        let settings = Settings {
            kind: ConnectionKind::Remote,
            socket_path: String::new(),
            books_path: String::new(),
            server_url: "https://books.lan:8443".to_string(),
            private_ca: " /home/me/ca\\dir/ca.pem".to_string(),
        };

        settings.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[Connection]\n"), "{text}");
        assert!(text.contains("Kind=remote\n"), "{text}");
        assert!(text.contains("[General]\nAllowRewrite=false\n"), "{text}");
        let loaded = Settings::load(&path);
        assert_eq!(loaded.kind, ConnectionKind::Remote);
        assert_eq!(loaded.server_url, "https://books.lan:8443");
        // Saved trimmed.
        assert_eq!(loaded.private_ca, "/home/me/ca\\dir/ca.pem");
    }
    #[test]
    fn missing_settings_mean_the_local_default_socket() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::load(&dir.path().join("nothing"));
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.kind, ConnectionKind::Local);
    }

    #[test]
    fn values_with_escapes_survive_a_round_trip() {
        for value in ["a\\b", "line\nbreak", " padded ", "tab\there"] {
            assert_eq!(unescape(&escape(value)), value);
        }
    }
}
