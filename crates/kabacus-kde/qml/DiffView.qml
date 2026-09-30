// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Journal lines as a diff ({text, kind: add | del | context}): the
// preview's journal and a commit's change.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Rectangle {
    id: view

    property var lines: []

    implicitHeight: column.implicitHeight + Kirigami.Units.largeSpacing
    color: Kirigami.Theme.alternateBackgroundColor
    radius: Kirigami.Units.cornerRadius
    clip: true

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: Kirigami.Units.smallSpacing
        spacing: 0
        Repeater {
            model: view.lines
            delegate: Controls.Label {
                required property var modelData
                text: modelData.text
                font.family: Kirigami.Theme.fixedWidthFont.family
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                color: modelData.kind === "add" ? Kirigami.Theme.positiveTextColor
                    : modelData.kind === "del" ? Kirigami.Theme.negativeTextColor
                    : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
        }
    }
}
