// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Compare one account with its statement, opened from the account's
// register. Checking writes nothing; likely explanations open as ordinary
// edits; once it matches, the reconciled balance check is written as one
// reviewed commit.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "reconcile"
    title: i18nc("%1 is an account", "Reconcile %1", accountName)

    globalToolBarStyle: app.narrow ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    required property var app
    // Set when it's opened (Main.qml's openReconcile).
    property string account: ""
    property string accountName: ""
    readonly property var ledger: app.ledger
    property Fetch pickerFetch: Fetch { ledger: page.ledger; view: "picker" }
    readonly property int places: pickerFetch.value ? pickerFetch.value.places : 2

    // The statement last checked; checking writes nothing.
    property var asked: null
    property Fetch checkFetch: Fetch {
        ledger: page.ledger
        view: page.asked ? "reconcile" : ""
        args: page.asked || ({})
        owner: page
    }
    readonly property var result: page.asked && !checkFetch.pending && checkFetch.problem === "" ? checkFetch.value : null
    readonly property string problem: checkFetch.problem

    function check() {
        asked = { account: page.account, date: date.text, balance: balance.text }
    }

    Component.onCompleted: date.text = ledger.today()

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.FormLayout {
            Layout.fillWidth: true
            Controls.TextField {
                id: date
                Kirigami.FormData.label: i18nc("@label", "Statement date:")
                Accessible.name: i18n("Statement date")
                placeholderText: i18nc("the date format to type; dates are always typed this way", "YYYY-MM-DD")
            }
            Controls.TextField {
                id: balance
                Kirigami.FormData.label: i18nc("@label", "Statement balance:")
                Accessible.name: i18n("Statement balance")
                inputMethodHints: Qt.ImhFormattedNumbersOnly
                onAccepted: if (balanceProblem.text === "") page.check()
            }
            // The same check and words as the Add forms.
            Controls.Label {
                id: balanceProblem
                visible: text !== ""
                text: balance.text === "" ? "" : page.ledger.checkAmount(balance.text, page.places, false)
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.Wrap
                Layout.fillWidth: true
                Accessible.name: i18nc("%1 is the field, %2 what's wrong with it", "%1 problem: %2", i18n("Statement balance"), text)
            }
            Controls.Button {
                text: i18nc("@action:button compare with the statement", "Check")
                icon.name: "view-refresh"
                enabled: balanceProblem.text === ""
                onClicked: page.check()
            }
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.problem !== ""
            type: Kirigami.MessageType.Error
            text: page.problem
        }

        Kirigami.InlineMessage {
            objectName: "reconcileResult"
            Layout.fillWidth: true
            visible: page.result !== null
            type: page.result && page.result.matched ? Kirigami.MessageType.Positive : Kirigami.MessageType.Warning
            text: page.result === null ? ""
                : page.result.matched ? i18nc("%1 is a balance, %2 a date", "Matched: %1 on %2, the same as the statement.", page.result.ledger.text, page.result.date)
                : i18nc("%1 is the books' balance, %2 the statement's, %3 the difference",
                    "Doesn't match yet: the books say %1, the statement %2 (difference %3). Nothing has been written.",
                    page.result.ledger.text, page.result.statement.text, page.result.difference.text)
            actions: page.result && page.result.matched && page.app.canWrite ? [reconcileAction] : []
            Kirigami.Action {
                id: reconcileAction
                text: i18n("Reconcile")
                icon.name: "dialog-ok-apply"
                onTriggered: page.app.openForm("reconcile", { account: page.result.account, date: page.result.date, balance: page.result.statement.exact })
            }
        }

        Kirigami.Heading {
            level: 4
            text: i18n("Likely explanations")
            visible: page.result !== null && !page.result.matched && (page.result.suggestions.length + page.result.later.length) > 0
        }
        Repeater {
            model: page.result && !page.result.matched ? page.result.suggestions.concat(page.result.later) : []
            delegate: Kirigami.AbstractCard {
                id: suggestionDelegate
                required property var modelData
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    Repeater {
                        model: suggestionDelegate.modelData
                        delegate: RowLayout {
                            id: postingDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Controls.Label { text: postingDelegate.modelData.date; opacity: 0.7 }
                            Controls.Label { text: postingDelegate.modelData.description; Layout.fillWidth: true; elide: Text.ElideRight }
                            Money { amount: postingDelegate.modelData.amount; colored: false }
                            Controls.Button {
                                text: i18n("Edit")
                                icon.name: "document-edit"
                                visible: postingDelegate.modelData.id !== "" && page.app.canWrite
                                onClicked: page.app.openEdit(postingDelegate.modelData.id)
                            }
                        }
                    }
                }
            }
        }
    }
}
