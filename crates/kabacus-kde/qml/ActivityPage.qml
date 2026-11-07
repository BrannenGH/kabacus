// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Every change to the books and who made it (this app,
// an MCP client, an external edit). A commit opens beside the list.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "activity"
    title: i18n("Activity")

    required property var app
    readonly property var ledger: app.ledger
    property string actor: ""
    property string selected: ""
    property Fetch fetch: Fetch {
        ledger: page.ledger
        view: "activity"
        args: ({ actor: page.actor })
        owner: page
    }
    readonly property var commits: fetch.value || []
    readonly property string problem: fetch.problem

    header: Controls.ToolBar {
        Flow {
            anchors.fill: parent
            spacing: Kirigami.Units.smallSpacing
            Controls.ButtonGroup { id: actorGroup }
            Repeater {
                model: [[i18nc("changes made by", "All"), ""], [i18nc("changes made by", "MCP"), "mcp"], [i18nc("changes made by", "This app"), "app"], [i18nc("changes made by", "External"), "external"]]
                delegate: Kirigami.Chip {
                    required property var modelData
                    text: modelData[0]
                    closable: false
                    checkable: true
                    checked: page.actor === modelData[1]
                    Controls.ButtonGroup.group: actorGroup
                    onClicked: page.actor = modelData[1]
                }
            }
        }
    }

    ListView {
        id: list
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.commits
        delegate: Controls.ItemDelegate {
            id: commitDelegate
            required property var modelData
            width: ListView.view.width
            highlighted: modelData.hash === page.selected
            Accessible.name: modelData.summary
            onClicked: {
                page.selected = modelData.hash
                while (page.app.pageStack.depth > 1) page.app.pageStack.pop()
                page.app.pageStack.push(commitPage, { commit: modelData })
            }
            contentItem: RowLayout {
                Kirigami.Icon {
                    source: commitDelegate.modelData.actor === "mcp" ? "network-connect" : commitDelegate.modelData.actor === "external" ? "document-edit" : "user"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Layout.preferredWidth
                }
                ColumnLayout {
                    spacing: 0
                    Layout.fillWidth: true
                    Controls.Label { text: commitDelegate.modelData.agent ? i18n("Agent") + ": " + commitDelegate.modelData.agent : ""; visible: text !== ""; font: Kirigami.Theme.smallFont }
                    Controls.Label { text: commitDelegate.modelData.summary; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                    Controls.Label { text: i18nc("%1 is who made a change, %2 its revision", "%1 · %2", commitDelegate.modelData.user || commitDelegate.modelData.actor_label, commitDelegate.modelData.short); opacity: 0.7; font: Kirigami.Theme.smallFont }
                }
                Controls.Label { text: commitDelegate.modelData.date; opacity: 0.7; font: Kirigami.Theme.smallFont }
            }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            text: page.problem !== "" ? i18n("Can't load the history") : i18n("No changes")
            explanation: page.problem
        }
    }

    Component {
        id: commitPage
        CommitPage { app: page.app }
    }
}
