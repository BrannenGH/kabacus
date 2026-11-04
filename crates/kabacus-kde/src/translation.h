/*
 * SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

#pragma once

// KI18n for the app (src/translation.rs): the catalog domain, i18n() and
// friends in QML, and lookups for the Rust side's i18n! macros.

#include <cstdint>

#include <QtCore/QByteArray>
#include <QtCore/QString>
#include <QtDBus/QDBusConnection>
#include <QtDBus/QDBusConnectionInterface>
#include <QtGui/QGuiApplication>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlContext>

#include <KLocalizedString>
// KF 6.8 replaced KLocalizedContext with KLocalizedQmlContext, in its own
// library; build.rs defines KABACUS_KI18N_QML when that library is there.
// Older KF6 (or a build without it) still has KLocalizedContext.
#ifdef KABACUS_KI18N_QML
#include <KLocalizedQmlContext>
#else
#include <KLocalizedContext>
#endif

#include "rust/cxx.h"

// Before any lookup and before QML loads: everything the app shows is in
// the "kabacus" catalog (share/locale/<lang>/LC_MESSAGES/kabacus.mo).
inline void kabacus_set_domain()
{
    KLocalizedString::setApplicationDomain(QByteArrayLiteral("kabacus"));
}

// Mirrors the app for a language written right to left (translation.rs
// decides from the catalog). Kirigami's windows turn on LayoutMirroring
// from this, so layouts, alignment and the drawers all flip.
inline void kabacus_set_right_to_left(bool right_to_left)
{
    QGuiApplication::setLayoutDirection(right_to_left ? Qt::RightToLeft : Qt::LeftToRight);
}

// Whether a global menu (Plasma's) is registered on the session bus: the
// platform theme then exports the window's native menu bar there, and the
// in-window one would be a duplicate.
inline bool kabacus_has_global_menu()
{
    auto bus = QDBusConnection::sessionBus();
    return bus.isConnected() && bus.interface()
        && bus.interface()->isServiceRegistered(QStringLiteral("com.canonical.AppMenu.Registrar"));
}

// Gives QML i18n(), i18nc(), i18np() and i18ncp().
inline void kabacus_setup_qml(QQmlApplicationEngine &engine)
{
#ifdef KABACUS_KI18N_QML
    auto *context = new KLocalizedQmlContext(&engine);
#else
    auto *context = new KLocalizedContext(&engine);
#endif
    engine.rootContext()->setContextObject(context);
    engine.setInitialProperties({{QStringLiteral("globalMenuRegistered"), kabacus_has_global_menu()}});
}

inline QByteArray kabacus_bytes(rust::Str text)
{
    return QByteArray(text.data(), qsizetype(text.size()));
}

// One lookup for the Rust macros (crate::i18n::Translator): the count
// (when plural) is %1, then the arguments in order. KI18n's lookups are
// thread-safe, and the screens are built on the tokio runtime's threads.
inline rust::String kabacus_translate(rust::Str context, rust::Str singular, rust::Str plural,
                                      bool has_count, std::int64_t count,
                                      rust::Slice<const rust::String> args)
{
    // KLocalizedString keeps copies, but these must outlive the
    // ki18n*() calls, which take C strings.
    const QByteArray c = kabacus_bytes(context);
    const QByteArray s = kabacus_bytes(singular);
    const QByteArray p = kabacus_bytes(plural);
    KLocalizedString text;
    if (has_count) {
        text = c.isEmpty() ? ki18np(s.constData(), p.constData())
                           : ki18ncp(c.constData(), s.constData(), p.constData());
        text = text.subs(qlonglong(count));
    } else {
        text = c.isEmpty() ? ki18n(s.constData()) : ki18nc(c.constData(), s.constData());
    }
    for (const rust::String &arg : args) {
        text = text.subs(QString::fromUtf8(arg.data(), qsizetype(arg.size())));
    }
    const QByteArray out = text.toString().toUtf8();
    return rust::String(out.constData(), std::size_t(out.size()));
}
