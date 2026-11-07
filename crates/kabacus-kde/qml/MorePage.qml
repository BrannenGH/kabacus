// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// On a phone, the destinations that don't fit the bottom tabs.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
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
            { name: "Loans", label: page.app.labels.Loans, icon: "view-bank" },
            { name: "People", label: page.app.labels.People, icon: "system-users" },
            { name: "Reports", label: page.app.labels.Reports, icon: "office-chart-bar" },
            { name: "Activity", label: page.app.labels.Activity, icon: "view-history" },
            { name: "Settings", label: i18n("Settings"), icon: "configure" }
        ]
        spacing: 8
        footer: ColumnLayout {
            width: ListView.view.width
            spacing: 8
            Kirigami.Heading { text: i18n("Books"); topPadding: 24 }
            Controls.Label { text: page.app.bookSource ? page.app.bookSource.name : "Kabacus"; font.bold: true; Layout.fillWidth: true }
            Controls.Label { text: page.app.bookSource && page.app.bookSource.kind === "local" ? i18n("On this computer") + " · " + page.app.bookSource.path : page.app.bookSource ? page.app.bookSource.url : page.app.connection.socket_path; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true }
            Controls.ItemDelegate { text: i18n("Switch Books…"); Layout.fillWidth: true; implicitHeight: 48; onClicked: page.app.openBooksSheet() }
            Controls.ItemDelegate { text: i18n("Connect to a Server…"); Layout.fillWidth: true; implicitHeight: 48; onClicked: page.app.runFileAction(page.app.connectServer) }
        }
        delegate: Controls.ItemDelegate {
            required property var modelData
            width: ListView.view.width
            implicitHeight: 48
            text: modelData.label
            icon.name: modelData.icon
            Accessible.name: modelData.label
            onClicked: modelData.name === "Settings" ? page.app.openSettings() : page.app.go(modelData.name)
        }
    }
}
