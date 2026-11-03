// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A file opened from the desktop gets a private gateway for the window's lifetime.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct DocumentProcess {
    child: Child,
    _runtime: tempfile::TempDir,
}

impl DocumentProcess {
    pub fn open(file: &Path) -> Result<Self, String> {
        let file = file.canonicalize().map_err(|e| e.to_string())?;
        let runtime = tempfile::Builder::new().prefix("kabacus-window-").tempdir().map_err(|e| e.to_string())?;
        let socket = runtime.path().join("server.sock");
        let sibling = std::env::current_exe().map_err(|e| e.to_string())?
            .with_file_name("kabacus-server");
        let executable = if sibling.is_file() { sibling.into_os_string() } else { "kabacus-server".into() };
        let child = Command::new(executable)
            .arg("--auth=none")
            .arg("--document").arg(file)
            .arg("--socket").arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn().map_err(|e| e.to_string())?;
        let mut process = Self { child, _runtime: runtime };
        let started = Instant::now();
        loop {
            if let Some(status) = process.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("the document gateway exited: {status}"));
            }
            if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                std::env::set_var("KABACUS_SOCKET", &socket);
                return Ok(process);
            }
            if started.elapsed() > Duration::from_secs(15) {
                return Err("the document gateway did not start in time".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for DocumentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
