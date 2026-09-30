// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// A full-width message above every page: sign-in, authentication off,
// offline. Kirigami.Action buttons on the right.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Rectangle {
    id: banner

    property string text: ""
    // "info", "warning" or "error".
    property string kind: "info"
    property list<QtObject> actions

    readonly property color tint: kind === "error" ? Kirigami.Theme.negativeTextColor
        : kind === "warning" ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.highlightColor

    Layout.fillWidth: true
    implicitHeight: row.implicitHeight + Kirigami.Units.largeSpacing
    color: Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(tint.r, tint.g, tint.b, 0.18))

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: banner.tint
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        Kirigami.Icon {
            source: banner.kind === "error" ? "dialog-error" : banner.kind === "warning" ? "dialog-warning" : "dialog-information"
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Layout.preferredWidth
        }
        Controls.Label {
            text: banner.text
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Repeater {
            model: banner.actions
            delegate: Controls.Button {
                required property var modelData
                action: modelData
                Accessible.name: modelData.text
            }
        }
    }
}
