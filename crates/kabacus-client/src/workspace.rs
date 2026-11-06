// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Window and recent-book state belongs to the application, outside the books.

use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    Local {
        path: PathBuf,
        name: String,
    },
    Remote {
        url: String,
        name: String,
        #[serde(default)]
        private_ca: String,
    },
}
impl Source {
    pub fn key(&self) -> String {
        match self {
            Self::Local { path, .. } => format!("file:{}", path.display()),
            Self::Remote { url, .. } => url.trim_end_matches('/').into(),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Local { name, .. } | Self::Remote { name, .. } => name,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub id: String,
    pub source: Option<Source>,
    #[serde(default)]
    pub automation: bool,
    #[serde(default = "control_port")]
    pub control_port: u16,
    #[serde(default = "overview")]
    pub page: String,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub selected: String,
    #[serde(default = "width")]
    pub width: u32,
    #[serde(default = "height")]
    pub height: u32,
}
fn control_port() -> u16 {
    7979
}
fn overview() -> String {
    "Overview".into()
}
fn width() -> u32 {
    1280
}
fn height() -> u32 {
    800
}
impl WindowState {
    pub fn welcome(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: None,
            automation: false,
            control_port: control_port(),
            page: overview(),
            account: String::new(),
            selected: String::new(),
            width: width(),
            height: height(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default = "yes")]
    pub reopen: bool,
    #[serde(default)]
    pub windows: Vec<WindowState>,
    #[serde(default)]
    pub recent: Vec<Source>,
    #[serde(default)]
    pub automation: bool,
    #[serde(default)]
    pub control_port: u16,
}
fn yes() -> bool {
    true
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            reopen: true,
            windows: Vec::new(),
            recent: Vec::new(),
            automation: false,
            control_port: 7979,
        }
    }
}
impl Workspace {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
    }
    pub fn remember(&mut self, mut window: WindowState) {
        if let Some(source) = &window.source {
            self.recent.retain(|s| s.key() != source.key());
            self.recent.insert(0, source.clone());
            self.recent.truncate(20);
        }
        if let Some(old) = self.windows.iter_mut().find(|w| w.id == window.id) {
            window.automation = old.automation;
            window.control_port = old.control_port;
            *old = window;
        } else {
            self.windows.push(window);
        }
    }
    pub fn close(&mut self, id: &str) {
        self.windows.retain(|w| w.id != id);
    }
    pub fn already_open(&self, source: &Source, except: &str) -> Option<&WindowState> {
        self.windows
            .iter()
            .find(|w| w.id != except && w.source.as_ref().is_some_and(|s| s.key() == source.key()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_windows_reopen_with_independent_selection_and_recents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("windows.json");
        let mut state = Workspace::default();
        for (id, name, account) in [
            ("one", "Household", "assets:bank:checking"),
            ("two", "Cabin", "expenses:repairs"),
        ] {
            let mut window = WindowState::welcome(id);
            window.source = Some(Source::Local {
                path: dir.path().join(format!("{name}.kabacus")),
                name: name.into(),
            });
            window.page = "Register".into();
            window.account = account.into();
            window.selected = "18".into();
            state.remember(window);
        }
        state.save(&path).unwrap();
        let loaded = Workspace::load(&path);
        assert!(loaded.reopen);
        assert_eq!(loaded.windows.len(), 2);
        assert_eq!(loaded.windows[1].account, "expenses:repairs");
        assert_eq!(loaded.windows[0].selected, "18");
        assert!(loaded
            .already_open(loaded.windows[1].source.as_ref().unwrap(), "one")
            .is_some());
        assert!(!std::fs::read_to_string(path).unwrap().contains("token"));
    }
}
