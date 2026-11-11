// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// An account's register, newest first. Balances are
// colour coded; unreserved card transactions use rowUnreserved (design/tokens.json). Clicking a row edits it
// in place.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "register"
    // Empty `account`: every real account's transactions together.
    readonly property bool all: account === ""
    title: all ? i18n("All transactions") : accountName !== "" ? accountName : account

    property string account: ""
    property string accountName: ""
    property bool unreservedOnly: false
    property bool fundTransfers: false
    readonly property var rows: fetch.value || []
    readonly property string problem: fetch.problem

    required property var app
    readonly property var ledger: app.ledger
    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    readonly property bool oneLine: width > Kirigami.Units.gridUnit * 34
    readonly property var chipLabels: ({
        unreserved: i18nc("a card transaction without its reserve lines", "unreserved"),
        move: i18nc("money moved between an account and its funds", "within account")
    })
    // One line: the chips' column is as wide as the longest of them in
    // the user's language (translations run longer than English), at
    // least 7 grid units.
    readonly property real chipColumnWidth: Math.max(Kirigami.Units.gridUnit * 7,
        unreservedSizer.implicitWidth, moveSizer.implicitWidth)

    // The search as last settled (typing waits 300 ms).
    property bool searchOpen: false
    property string query: ""
    property Fetch fetch: Fetch {
        ledger: page.ledger
        view: "register"
        args: ({ account: page.account, text: page.query, unreservedOnly: page.unreservedOnly, fundTransfers: page.fundTransfers })
        owner: page
    }

    // Only accounts a statement comes for can be reconciled (the server
    // refuses the rest): banks, cards, loans and investments.
    property Fetch kinds: Fetch { ledger: page.ledger; view: "picker" }
    readonly property bool reconcilable: (kinds.value ? kinds.value.accounts : [])
        .some(a => a.account === page.account && !a.account.startsWith("equity")
            && ["money", "revolving-debt", "term-liability", "non-liquid"].includes(a.kind))

    // Add and Reconcile stay on a phone's narrow toolbar; Refresh can go
    // to its overflow (pulling the list down refreshes too).
    actions: [
        Kirigami.Action { text: i18n("Add transaction"); icon.name: "list-add"; displayHint: page.app.narrow ? Kirigami.DisplayHint.AlwaysHide : Kirigami.DisplayHint.KeepVisible; enabled: page.app.canWrite; shortcut: StandardKey.New; onTriggered: page.app.openChooser(page.account) },
        Kirigami.Action {
            text: i18n("Account type")
            icon.name: "document-properties"
            displayHint: page.app.narrow ? Kirigami.DisplayHint.AlwaysHide : Kirigami.DisplayHint.KeepVisible
            enabled: page.app.canWrite && page.kinds.value !== null
            onTriggered: {
                const account = page.kinds.value.accounts.find(a => a.account === page.account)
                if (account) page.app.openForm("account_type", { account: account.account, synthetic: account.synthetic })
            }
        },
        Kirigami.Action {
            text: i18nc("@action compare an account with its statement", "Reconcile…")
            icon.name: "dialog-ok-apply"
            displayHint: page.app.narrow ? Kirigami.DisplayHint.AlwaysHide : Kirigami.DisplayHint.KeepVisible
            visible: page.reconcilable
            onTriggered: page.app.openReconcile(page.account, page.title)
        },
        Kirigami.Action { text: i18n("Refresh"); icon.name: "view-refresh"; shortcut: StandardKey.Refresh; onTriggered: page.ledger.refresh() }
    ]

    header: Controls.ToolBar {
        visible: !page.app.narrow || page.searchOpen
        height: visible ? implicitHeight : 0
        RowLayout {
            anchors.fill: parent
            Kirigami.SearchField {
                id: search
                Accessible.name: i18n("Search")
                placeholderText: i18n("Search payee, memo, account")
                Layout.fillWidth: true
                onAccepted: page.query = text
                onTextChanged: searchTimer.restart()
            }
            // Not shown: they measure the chips for chipColumnWidth.
            Kirigami.Chip { id: unreservedSizer; visible: false; closable: false; checkable: false; text: page.chipLabels.unreserved }
            Kirigami.Chip { id: moveSizer; visible: false; closable: false; checkable: false; text: page.chipLabels.move }
            Controls.Switch {
                visible: !page.app.narrow
                text: i18n("Fund transfers")
                checked: page.fundTransfers
                onToggled: page.fundTransfers = checked
            }
            Controls.Switch {
                visible: !page.app.narrow
                text: i18n("Unreserved only")
                checked: page.unreservedOnly
                onToggled: page.unreservedOnly = checked
            }
        }
        Timer {
            id: searchTimer
            interval: 300
            onTriggered: page.query = search.text
        }
    }

    ListView {
        id: register
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.rows
        spacing: page.app.narrow ? 8 : 0
        delegate: ColumnLayout {
            id: dayGroup
            required property var modelData
            required property int index
            width: ListView.view.width
            spacing: page.app.narrow ? 8 : 0
            Controls.Label {
                visible: page.app.narrow && (dayGroup.index === 0 || page.rows[dayGroup.index - 1].date !== dayGroup.modelData.date)
                text: dayGroup.modelData.date
                padding: 8
                font.bold: true
            }
            Controls.ItemDelegate {
            id: transactionDelegate
            readonly property var modelData: dayGroup.modelData
            Layout.fillWidth: true
            implicitHeight: Math.max(page.app.narrow ? 64 : 0, contentItem.implicitHeight + topPadding + bottomPadding)
            Accessible.name: i18nc("%1 is a payee, %2 a date", "%1 %2", modelData.payee, modelData.date)
            background: Rectangle {
                color: transactionDelegate.modelData.unreserved ? page.tokens.rowUnreserved : "transparent"
                border.width: (page.app.agent.changed || []).indexOf(Number(transactionDelegate.modelData.id)) >= 0 ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                Controls.ToolTip.visible: transactionDelegate.hovered && border.width > 0
                Controls.ToolTip.text: page.app.agent.changed_by || ""
                Rectangle {
                    anchors.fill: parent
                    visible: transactionDelegate.down || transactionDelegate.hovered
                    color: Kirigami.Theme.hoverColor
                    opacity: 0.2
                }
            }
            highlighted: page.app.selectedRow === String(modelData.id)
            onClicked: if (modelData.id !== "" && page.app.canWrite) { page.app.selectedRow = String(modelData.id); page.app.openEdit(modelData.id) }
            Keys.onReturnPressed: clicked()
            Keys.onEnterPressed: clicked()
            contentItem: ColumnLayout {
                spacing: 0
                // Wide: one line. Narrow: payee and amount over date,
                // account and balance.
                RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    Controls.Label {
                        visible: page.app.narrow
                        text: ["*", "cleared"].includes(transactionDelegate.modelData.status) ? "●" : "○"
                        color: ["*", "cleared"].includes(transactionDelegate.modelData.status) ? page.tokens.balancePositive : Kirigami.Theme.textColor
                    }
                    Controls.Label {
                        visible: page.oneLine
                        text: transactionDelegate.modelData.date
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 5
                    }
                    Controls.Label {
                        text: transactionDelegate.modelData.payee
                        font.bold: true
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                    }
                    Controls.Label {
                        visible: page.oneLine
                        text: transactionDelegate.modelData.memo !== ""
                            ? i18nc("%1 is an account, %2 a memo", "%1 · %2", transactionDelegate.modelData.account, transactionDelegate.modelData.memo)
                            : transactionDelegate.modelData.account
                        opacity: 0.7
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                    }
                    // One line: a fixed column, empty or not, so every row
                    // lines up and has the same height.
                    Item {
                        readonly property bool hasChip: transactionDelegate.modelData.kind !== "" || transactionDelegate.modelData.unreserved
                        visible: page.oneLine || hasChip
                        implicitWidth: page.oneLine ? page.chipColumnWidth : chip.implicitWidth
                        implicitHeight: chip.implicitHeight
                        Kirigami.Chip {
                            id: chip
                            visible: parent.hasChip
                            anchors.verticalCenter: parent.verticalCenter
                            closable: false
                            checkable: false
                            text: transactionDelegate.modelData.unreserved ? page.chipLabels.unreserved
                                : page.chipLabels[transactionDelegate.modelData.kind] ?? ""
                            // A tap on the chip is a tap on its row.
                            onClicked: transactionDelegate.clicked()
                        }
                    }
                    Money {
                        amount: transactionDelegate.modelData.amount
                        colored: false
                        text: transactionDelegate.modelData.kind === "move" ? i18nc("%1 is an amount", "%1 moved", transactionDelegate.modelData.amount.text.replace("−", "")) : transactionDelegate.modelData.amount.text
                        opacity: transactionDelegate.modelData.kind === "move" ? 0.7 : 1
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    }
                    // A running balance belongs to one account.
                    Money {
                        visible: page.oneLine && !page.all
                        amount: transactionDelegate.modelData.balance
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    }
                }
                RowLayout {
                    visible: !page.oneLine
                    Controls.Label {
                        text: page.app.narrow ? transactionDelegate.modelData.memo || transactionDelegate.modelData.account : i18nc("%1 is a date, %2 an account", "%1 · %2", transactionDelegate.modelData.date, transactionDelegate.modelData.account)
                        opacity: 0.7
                        elide: Text.ElideRight
                        font: Kirigami.Theme.smallFont
                        Layout.fillWidth: true
                    }
                    Money {
                        visible: !page.all
                        amount: transactionDelegate.modelData.balance
                        font: Kirigami.Theme.smallFont
                    }
                }
            }
        }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: register.count === 0
            text: page.problem !== "" ? i18n("Can't load this register")
                : page.query !== "" ? i18nc("%1 is what was searched for", "No transactions match “%1”", page.query)
                : page.unreservedOnly ? i18n("No unreserved transactions") : i18n("No transactions")
            explanation: page.problem
            // With a search, clearing it is the way back.
            helpfulAction: page.query !== "" ? clearAction : page.app.canWrite ? addAction : null
            Kirigami.Action {
                id: clearAction
                text: i18n("Clear search")
                icon.name: "edit-clear"
                onTriggered: {
                    search.text = ""
                    page.query = ""
                }
            }
            Kirigami.Action {
                id: addAction
                text: i18n("Add transaction")
                icon.name: "list-add"
                onTriggered: page.app.openChooser(page.account)
            }
        }
    }
}
