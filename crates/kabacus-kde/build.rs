// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Where KDE Frameworks' KI18n is (src/translation.h). KF6 ships CMake
/// files, not pkg-config ones, so its headers are looked for where every
/// supported distribution and the KDE flatpak SDK put them, under
/// `<prefix>/include/KF6`; `KF6_INCLUDE_DIR` points elsewhere.
#[cfg(feature = "gui")]
struct Ki18n {
    include_dirs: Vec<std::path::PathBuf>,
    /// KF 6.8 and later: KLocalizedQmlContext, in KI18nQml.
    qml: bool,
}

#[cfg(feature = "gui")]
impl Ki18n {
    fn find() -> Self {
        use std::path::PathBuf;
        println!("cargo::rerun-if-env-changed=KF6_INCLUDE_DIR");
        let candidates: Vec<PathBuf> = std::env::var_os("KF6_INCLUDE_DIR")
            .map(PathBuf::from)
            .into_iter()
            .chain(
                [
                    "/usr/include/KF6",
                    "/usr/local/include/KF6",
                    "/app/include/KF6",
                ]
                .map(PathBuf::from),
            )
            .collect();
        let Some(kf6) = candidates
            .into_iter()
            .find(|dir| dir.join("KI18n/KLocalizedString").exists())
        else {
            panic!(
                "KDE Frameworks' KI18n headers weren't found (KF6/KI18n/KLocalizedString): \
                 install its development package (kf6-ki18n-devel, libkf6i18n-dev) \
                 or set KF6_INCLUDE_DIR"
            );
        };
        let qml_dir = kf6.join("KI18nQml");
        let qml = qml_dir.join("KLocalizedQmlContext").exists();
        let mut include_dirs = vec![kf6.join("KI18n")];
        if qml {
            include_dirs.push(qml_dir);
        }
        Ki18n { include_dirs, qml }
    }
}

fn main() {
    // The Qt UI is only built with `--features gui` (inside the KDE 6
    // SDK); the model builds and tests without Qt.
    #[cfg(feature = "gui")]
    {
        use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};
        // Theme.qml links to design/generated/Theme.qml, generated from
        // design/tokens.json.
        let pages = [
            "qml/Main.qml",
            "qml/Money.qml",
            "qml/InfoButton.qml",
            "qml/StatusBanner.qml",
            "qml/PreviewPane.qml",
            "qml/DiffView.qml",
            "qml/Fetch.qml",
            "qml/AccountPicker.qml",
            "qml/OverviewPage.qml",
            "qml/AccountsPage.qml",
            "qml/RegisterPage.qml",
            "qml/FundsPage.qml",
            "qml/LoansPage.qml",
            "qml/CardsPage.qml",
            "qml/PeoplePage.qml",
            "qml/StatementPage.qml",
            "qml/ReportsPage.qml",
            "qml/ReconcilePage.qml",
            "qml/ActivityPage.qml",
            "qml/CommitPage.qml",
            "qml/MorePage.qml",
            "qml/SettingsPage.qml",
            "qml/AddChooser.qml",
            "qml/TransactionForm.qml",
            "qml/ReviewPage.qml",
        ];
        for page in pages.iter().chain(["qml/Theme.qml"].iter()) {
            println!("cargo::rerun-if-changed={page}");
        }
        let ki18n = Ki18n::find();
        let builder = CxxQtBuilder::new_qml_module(
            QmlModule::new("me.hallb.kabacus")
                .qml_files(pages)
                .qml_file(QmlFile::from("qml/Theme.qml").singleton(true)),
        )
        .qt_module("Quick")
        .files(["src/bridge.rs", "src/logging.rs", "src/translation.rs"]);
        // SAFETY: only adds include directories and a define.
        let builder = unsafe {
            builder.cc_builder(|cc| {
                for dir in &ki18n.include_dirs {
                    cc.include(dir);
                }
                if ki18n.qml {
                    cc.define("KABACUS_KI18N_QML", None);
                }
            })
        };
        builder.build();
        println!("cargo::rustc-link-lib=KF6I18n");
        if ki18n.qml {
            println!("cargo::rustc-link-lib=KF6I18nQml");
        }
    }
}
