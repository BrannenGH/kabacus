// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// One Add, named after what happened. Each choice is one write tool:
// add_transaction, record_card_purchase, allocate or pay_card.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.Dialog {
    id: chooser
    title: i18n("Add")
    standardButtons: Kirigami.Dialog.NoButton
    preferredWidth: Kirigami.Units.gridUnit * 26
    padding: Kirigami.Units.largeSpacing

    required property var app
    // The register it was opened from, if any: preset in the form.
    property string account: ""
    property Fetch pickerFetch: Fetch { ledger: chooser.app.ledger; view: "picker" }
    readonly property bool onCard: account !== "" && pickerFetch.value !== null
        && pickerFetch.value.accounts.some(a => a.account === chooser.account && a.kind === "revolving-debt")

    function preset(kind) {
        if (account === "") return {}
        if (kind === "received") return { to: account }
        if (kind === "card_purchase" || kind === "pay_card") return onCard ? { card: account } : {}
        return { from: account }
    }

    // On a card's register, Card purchase comes first.
    readonly property var kinds: onCard ? allKinds.filter(k => k.kind === "card_purchase").concat(allKinds.filter(k => k.kind !== "card_purchase")) : allKinds
    readonly property var allKinds: [
        { kind: "spent", title: i18nc("@action what happened", "Spent"), detail: i18n("Money left an account"), icon: "go-up" },
        { kind: "received", title: i18nc("@action what happened", "Received"), detail: i18n("Income or a refund"), icon: "go-down" },
        { kind: "card_purchase", title: i18n("Card purchase"), detail: i18n("Adds the reserve lines for you"), icon: "view-credit-card" },
        { kind: "transfer", title: i18n("Transfer"), detail: i18n("Between two bank accounts"), icon: "exchange-positions" },
        { kind: "set_aside", title: i18n("Set aside"), detail: i18n("Into a reserve, out, or between reserves"), icon: "folder-text" },
        { kind: "pay_card", title: i18n("Pay a card"), detail: i18n("From its reserve"), icon: "view-credit-card" },
        { kind: "split", title: i18n("Split with someone"), detail: i18n("They owe you, or you owe them"), icon: "system-users" },
        { kind: "postings", title: i18n("Postings"), detail: i18n("Any two accounts"), icon: "view-list-details" }
    ]

    GridLayout {
        columns: chooser.app.narrow ? 1 : 2
        columnSpacing: Kirigami.Units.smallSpacing
        rowSpacing: Kirigami.Units.smallSpacing

        Repeater {
            model: chooser.kinds
            delegate: Controls.ItemDelegate {
                id: choiceDelegate
                required property var modelData
                Layout.fillWidth: true
                Layout.minimumHeight: Kirigami.Units.gridUnit * 3
                Accessible.name: modelData.title
                onClicked: chooser.app.openForm(modelData.kind, chooser.preset(modelData.kind))
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        source: choiceDelegate.modelData.icon
                        Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                        Layout.preferredHeight: Layout.preferredWidth
                    }
                    ColumnLayout {
                        spacing: 0
                        Layout.fillWidth: true
                        Controls.Label { text: choiceDelegate.modelData.title; font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
                        Controls.Label { text: choiceDelegate.modelData.detail; opacity: 0.7; wrapMode: Text.Wrap; font: Kirigami.Theme.smallFont; Layout.fillWidth: true }
                    }
                }
            }
        }
    }
}
