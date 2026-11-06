// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus`: the Kirigami app for local and remote books.

use cxx_qt_lib::{QQmlApplicationEngine, QString, QUrl};

// The library holds the bridge and QML module; without a reference rustc
// wouldn't link it (and its Qt initializers) into the binary.
extern crate kabacus_kde;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("cli") {
        let cli = if cfg!(windows) {
            "kabacus-cli.exe"
        } else {
            "kabacus-cli"
        };
        let sibling = std::env::current_exe().ok().map(|p| p.with_file_name(cli));
        let program = sibling
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

    cxx_qt::init_crate!(kabacus_kde);
    kabacus_kde::logging::init();
    let mut app = kabacus_kde::translation::application();
    if let Some(mut app) = app.as_mut() {
        // Matches the desktop entry's StartupWMClass; the binary, the
        // package and the settings file are lowercase, the name people
        // see is capitalized, as KDE apps' are.
        app.as_mut().set_application_name(&QString::from("kabacus"));
        app.as_mut()
            .set_application_display_name(&QString::from("Kabacus"));
        app.set_organization_domain(&QString::from("hallb.me"));
    }
    kabacus_kde::translation::init();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        kabacus_kde::translation::setup_qml(engine.as_mut());
        engine.load(&QUrl::from("qrc:/qt/qml/me/hallb/kabacus/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
