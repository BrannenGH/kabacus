// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// On a phone, the destinations that don't fit the bottom tabs.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "more"
    title: i18nc("the other destinations", "More")

    required property var app

    ListView {
        model: [
            // `name` is the destination's key (Main.qml's `pages`), `label`
            // what's shown for it.
            { name: "Cards", label: page.app.labels.Cards, icon: "view-credit-card" },
            { name: "Loans", label: page.app.labels.Loans, icon: "view-bank" },
            { name: "Reports", label: page.app.labels.Reports, icon: "office-chart-bar" },
            { name: "Activity", label: page.app.labels.Activity, icon: "view-history" },
            { name: "Settings", label: i18n("Settings"), icon: "configure" }
        ]
        delegate: Controls.ItemDelegate {
            required property var modelData
            width: ListView.view.width
            text: modelData.label
            icon.name: modelData.icon
            Accessible.name: modelData.label
            onClicked: modelData.name === "Settings" ? page.app.openSettings() : page.app.go(modelData.name)
        }
    }
}
