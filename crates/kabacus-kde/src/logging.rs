// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app logs the KDE way: through Qt's logging category
//! `me.hallb.kabacus` (`logging.h`), so messages reach the journal like
//! any other Qt app's and debug output is switched on with
//! `QT_LOGGING_RULES="me.hallb.kabacus.debug=true"` or kdebugsettings.
//! Rust code (this crate and kabacus-client) uses the `log` macros;
//! [`init`] sends them to that category.

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("kabacus-kde/src/logging.h");
        fn kabacus_log(level: i32, message: &str);
    }
}

struct QtLogger;

impl log::Log for QtLogger {
    /// Only Kabacus's own crates: the HTTP and TLS libraries log through
    /// `log` too, and their debug output would drown ours.
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().starts_with("kabacus")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            log::Level::Error => 3,
            log::Level::Warn => 2,
            log::Level::Info => 1,
            log::Level::Debug | log::Level::Trace => 0,
        };
        ffi::kabacus_log(level, &record.args().to_string());
    }

    fn flush(&self) {}
}

/// Send `log` records to Qt. Qt's category rules decide what's shown.
pub fn init() {
    static LOGGER: QtLogger = QtLogger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
}
