// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// A person's netted statement with its running balance.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "statement"
    title: displayName !== "" ? displayName : person

    property string person: ""
    property string displayName: ""
    property bool openItems: false

    required property var app
    readonly property var ledger: app.ledger

    property Fetch fetch: Fetch {
        ledger: page.ledger
        view: "statement"
        args: ({ person: page.person, openItems: page.openItems })
        owner: page
    }
    readonly property var rows: fetch.value || []
    readonly property string problem: fetch.problem

    header: Controls.ToolBar {
        RowLayout {
            anchors.fill: parent
            Item { Layout.fillWidth: true }
            Controls.Switch {
                text: i18n("Open items only")
                checked: page.openItems
                onToggled: page.openItems = checked
            }
        }
    }

    ListView {
        id: list
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.rows
        delegate: Controls.ItemDelegate {
            id: lineDelegate
            required property var modelData
            width: ListView.view.width
            Accessible.name: i18nc("%1 is a transaction, %2 its date", "%1 %2", modelData.description, modelData.date)
            contentItem: RowLayout {
                Controls.Label { text: lineDelegate.modelData.date; opacity: 0.7; Layout.preferredWidth: Kirigami.Units.gridUnit * 5 }
                Controls.Label { text: lineDelegate.modelData.description; elide: Text.ElideRight; Layout.fillWidth: true }
                Money { amount: lineDelegate.modelData.amount; colored: false }
                Money { amount: lineDelegate.modelData.running; Layout.preferredWidth: Kirigami.Units.gridUnit * 5 }
            }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            text: page.problem !== "" ? i18n("Can't load the statement") : i18n("Nothing this year")
            explanation: page.problem
        }
    }
}
