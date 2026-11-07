// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("cli") {
        let cli = if cfg!(windows) {
            "kabacus-cli.exe"
        } else {
            "kabacus-cli"
        };
        let program = std::env::current_exe()
            .ok()
            .map(|p| p.with_file_name(cli))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| cli.into());
        match std::process::Command::new(program)
            .args(std::env::args().skip(2))
            .status()
        {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    kabacus_desktop_lib::run()
}
