// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Reports that answer questions. Why Cash on Hand changed this
// month, income and expenses with loan principal, and
// where the money went.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "reports"
    title: i18n("Reports")

    required property var app
    readonly property var ledger: app.ledger
    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    property int year: new Date().getFullYear()
    property int month: new Date().getMonth() + 1
    property int tab: 0
    property bool principal: true

    // The year as text: KI18n formats a number argument with the
    // locale's digit grouping (2,026).
    readonly property string monthName: i18nc("%1 is a month's name, %2 its year", "%1 %2", Qt.locale().standaloneMonthName(month - 1), String(year))
    // The last columns are as wide as their headings in the user's
    // language (translations run longer than English), at least 5 grid
    // units, and the rows below line up with them.
    readonly property real nonCashWidth: Math.max(Kirigami.Units.gridUnit * 5, nonCashHeader.implicitWidth)
    readonly property real changeWidth: Math.max(Kirigami.Units.gridUnit * 5, changeHeader.implicitWidth)
    // The comparison column's heading.
    readonly property string previousMonth: Qt.locale().standaloneMonthName((month + 10) % 12, Locale.ShortFormat)

    // One report per tab, loaded when its tab is open.
    property Fetch flowFetch: Fetch {
        ledger: page.ledger
        view: page.tab === 0 ? "report" : ""
        args: ({ kind: "cash_flow", year: page.year, month: page.month })
        owner: page.tab === 0 ? page : null
    }
    property Fetch incomeFetch: Fetch {
        ledger: page.ledger
        view: page.tab === 1 ? "report" : ""
        args: ({ kind: "income", year: page.year, month: page.month, principal: page.principal })
        owner: page.tab === 1 ? page : null
    }
    property Fetch categoriesFetch: Fetch {
        ledger: page.ledger
        view: page.tab === 2 ? "report" : ""
        args: ({ kind: "categories", year: page.year, month: page.month })
        owner: page.tab === 2 ? page : null
    }
    readonly property var flow: flowFetch.value
    readonly property var income: incomeFetch.value
    readonly property var categories: categoriesFetch.value || []
    readonly property string problem: [flowFetch, incomeFetch, categoriesFetch][tab].problem

    function step(by) {
        let m = month + by
        let y = year
        if (m < 1) { m = 12; y -= 1 }
        if (m > 12) { m = 1; y += 1 }
        month = m
        year = y
    }

    header: ColumnLayout {
        spacing: 0
        Controls.ToolBar {
            Layout.fillWidth: true
            RowLayout {
                anchors.fill: parent
                // Arrows point the way the language reads: mirrored, the
                // previous month is to the right.
                Controls.ToolButton { Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; Layout.minimumWidth: page.app.narrow ? 48 : implicitWidth; icon.name: (page.LayoutMirroring.enabled ? "go-next" : "go-previous"); text: i18n("Previous month"); display: Controls.AbstractButton.IconOnly; onClicked: page.step(-1) }
                Controls.Label { text: page.monthName; font.bold: true; horizontalAlignment: Text.AlignHCenter; Layout.fillWidth: true }
                Controls.ToolButton { Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; Layout.minimumWidth: page.app.narrow ? 48 : implicitWidth; icon.name: (page.LayoutMirroring.enabled ? "go-previous" : "go-next"); text: i18n("Next month"); display: Controls.AbstractButton.IconOnly; onClicked: page.step(1) }
            }
        }
        Controls.TabBar {
            Layout.fillWidth: true
            currentIndex: page.tab
            onCurrentIndexChanged: page.tab = currentIndex
            Controls.TabButton { text: i18n("Cash flow") }
            Controls.TabButton { text: i18n("Income statement") }
            Controls.TabButton { text: i18n("Categories") }
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.problem !== ""
            type: Kirigami.MessageType.Error
            text: page.problem
        }

        // The bridge from opening to closing Cash on Hand.
        ColumnLayout {
            visible: page.tab === 0 && page.flow !== null
            Layout.fillWidth: true
            Controls.Label { text: i18nc("%1 is a month and year", "Why Cash on Hand changed in %1", page.monthName); font.bold: true }
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: Kirigami.Units.gridUnit * 12
                spacing: Kirigami.Units.smallSpacing
                Repeater {
                    model: page.flow ? page.flow.bars : []
                    delegate: ColumnLayout {
                        id: barDelegate
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Accessible.name: i18nc("%1 is a label, %2 an amount", "%1 %2", modelData.name, modelData.amount.text)
                        Item {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Rectangle {
                                anchors.horizontalCenter: parent.horizontalCenter
                                width: parent.width * 0.6
                                y: parent.height * (1 - barDelegate.modelData.hi)
                                height: Math.max(3, parent.height * (barDelegate.modelData.hi - barDelegate.modelData.lo))
                                radius: 3
                                color: barDelegate.modelData.kind === "base" ? page.tokens.splitFree
                                    : barDelegate.modelData.kind === "up" ? page.tokens.balancePositive : page.tokens.balanceNegative
                            }
                        }
                        Money { amount: barDelegate.modelData.amount; colored: barDelegate.modelData.kind !== "base"; horizontalAlignment: Text.AlignHCenter; Layout.fillWidth: true; font: Kirigami.Theme.smallFont }
                        Controls.Label { text: barDelegate.modelData.name; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap; opacity: 0.7; Layout.fillWidth: true; font: Kirigami.Theme.smallFont }
                    }
                }
            }
            // Each column says what it is.
            RowLayout {
                Layout.fillWidth: true
                visible: page.flow !== null && page.flow.lines.length > 0
                Item { Layout.fillWidth: true }
                Controls.Label { text: i18nc("column: money moved in cash", "Cash"); font: Kirigami.Theme.smallFont; opacity: 0.7; horizontalAlignment: Text.AlignRight }
                Controls.Label { id: nonCashHeader; text: i18nc("column: changes that aren't cash", "Not cash"); font: Kirigami.Theme.smallFont; opacity: 0.7; horizontalAlignment: Text.AlignRight; Layout.preferredWidth: page.nonCashWidth }
            }
            Repeater {
                model: page.flow ? page.flow.lines : []
                delegate: RowLayout {
                    id: flowLineDelegate
                    required property var modelData
                    Layout.fillWidth: true
                    Controls.Label { text: flowLineDelegate.modelData.name; Layout.fillWidth: true; elide: Text.ElideRight }
                    Money { amount: flowLineDelegate.modelData.cash }
                    Money { amount: flowLineDelegate.modelData.non_cash; colored: false; opacity: 0.7; Layout.preferredWidth: page.nonCashWidth }
                }
            }
        }

        // Income, expenses and loan principal.
        Kirigami.FormLayout {
            visible: page.tab === 1 && page.income !== null
            Layout.fillWidth: true
            Money { Kirigami.FormData.label: i18nc("@label", "Income:"); amount: page.income ? page.income.income : null; colored: false }
            Money { Kirigami.FormData.label: i18nc("@label", "Expenses:"); amount: page.income ? page.income.expenses : null; colored: false }
            Money {
                Kirigami.FormData.label: i18nc("@label", "Loan principal:")
                visible: page.income !== null && page.income.conversion !== null
                amount: page.income ? page.income.conversion : null
                colored: false
            }
            Money { Kirigami.FormData.label: i18nc("@label income less expenses", "Net:"); amount: page.income ? page.income.net : null; font.bold: true }
            Controls.Switch {
                text: i18n("Count loan principal as money spent")
                checked: page.principal
                onToggled: page.principal = checked
            }
        }

        // Where the money went, with the change from the month before.
        RowLayout {
            Layout.fillWidth: true
            visible: page.tab === 2 && page.categories.length > 0
            Item { Layout.fillWidth: true }
            Controls.Label { text: i18nc("column: money spent", "Spent"); font: Kirigami.Theme.smallFont; opacity: 0.7; horizontalAlignment: Text.AlignRight }
            Controls.Label { id: changeHeader; text: i18nc("column: compared with %1, the month before", "vs %1", page.previousMonth); font: Kirigami.Theme.smallFont; opacity: 0.7; horizontalAlignment: Text.AlignRight; Layout.preferredWidth: page.changeWidth }
        }
        Repeater {
            model: page.tab === 2 ? page.categories : []
            delegate: ColumnLayout {
                id: categoryDelegate
                required property var modelData
                Layout.fillWidth: true
                spacing: 2
                Accessible.name: i18nc("%1 is a label, %2 an amount", "%1 %2", modelData.name, modelData.amount.text)
                RowLayout {
                    Controls.Label {
                        text: categoryDelegate.modelData.name
                        color: categoryDelegate.modelData.uncategorized ? page.tokens.warningText : Kirigami.Theme.textColor
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                    Money { amount: categoryDelegate.modelData.amount; colored: false }
                    Money { amount: categoryDelegate.modelData.change; colored: false; opacity: 0.7; Layout.preferredWidth: page.changeWidth }
                }
                Item {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Kirigami.Units.smallSpacing
                    Rectangle {
                        width: Math.max(2, parent.width * categoryDelegate.modelData.share)
                        height: parent.height
                        radius: height / 2
                        color: categoryDelegate.modelData.uncategorized ? page.tokens.splitReserves : page.tokens.splitFree
                    }
                }
            }
        }
        Kirigami.PlaceholderMessage {
            visible: page.tab === 2 && page.categories.length === 0
            Layout.fillWidth: true
            text: i18nc("%1 is a month and year", "No spending in %1", page.monthName)
        }
    }
}
