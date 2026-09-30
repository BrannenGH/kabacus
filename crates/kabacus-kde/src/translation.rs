// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! KI18n for the gui (`translation.h`): the `kabacus` catalog for QML's
//! `i18n()` and for [`crate::i18n`]'s macros, so both are translated from
//! the same `.mo` files, in the language KDE's settings (or `LANGUAGE`)
//! ask for.

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("kabacus-kde/src/translation.h");
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        fn kabacus_set_domain();
        fn kabacus_set_right_to_left(right_to_left: bool);
        fn kabacus_setup_qml(engine: Pin<&mut QQmlApplicationEngine>);
        fn kabacus_translate(
            context: &str,
            singular: &str,
            plural: &str,
            has_count: bool,
            count: i64,
            args: &[String],
        ) -> String;
    }
}

pub use ffi::QQmlApplicationEngine;

fn translate(
    context: &str,
    singular: &str,
    plural: &str,
    count: Option<i64>,
    args: &[String],
) -> String {
    ffi::kabacus_translate(
        context,
        singular,
        plural,
        count.is_some(),
        count.unwrap_or(0),
        args,
    )
}

/// After the application exists, before anything is shown or looked up:
/// the catalog domain, the Rust side's macros through KI18n, and the
/// layout direction.
pub fn init() {
    ffi::kabacus_set_domain();
    crate::i18n::set_translator(translate);
    // KI18n doesn't set Qt's layout direction, so the catalog says which
    // way its language is written. Arabic, Hebrew, Persian and Uyghur
    // translate this as RTL, which mirrors the whole app: Kirigami follows
    // the application's layout direction. `cargo xtask i18n-extract
    // --check` holds every catalog to its script's direction.
    let rtl = crate::i18nc!(
        "Which way this language is written: translate as RTL if right to left, LTR otherwise",
        "LTR"
    ) == "RTL";
    ffi::kabacus_set_right_to_left(rtl);
}

/// Before `engine` loads any QML.
pub fn setup_qml(engine: std::pin::Pin<&mut QQmlApplicationEngine>) {
    ffi::kabacus_setup_qml(engine);
}
