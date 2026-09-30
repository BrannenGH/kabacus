// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Transactions: every account grouped by kind, each opening its
// register. Funds and card reserves are under their bank account
// (Reserves), not more rows here.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "accounts"
    title: app.labels.Accounts

    required property var app
    readonly property var ledger: app.ledger
    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    // An account to open right away (from an Overview warning).
    property string open: ""
    property string selected: ""
    property Fetch fetch: Fetch { ledger: page.ledger; view: "accounts"; owner: page }
    readonly property string problem: fetch.problem
    // The groups flattened; each group's first row carries its heading,
    // drawn before it, so screen readers meet the heading first.
    // ListView sections are created after their first item.
    readonly property var rows: {
        const out = []
        for (const g of fetch.value || []) {
            g.rows.forEach((row, i) => {
                row.group = i === 0 ? g.title : ""
                out.push(row)
            })
        }
        return out
    }

    Component.onCompleted: {
        if (open !== "") {
            selected = open
            app.showRegister(open, open)
        }
    }

    actions: [
        Kirigami.Action { text: i18n("Add"); icon.name: "list-add"; enabled: page.app.canWrite; onTriggered: page.app.openChooser() }
    ]

    ListView {
        id: list
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.rows
        delegate: ColumnLayout {
            id: groupDelegate
            required property var modelData
            width: ListView.view.width
            spacing: 0
            Kirigami.ListSectionHeader {
                visible: groupDelegate.modelData.group !== ""
                text: groupDelegate.modelData.group
                Accessible.role: Accessible.Heading
                Accessible.name: text
                Layout.fillWidth: true
            }
        Controls.ItemDelegate {
            id: accountDelegate
            readonly property var modelData: groupDelegate.modelData
            Layout.fillWidth: true
            highlighted: modelData.account === page.selected
            Accessible.name: modelData.name
            onClicked: {
                page.selected = modelData.account
                page.app.showRegister(modelData.account, modelData.name)
            }
            contentItem: RowLayout {
                ColumnLayout {
                    spacing: 0
                    Layout.fillWidth: true
                    Controls.Label {
                        text: accountDelegate.modelData.name
                        color: accountDelegate.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        font.bold: true
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                    Controls.Label {
                        visible: accountDelegate.modelData.detail !== ""
                        text: accountDelegate.modelData.detail
                        // No dimming on the selection colour.
                        color: accountDelegate.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        opacity: accountDelegate.highlighted ? 1 : 0.7
                        font: Kirigami.Theme.smallFont
                        // Translated, so it may run long: wrap, don't cut.
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                }
                Kirigami.Icon {
                    visible: accountDelegate.modelData.warn
                    source: "data-warning"
                    color: accountDelegate.highlighted ? Kirigami.Theme.highlightedTextColor : page.tokens.warningText
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Layout.preferredWidth
                }
                Money { amount: accountDelegate.modelData.balance; highlighted: accountDelegate.highlighted }
            }
        }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            text: page.app.needsSignIn ? i18n("Not signed in") : page.problem !== "" ? i18n("Can't reach kabacus-server") : i18n("No accounts")
            explanation: page.problem
        }
    }
}
