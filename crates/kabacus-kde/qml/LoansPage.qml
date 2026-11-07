// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Loans: each loan (a term liability, such as a car loan or a mortgage)
// as Transactions lists it, with what's still owed. A loan opens its
// register for now; mortgages and installment loans will get pages of
// their own.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "loans"
    title: app.labels.Loans

    required property var app
    readonly property var ledger: app.ledger
    property Fetch fetch: Fetch { ledger: page.ledger; view: "loans"; owner: page }
    readonly property string problem: fetch.problem

    ListView {
        id: list
        // Room for the floating Add button on a phone.
        bottomMargin: page.app.narrow ? Kirigami.Units.gridUnit * 4 : 0
        model: page.fetch.value || []
        delegate: Controls.ItemDelegate { implicitHeight: page.app.narrow ? Math.max(56, contentItem.implicitHeight + topPadding + bottomPadding) : contentItem.implicitHeight + topPadding + bottomPadding;
            id: loanDelegate
            required property var modelData
            width: ListView.view.width
            Accessible.name: modelData.name
            onClicked: page.app.showRegister(modelData.account, modelData.name)
            contentItem: RowLayout {
                Kirigami.Icon {
                    source: "view-bank"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Layout.preferredWidth
                }
                Controls.Label {
                    text: loanDelegate.modelData.name
                    font.bold: true
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Money { amount: loanDelegate.modelData.balance }
            }
        }
        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            text: page.app.needsSignIn ? i18n("Not signed in") : page.problem !== "" ? i18n("Can't reach kabacus-server") : i18n("No loans")
            explanation: page.problem !== "" ? page.problem
                : i18n("A loan or mortgage shows here once it's in the books.")
        }
    }
}
