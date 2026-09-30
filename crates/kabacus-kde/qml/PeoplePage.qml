// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// One net line per person; their statement opens beside it.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as KirigamiComponents
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "people"
    title: i18n("People")

    required property var app
    readonly property var ledger: app.ledger
    property string selected: ""
    property Fetch fetch: Fetch { ledger: page.ledger; view: "people"; owner: page }
    readonly property var people: fetch.value || []
    readonly property string problem: fetch.problem

    actions: [
        Kirigami.Action { text: i18nc("@action split a cost with someone", "Split"); icon.name: "split"; enabled: page.app.canWrite; onTriggered: page.app.openForm("split") }
    ]

    ListView {
        id: list
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.people
        delegate: Controls.ItemDelegate {
            id: personDelegate
            required property var modelData
            width: ListView.view.width
            highlighted: modelData.name === page.selected
            Accessible.name: modelData.display
            onClicked: {
                page.selected = modelData.name
                while (page.app.pageStack.depth > 1) page.app.pageStack.pop()
                page.app.pageStack.push(statementPage, { person: modelData.name, displayName: modelData.display })
            }
            contentItem: RowLayout {
                KirigamiComponents.Avatar {
                    name: personDelegate.modelData.display
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Layout.preferredWidth
                }
                ColumnLayout {
                    spacing: 0
                    Layout.fillWidth: true
                    Controls.Label { text: personDelegate.modelData.display; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                    Controls.Label { text: personDelegate.modelData.label; opacity: 0.7; font: Kirigami.Theme.smallFont }
                }
                Money { amount: personDelegate.modelData.net }
            }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            text: page.problem !== "" ? i18n("Can't load people") : i18n("Nobody owes or is owed anything")
            explanation: page.problem
        }
    }

    Component {
        id: statementPage
        StatementPage { app: page.app }
    }
}
