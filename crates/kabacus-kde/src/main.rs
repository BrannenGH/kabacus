// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus`: the Kirigami app. Talks only to kabacus-server.

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

// The library holds the bridge and QML module; without a reference rustc
// wouldn't link it (and its Qt initializers) into the binary.
extern crate kabacus_kde;

mod document;

fn main() {
    let file = std::env::args_os().skip(1).find(|arg| {
        std::path::Path::new(arg).extension().is_some_and(|extension| extension == "kabacus")
    });
    let _document = file.map(|file| {
        document::DocumentProcess::open(std::path::Path::new(&file)).unwrap_or_else(|error| {
            eprintln!("Unable to open the document: {error}");
            std::process::exit(1);
        })
    });
    cxx_qt::init_crate!(kabacus_kde);
    kabacus_kde::logging::init();
    let mut app = QGuiApplication::new();
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
