// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Reserves: each bank account with its free money, funds and card
// reserves. People see funds called reserves (Main.qml's labels); card
// reserves are the reserves for paying a card. Funds carry forward for
// ever; there are no monthly budgets.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "funds"
    title: app.labels.Funds

    required property var app
    readonly property var ledger: app.ledger
    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    property Fetch fetch: Fetch { ledger: page.ledger; view: "funds"; owner: page }
    readonly property var homes: fetch.value || []
    readonly property string problem: fetch.problem

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.ActionToolBar {
            Layout.fillWidth: true
            alignment: page.app.narrow ? Qt.AlignHCenter : Qt.AlignRight
            actions: [
                Kirigami.Action { text: i18n("Set aside"); icon.name: "list-add"; enabled: page.app.canWrite; onTriggered: page.app.openForm("set_aside") },
                Kirigami.Action { text: i18nc("@action money out of a fund", "Release"); icon.name: "go-up"; enabled: page.app.canWrite; onTriggered: page.app.openForm("set_aside", { mode: "release" }) },
                Kirigami.Action { text: i18nc("@action money between funds", "Move"); icon.name: "exchange-positions"; enabled: page.app.canWrite; onTriggered: page.app.openForm("set_aside", { mode: "move" }) }
            ]
        }

        Kirigami.PlaceholderMessage {
            visible: page.homes.length === 0
            Layout.fillWidth: true
            text: page.problem !== "" ? i18n("Can't load reserves") : i18n("No bank accounts with reserves yet")
            explanation: page.problem
        }

        Repeater {
            model: page.homes
            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    RowLayout {
                        Kirigami.Heading { level: 3; text: card.modelData.home.name; Layout.fillWidth: true }
                        Controls.Label { text: i18n("in the bank"); opacity: 0.7 }
                        Money { amount: card.modelData.home.inclusive; font.bold: true }
                    }
                    // The bank balance split into free money, funds and reserves.
                    Item {
                        id: split
                        Layout.fillWidth: true
                        Layout.preferredHeight: Kirigami.Units.smallSpacing * 2
                        readonly property var h: card.modelData.home
                        Rectangle { x: 0; width: split.width * split.h.free_share; height: parent.height; color: page.tokens.splitFree }
                        Rectangle { x: split.width * split.h.free_share; width: split.width * split.h.funds_share; height: parent.height; color: page.tokens.splitFunds }
                        Rectangle { x: split.width * (split.h.free_share + split.h.funds_share); width: split.width * split.h.reserves_share; height: parent.height; color: page.tokens.splitReserves }
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.largeSpacing
                        Repeater {
                            model: [
                                { label: i18nc("free money", "Free"), amount: card.modelData.home.free, color: page.tokens.splitFree },
                                { label: page.app.labels.Funds, amount: card.modelData.home.funds, color: page.tokens.splitFunds },
                                { label: i18n("Card reserves"), amount: card.modelData.home.reserves, color: page.tokens.splitReserves }
                            ]
                            delegate: RowLayout {
                                id: legendDelegate
                                required property var modelData
                                Rectangle {
                                    Layout.preferredWidth: Kirigami.Units.smallSpacing * 3
                                    Layout.preferredHeight: Layout.preferredWidth
                                    radius: Kirigami.Units.cornerRadius / 2
                                    color: legendDelegate.modelData.color
                                }
                                Controls.Label { text: i18nc("%1 is a label, %2 an amount", "%1 %2", legendDelegate.modelData.label, legendDelegate.modelData.amount.text) }
                            }
                        }
                    }
                    Kirigami.ListSectionHeader { text: page.app.labels.Funds; Layout.fillWidth: true; visible: card.modelData.funds.length > 0 }
                    Repeater {
                        model: card.modelData.funds
                        delegate: Controls.ItemDelegate {
                            id: fundDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: modelData.name
                            onClicked: page.app.showRegister(modelData.account, modelData.name)
                            contentItem: RowLayout {
                                Kirigami.Icon { source: "folder-text"; Layout.preferredWidth: Kirigami.Units.iconSizes.small; Layout.preferredHeight: Layout.preferredWidth }
                                Controls.Label { text: fundDelegate.modelData.name; Layout.fillWidth: true; elide: Text.ElideRight }
                                Money { amount: fundDelegate.modelData.amount }
                                Controls.ToolButton {
                                    icon.name: "list-add"
                                    text: i18n("Set aside")
                                    display: page.app.narrow ? Controls.AbstractButton.IconOnly : Controls.AbstractButton.TextBesideIcon
                                    enabled: page.app.canWrite
                                    Accessible.name: i18nc("%1 is a fund", "Set aside for %1", fundDelegate.modelData.name)
                                    onClicked: page.app.openForm("set_aside", { from: card.modelData.home.account, to: fundDelegate.modelData.account })
                                }
                            }
                        }
                    }
                    Kirigami.ListSectionHeader { text: i18n("Card reserves"); Layout.fillWidth: true; visible: card.modelData.reserves.length > 0 }
                    Repeater {
                        model: card.modelData.reserves
                        delegate: Controls.ItemDelegate {
                            id: reserveDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: i18nc("%1 is a card", "%1 reserve", modelData.name)
                            onClicked: page.app.go("Cards", { focusCard: modelData.card })
                            contentItem: RowLayout {
                                Kirigami.Icon { source: "view-credit-card"; Layout.preferredWidth: Kirigami.Units.iconSizes.small; Layout.preferredHeight: Layout.preferredWidth }
                                ColumnLayout {
                                    spacing: 0
                                    Layout.fillWidth: true
                                    Controls.Label { text: i18nc("%1 is a card", "%1 reserve", reserveDelegate.modelData.name); elide: Text.ElideRight; Layout.fillWidth: true }
                                    Controls.Label {
                                        text: reserveDelegate.modelData.covered ? i18n("Covers the card") : i18nc("%1 is an amount", "%1 still to reserve", reserveDelegate.modelData.short.text)
                                        color: reserveDelegate.modelData.covered ? Kirigami.Theme.disabledTextColor : page.tokens.warningText
                                        font: Kirigami.Theme.smallFont
                                    }
                                }
                                Money { amount: reserveDelegate.modelData.balance }
                            }
                        }
                    }
                }
            }
        }
    }
}
