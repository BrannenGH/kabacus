// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    required property var app
    title: i18n("Welcome to Kabacus")
    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - Kirigami.Units.gridUnit * 4, Kirigami.Units.gridUnit * 32)
        spacing: Kirigami.Units.largeSpacing
        Kirigami.Icon { source: "me.hallb.kabacus"; Layout.preferredWidth: 72; Layout.preferredHeight: 72; Layout.alignment: Qt.AlignHCenter }
        Kirigami.Heading { text: i18n("Welcome to Kabacus"); level: 1; Layout.alignment: Qt.AlignHCenter }
        Controls.Label { text: i18n("Your books, on this computer. Every change is saved."); wrapMode: Text.Wrap; Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter }
        Controls.Button { text: i18n("New Books…"); icon.name: "document-new"; Layout.fillWidth: true; onClicked: page.app.newBooks() }
        Controls.Button { text: i18n("Open Books…"); icon.name: "document-open"; Layout.fillWidth: true; onClicked: page.app.openBooks() }
        Controls.Button { text: i18n("Connect to a Server…"); icon.name: "network-connect"; flat: true; Layout.fillWidth: true; onClicked: page.app.connectServer() }
        Kirigami.Heading { text: i18n("Open Recent"); level: 3; visible: page.app.recentBooks.length > 0 }
        Repeater {
            model: page.app.recentBooks
            delegate: RowLayout {
                id: recentDelegate
                required property var modelData
                Layout.fillWidth: true
                Controls.Button { text: recentDelegate.modelData.name; flat: true; Layout.fillWidth: true; onClicked: page.app.openSource(recentDelegate.modelData) }
                Controls.ToolButton { text: i18n("New Window"); icon.name: "window-new"; display: Controls.AbstractButton.IconOnly; onClicked: page.app.newWindow(recentDelegate.modelData) }
            }
        }
    }
}
