// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! kabacus-kde: the Kirigami app. [`model`] holds everything the
//! UI does, independent of Qt; the `gui` feature adds the cxx-qt bridge.

pub mod auth;
pub mod i18n;
pub mod loads;
pub mod model;
pub mod secrets;
pub mod settings;
pub mod views;
pub mod writes;

#[cfg(feature = "gui")]
pub mod bridge;
#[cfg(feature = "gui")]
pub mod logging;
#[cfg(feature = "gui")]
pub mod translation;
