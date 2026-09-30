/*
 * SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

#pragma once

// The app's logging category, as ecm_qt_declare_logging_category would
// declare it: debug off by default, turned on with
// QT_LOGGING_RULES="me.hallb.kabacus.debug=true" or kdebugsettings.

#include <QtCore/QLoggingCategory>
#include <QtCore/QString>

#include "rust/cxx.h"

inline const QLoggingCategory &KABACUS_LOG()
{
    static const QLoggingCategory category("me.hallb.kabacus", QtInfoMsg);
    return category;
}

// level: 0 debug, 1 info, 2 warning, 3 critical.
inline void kabacus_log(int level, rust::Str message)
{
    const QString text = QString::fromUtf8(message.data(), qsizetype(message.size()));
    switch (level) {
    case 0:
        qCDebug(KABACUS_LOG).noquote() << text;
        break;
    case 1:
        qCInfo(KABACUS_LOG).noquote() << text;
        break;
    case 2:
        qCWarning(KABACUS_LOG).noquote() << text;
        break;
    default:
        qCCritical(KABACUS_LOG).noquote() << text;
        break;
    }
}
