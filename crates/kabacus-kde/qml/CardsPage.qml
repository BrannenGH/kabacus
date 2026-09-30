// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Each card's three numbers (owes, reserve, still
// to reserve), what the reserve holds, and the transactions missing their
// reserve lines, fixed together in one reviewed commit.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "cards"
    title: app.labels.Cards

    required property var app
    readonly property var ledger: app.ledger
    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    // The card to bring forward (from an Overview warning).
    property string focusCard: ""
    property Fetch fetch: Fetch { ledger: page.ledger; view: "cards"; owner: page }
    // The focused card first.
    readonly property var cards: (fetch.value || []).slice()
        .sort((a, b) => (b.card.card === focusCard) - (a.card.card === focusCard))
    readonly property string problem: fetch.problem

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.PlaceholderMessage {
            visible: page.cards.length === 0
            Layout.fillWidth: true
            text: page.problem !== "" ? i18n("Can't load cards") : i18n("No credit cards")
            explanation: page.problem
        }

        Repeater {
            model: page.cards
            delegate: Kirigami.AbstractCard {
                id: cardItem
                required property var modelData
                readonly property var c: modelData.card
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.largeSpacing
                    RowLayout {
                        Kirigami.Heading { level: 2; text: cardItem.c.name; Layout.fillWidth: true }
                        Kirigami.ActionToolBar {
                            alignment: Qt.AlignRight
                            Layout.fillWidth: true
                            actions: [
                                Kirigami.Action {
                                    text: i18n("Record purchase"); icon.name: "list-add"; enabled: page.app.canWrite
                                    onTriggered: page.app.openForm("card_purchase", { card: cardItem.c.card })
                                },
                                Kirigami.Action {
                                    text: i18n("Pay card"); icon.name: "view-credit-card"; enabled: page.app.canWrite
                                    onTriggered: page.app.openForm("pay_card", { card: cardItem.c.card, amount: cardItem.c.owed.exact })
                                }
                            ]
                        }
                    }
                    GridLayout {
                        columns: page.app.narrow ? 1 : 3
                        columnSpacing: Kirigami.Units.gridUnit * 2
                        Layout.fillWidth: true
                        Repeater {
                            model: [
                                { label: i18nc("what a card owes", "Owes"), amount: cardItem.c.owed, warn: false },
                                { label: i18nc("a card's reserve", "Reserve"), amount: cardItem.c.reserve, warn: false },
                                { label: i18n("Still to reserve"), amount: cardItem.c.short, warn: !cardItem.c.covered }
                            ]
                            delegate: ColumnLayout {
                                id: lineDelegate
                                required property var modelData
                                Layout.fillWidth: true
                                Accessible.name: i18nc("%1 is a label, %2 an amount", "%1 %2", modelData.label, modelData.amount.text)
                                Controls.Label { text: lineDelegate.modelData.label; opacity: 0.7 }
                                Money {
                                    amount: lineDelegate.modelData.amount
                                    colored: false
                                    color: lineDelegate.modelData.warn ? page.tokens.warningText : Kirigami.Theme.textColor
                                    horizontalAlignment: Text.AlignLeft
                                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.8
                                    font.weight: Font.Light
                                }
                            }
                        }
                    }
                    Kirigami.InlineMessage {
                        Layout.fillWidth: true
                        visible: cardItem.modelData.unreserved.length > 0
                        type: Kirigami.MessageType.Warning
                        text: i18np("%1 transaction moves the card without moving its reserve.", "%1 transactions move the card without moving its reserve.", cardItem.modelData.unreserved.length)
                        actions: [
                            Kirigami.Action {
                                text: i18n("Add all reserve lines")
                                enabled: page.app.canWrite
                                onTriggered: page.app.openForm("fix_reserves", {
                                    card: cardItem.c.card,
                                    ids: cardItem.modelData.unreserved.map(u => u.id)
                                })
                            }
                        ]
                    }
                    Repeater {
                        model: cardItem.modelData.unreserved
                        delegate: RowLayout {
                            id: unreservedDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: i18nc("%1 is a transaction", "%1 unreserved", modelData.description)
                            Controls.Label { text: unreservedDelegate.modelData.date; opacity: 0.7 }
                            ColumnLayout {
                                spacing: 0
                                Layout.fillWidth: true
                                Controls.Label { text: unreservedDelegate.modelData.description; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                                Controls.Label { text: i18nc("%1 is an amount", "Missing reserve %1", unreservedDelegate.modelData.missing.text); opacity: 0.7; font: Kirigami.Theme.smallFont }
                            }
                            Controls.Button {
                                text: i18n("Add reserve lines")
                                icon.name: "list-add"
                                enabled: page.app.canWrite
                                onClicked: page.app.openForm("fix_reserves", { card: cardItem.c.card, ids: [unreservedDelegate.modelData.id] })
                            }
                        }
                    }
                    Controls.Label { text: i18n("What the reserve holds"); font.bold: true; visible: cardItem.modelData.sources.length > 0 }
                    Repeater {
                        model: cardItem.modelData.sources
                        delegate: ColumnLayout {
                            id: sourceDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: 2
                            RowLayout {
                                Controls.Label { text: sourceDelegate.modelData.name; Layout.fillWidth: true }
                                Money { amount: sourceDelegate.modelData.amount; colored: false }
                            }
                            Item {
                                Layout.fillWidth: true
                                Layout.preferredHeight: Kirigami.Units.smallSpacing
                                Rectangle {
                                    width: Math.max(2, parent.width * sourceDelegate.modelData.share)
                                    height: parent.height
                                    color: page.tokens.splitFree
                                    radius: height / 2
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
