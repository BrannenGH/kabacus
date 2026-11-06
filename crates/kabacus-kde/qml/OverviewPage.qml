// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Cash on Hand first, beside the free money it comes from, then the
// warnings that put it at risk (each with the one action that fixes it),
// then the reserves and cards it's kept apart from. Each figure says what it
// means behind its ⓘ, and every row opens where it can be changed.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "overview"
    title: i18n("Overview")

    required property var app
    readonly property var ledger: app.ledger
    readonly property var overview: fetch.value
    readonly property string problem: fetch.problem
    property Fetch fetch: Fetch { ledger: page.ledger; view: "overview"; owner: page }

    // Each warning's one fix.
    function fix(w) {
        if (w.target === "card") app.go("Cards", { focusCard: w.account })
        else if (w.target === "register") app.go("Accounts", { open: w.account || "expenses:uncategorized" })
        else if (w.target === "funds") app.go("Funds")
        else if (w.target === "settings") app.openSettings()
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        // The page's actions, labelled.
        Kirigami.ActionToolBar {
            Layout.fillWidth: true
            alignment: page.app.narrow ? Qt.AlignHCenter : Qt.AlignRight
            actions: [
                Kirigami.Action { text: i18n("Add"); icon.name: "list-add"; displayHint: Kirigami.DisplayHint.KeepVisible; enabled: page.app.canWrite; shortcut: StandardKey.New; onTriggered: page.app.openChooser() },
                Kirigami.Action { text: i18n("Set aside"); icon.name: "folder-text"; enabled: page.app.canWrite; onTriggered: page.app.openForm("set_aside") },
                Kirigami.Action { text: i18n("Refresh"); icon.name: "view-refresh"; shortcut: StandardKey.Refresh; onTriggered: page.ledger.refresh() }
            ]
        }

        Kirigami.PlaceholderMessage {
            visible: page.overview === null
            Layout.fillWidth: true
            text: page.app.needsSignIn ? i18n("Not signed in") : page.problem !== "" ? i18n("Can't load Cash on Hand") : i18n("Loading…")
            explanation: page.problem
        }

        // Two columns, each card half the page, where there's room (a
        // desktop or tablet window); one on a phone, and beside a register
        // on a wide window, where the page is a narrow column.
        GridLayout {
            id: grid
            visible: page.overview !== null
            columns: page.width >= Kirigami.Units.gridUnit * 30 ? 2 : 1
            uniformCellWidths: true
            columnSpacing: Kirigami.Units.largeSpacing
            rowSpacing: Kirigami.Units.largeSpacing
            Layout.fillWidth: true

            // Cash on Hand, and the sum it comes from behind Money on Hand.
            Kirigami.AbstractCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.alignment: Qt.AlignTop
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing
                    RowLayout {
                        Controls.Label { text: i18n("Cash on Hand"); font.bold: true }
                        InfoButton {
                            text: i18n("About Cash on Hand")
                            topic: i18n("Cash on Hand")
                            explanation: i18n("Free money you can spend right now, after reserves and card reserves.")
                        }
                        Item { Layout.fillWidth: true }
                        Kirigami.Chip {
                            visible: page.overview !== null && page.overview.change !== null
                            closable: false
                            checkable: false
                            // No arrow when nothing changed.
                            icon.name: !page.overview || !page.overview.change || page.overview.change.tone === "zero" ? ""
                                : page.overview.change.tone === "neg" ? "go-down" : "go-up"
                            text: page.overview && page.overview.change
                                ? i18nc("%1 is the change in Cash on Hand, %2 a date", "%1 since %2", (page.overview.change.tone === "pos" ? "+" : "") + page.overview.change.text, page.overview.compare_to) : ""
                        }
                    }
                    Money {
                        objectName: "cashOnHand"
                        amount: page.overview ? page.overview.cash_on_hand : null
                        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 2.6
                        font.weight: Font.Light
                        horizontalAlignment: Text.AlignLeft
                        Accessible.name: i18nc("%1 is an amount", "Cash on Hand %1", text)
                    }
                    Kirigami.Separator { Layout.fillWidth: true }
                    // Closed at first, on every form factor: Cash on Hand is
                    // the figure, the sum is there when asked for.
                    RowLayout {
                        Layout.fillWidth: true
                        Controls.ToolButton {
                            id: breakdown
                            objectName: "moneyOnHand"
                            checkable: true
                            text: i18n("Money on Hand")
                            icon.name: checked ? "arrow-up" : "arrow-down"
                            display: Controls.AbstractButton.TextBesideIcon
                            Accessible.name: text
                        }
                        InfoButton {
                            text: i18n("About Money on Hand")
                            topic: i18n("Money on Hand")
                            explanation: i18n("Everything in your bank accounts, reserves and card reserves included, plus what people owe you. Cash on Hand is what's left once reserves, card reserves and what you're owed are set apart.")
                        }
                        Item { Layout.fillWidth: true }
                        Money { amount: page.overview ? page.overview.money_on_hand : null; colored: false }
                    }
                    GridLayout {
                        visible: breakdown.checked
                        columns: 2
                        Layout.fillWidth: true
                        // Wraps rather than elides: translations run longer.
                        Controls.Label { text: i18n("− Future allocations (reserves and card reserves)"); Layout.fillWidth: true; wrapMode: Text.Wrap }
                        Money { amount: page.overview ? page.overview.future_allocations : null; colored: false }
                        Controls.Label { text: i18n("− Receivables (owed to you)"); Layout.fillWidth: true; wrapMode: Text.Wrap }
                        Money { amount: page.overview ? page.overview.receivables : null; colored: false }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            // Only accounts with free money (views.rs leaves the rest
            // off); each opens its register.
            Kirigami.AbstractCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.alignment: Qt.AlignTop
                contentItem: ColumnLayout {
                    Controls.Label { text: i18n("Free money by account"); font.bold: true }
                    Repeater {
                        model: page.overview ? page.overview.homes : []
                        delegate: Controls.ItemDelegate {
                            id: homeDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: i18nc("%1 is an account, %2 its free money", "%1 free %2", modelData.name, modelData.free.text)
                            onClicked: page.app.showRegister(modelData.account, modelData.name)
                            contentItem: RowLayout {
                                Controls.Label { text: homeDelegate.modelData.name; elide: Text.ElideRight; Layout.fillWidth: true }
                                Money { amount: homeDelegate.modelData.free }
                            }
                        }
                    }
                    Controls.Label {
                        visible: page.overview !== null && page.overview.homes.length === 0
                        text: i18n("No account has free money right now.")
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        Layout.fillWidth: true
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            // Warnings, each with its fix, across the page.
            Repeater {
                model: page.overview ? page.overview.warnings : []
                delegate: Kirigami.InlineMessage {
                    id: warningDelegate
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.columnSpan: grid.columns
                    visible: true
                    text: modelData.message
                    type: modelData.code === "negative-free-money" ? Kirigami.MessageType.Error
                        : modelData.code === "unclassified" || modelData.code === "duplicate-display-name"
                            ? Kirigami.MessageType.Information : Kirigami.MessageType.Warning
                    actions: modelData.action !== "" ? [fixAction] : []
                    Kirigami.Action {
                        id: fixAction
                        text: warningDelegate.modelData.action
                        onTriggered: page.fix(warningDelegate.modelData)
                    }
                }
            }

            // Reserves (funds): only those with money in them (views.rs
            // leaves the rest off; the Reserves page has them all). Each
            // opens the set-aside form for it, where it can be topped up,
            // released or moved.
            Kirigami.AbstractCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.alignment: Qt.AlignTop
                contentItem: ColumnLayout {
                    RowLayout {
                        Controls.Label { text: page.app.labels.Funds; font.bold: true; Layout.fillWidth: true }
                        Money { amount: page.overview ? page.overview.funds_total : null; colored: false; opacity: 0.7 }
                    }
                    Repeater {
                        model: page.overview ? page.overview.funds : []
                        delegate: Controls.ItemDelegate {
                            id: fundDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: modelData.name
                            enabled: page.app.canWrite
                            onClicked: page.app.openForm("set_aside", { from: modelData.home, to: modelData.account })
                            contentItem: RowLayout {
                                Controls.Label { text: fundDelegate.modelData.name; elide: Text.ElideRight; Layout.fillWidth: true }
                                Money { amount: fundDelegate.modelData.amount }
                            }
                        }
                    }
                    Controls.Label {
                        visible: page.overview !== null && page.overview.funds.length === 0
                        text: i18n("Nothing is set aside in a reserve.")
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        Layout.fillWidth: true
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            // Each card opens on the Cards page.
            Kirigami.AbstractCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.alignment: Qt.AlignTop
                contentItem: ColumnLayout {
                    Controls.Label { text: page.app.labels.Cards; font.bold: true }
                    Repeater {
                        model: page.overview ? page.overview.cards : []
                        delegate: Controls.ItemDelegate {
                            id: cardDelegate
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: modelData.name
                            onClicked: page.app.go("Cards", { focusCard: modelData.card })
                            contentItem: RowLayout {
                                Controls.Label { text: cardDelegate.modelData.name; elide: Text.ElideRight; Layout.fillWidth: true }
                                Kirigami.Chip {
                                    closable: false
                                    checkable: false
                                    text: cardDelegate.modelData.covered ? i18nc("a card's reserve covers what it owes", "covered") : i18nc("%1 is an amount", "%1 still to reserve", cardDelegate.modelData.short.text)
                                    onClicked: page.app.go("Cards", { focusCard: cardDelegate.modelData.card })
                                }
                                Money { amount: cardDelegate.modelData.owed; colored: false }
                            }
                        }
                    }
                    Controls.Label {
                        text: i18n("Each card's reserve should equal what it owes.")
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        Layout.fillWidth: true
                    }
                    Item { Layout.fillHeight: true }
                }
            }
        }

        Kirigami.AbstractCard {
            visible: page.overview !== null && page.overview.history.length > 1
            Layout.fillWidth: true
            contentItem: ColumnLayout {
                Controls.Label { text: i18n("Cash on Hand over time"); font.bold: true }
                Canvas {
                    id: chart
                    Layout.fillWidth: true
                    Layout.preferredHeight: Kirigami.Units.gridUnit * 4
                    property var points: page.overview ? page.overview.history : []
                    // What the line shows, for screen readers.
                    readonly property var values: points.map(p => p.value)
                    readonly property var lowest: points.reduce((a, p) => a === null || p.y < a.y ? p : a, null)
                    readonly property var highest: points.reduce((a, p) => a === null || p.y > a.y ? p : a, null)
                    Accessible.role: Accessible.Chart
                    Accessible.name: i18n("Cash on Hand over time")
                    Accessible.description: points.length < 2 || values.length !== points.length || !lowest || !highest ? ""
                        : i18nc("%1 and %3 are amounts on dates %2 and %4; %5 and %6 the lowest and highest amounts",
                            "From %1 on %2 to %3 on %4; lowest %5, highest %6",
                            values[0].text, points[0].date, values[values.length - 1].text,
                            points[points.length - 1].date, lowest.value.text, highest.value.text)
                    onPointsChanged: requestPaint()
                    onWidthChanged: requestPaint()
                    onPaint: {
                        const ctx = getContext("2d")
                        ctx.reset()
                        const n = points.length
                        if (n < 2) return
                        const pad = 6
                        // Time runs the way the language reads: right to
                        // left when the app is mirrored, so the line meets
                        // the start and end labels below.
                        const mirrored = chart.LayoutMirroring.enabled
                        ctx.strokeStyle = Kirigami.Theme.highlightColor
                        ctx.lineWidth = 2.5
                        ctx.beginPath()
                        for (let i = 0; i < n; i++) {
                            const along = pad + i * (width - 2 * pad) / (n - 1)
                            const x = mirrored ? width - along : along
                            const y = height - pad - points[i].y * (height - 2 * pad)
                            if (i === 0) ctx.moveTo(x, y)
                            else ctx.lineTo(x, y)
                        }
                        ctx.stroke()
                    }
                }
                // Its start and end, in words.
                RowLayout {
                    visible: chart.points.length > 1
                    Layout.fillWidth: true
                    Controls.Label { text: chart.points.length > 1 && chart.values.length === chart.points.length ? i18nc("the chart's first point: %1 is its date, %2 its amount", "%1  %2", chart.points[0].date, chart.values[0].text) : ""; font: Kirigami.Theme.smallFont; opacity: 0.7; Layout.fillWidth: true }
                    Controls.Label { text: chart.points.length > 1 && chart.values.length === chart.points.length ? i18nc("the chart's last point, at its far end: %1 is its amount, %2 its date", "%1  %2", chart.values[chart.values.length - 1].text, chart.points[chart.points.length - 1].date) : ""; font: Kirigami.Theme.smallFont; opacity: 0.7 }
                }
            }
        }
        // On a phone, room for the floating Add button.
        Item { visible: page.app.narrow; Layout.preferredHeight: Kirigami.Units.gridUnit * 4 }
    }
}
