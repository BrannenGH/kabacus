// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "statementImport"
    title: saved ? i18n("Import complete") : result ? i18n("Review") : i18n("Import statement…")
    required property var app
    required property string account
    required property string accountName
    readonly property var ledger: app.ledger
    globalToolBarStyle: app.narrow ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    property var file: null
    property var statements: null
    property var inspection: null
    property var mapping: ({})
    property var result: null
    property var saved: null
    property var decisions: ({})
    property string head: ""
    property string problem: ""
    property string stage: ""
    property int ticket: 0
    property bool splitAmounts: false
    property bool needsReview: false
    readonly property bool busy: stage !== ""
    readonly property bool ready: file !== null && (!inspection || (mapping.date != null && mapping.description?.length > 0 &&
        (splitAmounts ? mapping.debit != null && mapping.credit != null : mapping.amount != null)))
    readonly property bool canSave: result !== null && !needsReview && result.counts.new + result.counts.matched > 0

    function send(method, path, body, phase) {
        stage = phase
        ticket = ledger.startRequest(JSON.stringify({ method, path, head, body }))
    }
    function importRequest(commit, cursor) {
        problem = ""
        const body = Object.assign({}, file, { dry_run: !commit, limit: 100, cursor: cursor || null, decisions: Object.values(decisions) })
        if (inspection) body.mapping = mapping
        if (statements) body.statement = statements[statementColumn.currentIndex].statement
        send("POST", "/accounts/" + encodeURIComponent(account) + "/import", body, commit ? "commit" : "preview")
    }
    function start() {
        if (head === "") send("GET", "/version", null, "head")
        else importRequest(false)
    }
    function setColumn(key, value) {
        const next = Object.assign({}, mapping)
        if (value < 0) delete next[key]
        else next[key] = value
        mapping = next
    }
    function reviewChoice(row: var, value: string) {
        const next = Object.assign({}, decisions)
        const components = value.split(":")
        const decision = {import_id: row.import_id, action: components[0]}
        if (components.length === 2) decision.transaction = Number(components[1])
        next[row.import_id] = decision
        decisions = next
        needsReview = true
    }
    function categorize() {
        app.go("Accounts", { open: "expenses:uncategorized" })
    }

    Connections {
        target: page.ledger
        function onRequestFinished(ticket: int, reply: string) {
            if (ticket !== page.ticket) return
            const phase = page.stage
            page.stage = ""
            const response = JSON.parse(reply)
            const body = response.body
            if (response.status >= 400) {
                page.problem = body.message || body.error || ""
                if (response.status === 409) { page.head = ""; page.needsReview = true }
                return
            }
            if (phase === "head") {
                page.head = body.head
                page.importRequest(false)
            } else if (phase === "commit") {
                page.saved = body
                page.result = null
                page.ledger.refresh()
            } else if (body.csv) {
                page.inspection = body.csv
                const savedMapping = JSON.parse(page.ledger.statementMapping(page.account))
                const compatible = savedMapping && JSON.stringify(savedMapping.columns) === JSON.stringify(body.csv.columns)
                page.mapping = compatible ? savedMapping.mapping : body.csv.mapping || {description: [], header: body.csv.header, delimiter: body.csv.delimiter}
                page.splitAmounts = page.mapping.amount == null && page.mapping.debit != null
            } else if (body.statements) {
                page.statements = body.statements
            } else {
                page.result = body
                page.needsReview = false
                page.head = body.head
                if (page.inspection) page.ledger.rememberStatementMapping(page.account, JSON.stringify({columns: page.inspection.columns, mapping: page.mapping}))
            }
        }
    }

    FileDialog {
        id: chooser
        title: i18n("Import statement…")
        nameFilters: [i18n("Statements (%1)", "*.ofx *.qfx *.csv"), i18n("All files (%1)", "*")]
        onAccepted: {
            const read = JSON.parse(page.ledger.readStatement(page.app.localPath(selectedFile)))
            if (read.error) { page.problem = read.error; return }
            page.file = read.ok
            page.problem = ""
            page.statements = null
            page.inspection = null
            page.result = null
            page.head = ""
            page.decisions = ({})
            page.start()
        }
    }

    footer: Controls.ToolBar {
        RowLayout {
            width: parent.width
            Controls.Button {
                text: page.saved ? i18n("Close") : page.result ? i18n("Back to edit") : i18n("Cancel")
                enabled: !page.busy
                onClicked: {
                    if (page.result) page.result = null
                    else page.app.pageStack.layers.pop()
                }
            }
            Item { Layout.fillWidth: true }
            Controls.Button {
                text: page.saved ? i18n("Categorize") : page.canSave ? i18n("Import") : i18n("Review")
                highlighted: true
                enabled: !page.busy && (page.saved !== null || (page.app.canWrite && page.ready && (page.result === null || page.needsReview || page.canSave)))
                implicitHeight: page.app.narrow ? 48 : implicitContentHeight + topPadding + bottomPadding
                onClicked: {
                    if (page.saved) page.categorize()
                    else if (page.canSave) page.importRequest(true)
                    else page.start()
                }
            }
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing
        Controls.Label { text: page.accountName; font.bold: true; Layout.fillWidth: true; wrapMode: Text.Wrap }
        Kirigami.InlineMessage { Layout.fillWidth: true; visible: page.problem !== ""; text: page.problem; type: Kirigami.MessageType.Error }
        Controls.Label { visible: page.busy; text: i18n("Checking with the server…"); Accessible.name: text }
        Controls.Label {
            visible: page.saved !== null
            text: page.saved ? i18nc("%1 is the number of new transactions, %2 is the number matched", "Imported %1 new and matched %2 transactions.", page.saved.counts.new, page.saved.counts.matched) : ""
            Layout.fillWidth: true; wrapMode: Text.Wrap
        }
        Controls.Button {
            text: i18n("Statement file")
            visible: page.saved === null && page.result === null
            enabled: !page.busy && page.app.canWrite
            onClicked: chooser.open()
        }
        Controls.Label { text: page.file?.filename || ""; textFormat: Text.PlainText; visible: page.saved === null && page.result === null; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere }
        Controls.ComboBox {
            id: statementColumn
            visible: page.statements !== null && page.result === null && page.saved === null
            model: (page.statements || []).map(item => (item.account_hint || item.statement + 1) + " · " + item.rows)
            Accessible.name: i18n("Statement")
            Layout.fillWidth: true
        }
        ColumnLayout {
            visible: page.inspection !== null && page.result === null && page.saved === null
            Layout.fillWidth: true
            Kirigami.Heading { level: 2; text: i18n("Match CSV columns") }
            Controls.Label { text: page.inspection?.sample[0]?.join(" · ") || ""; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.Wrap }
            Controls.CheckBox {
                text: i18n("Separate debit and credit columns")
                checked: page.splitAmounts
                onToggled: {
                    page.splitAmounts = checked
                    page.setColumn("amount", -1); page.setColumn("debit", -1); page.setColumn("credit", -1)
                }
            }
            Repeater {
                model: [
                    {key: "date", label: i18n("Date")},
                    {key: "amount", label: i18n("Amount")},
                    {key: "debit", label: i18n("Debit")},
                    {key: "credit", label: i18n("Credit")},
                    {key: "id", label: i18n("Transaction ID")},
                    {key: "balance", label: i18n("Balance")}
                ]
                delegate: ColumnLayout {
                    id: mappingField
                    required property var modelData
                    visible: modelData.key === "amount" ? !page.splitAmounts : ["debit", "credit"].includes(modelData.key) ? page.splitAmounts : true
                    Layout.fillWidth: true
                    Controls.Label { text: mappingField.modelData.label }
                    Controls.ComboBox {
                        Accessible.name: mappingField.modelData.label
                        model: [i18n("None")].concat(page.inspection?.columns || [])
                        currentIndex: (page.mapping[mappingField.modelData.key] ?? -1) + 1
                        onActivated: index => page.setColumn(mappingField.modelData.key, index - 1)
                        Layout.fillWidth: true
                    }
                }
            }
            Controls.Label { text: i18n("Date format") }
            Controls.ComboBox {
                readonly property var formats: [null, "%Y-%m-%d", "%m/%d/%Y", "%d/%m/%Y", "%d.%m.%Y"]
                model: [i18n("Automatic"), "2026-10-25", "10/25/2026", "25/10/2026", "25.10.2026"]
                currentIndex: Math.max(0, formats.indexOf(page.mapping.date_format))
                Accessible.name: i18n("Date format")
                Layout.fillWidth: true
                onActivated: index => page.mapping = Object.assign({}, page.mapping, {date_format: formats[index]})
            }
            Controls.Label { text: i18n("Description") }
            Repeater {
                model: page.inspection?.columns || []
                delegate: Controls.CheckBox {
                    required property int index
                    required property string modelData
                    text: modelData
                    checked: page.mapping.description?.includes(index) || false
                    onToggled: {
                        const before = page.mapping.description || []
                        page.mapping = Object.assign({}, page.mapping, {description: checked ? before.concat(index) : before.filter(i => i !== index)})
                    }
                }
            }
            Controls.CheckBox {
                visible: !page.splitAmounts
                text: i18n("Reverse amount signs")
                checked: page.mapping.invert === true
                onToggled: page.mapping = Object.assign({}, page.mapping, {invert: checked})
            }
        }
        ColumnLayout {
            visible: page.result !== null
            Layout.fillWidth: true
            Kirigami.Heading { level: 2; text: i18n("Review before saving") }
            GridLayout {
                columns: 2
                Layout.fillWidth: true
                Repeater {
                    model: page.result ? [
                        {label: i18n("New transactions"), value: page.result.counts.new},
                        {label: i18n("Matched"), value: page.result.counts.matched},
                        {label: i18n("Already imported"), value: page.result.counts.duplicate},
                        {label: i18n("Skipped"), value: page.result.counts.skipped}
                    ] : []
                    delegate: RowLayout {
                        id: countRow
                        required property var modelData
                        Layout.fillWidth: true
                        Controls.Label { text: countRow.modelData.label; Layout.fillWidth: true; wrapMode: Text.Wrap }
                        Controls.Label { text: countRow.modelData.value; font.bold: true }
                    }
                }
            }
            Repeater {
                model: page.result?.warnings || []
                delegate: Kirigami.InlineMessage { required property var modelData; text: modelData.message; visible: true; Layout.fillWidth: true; type: Kirigami.MessageType.Warning }
            }
            Kirigami.Heading { level: 3; text: i18n("Balance check"); visible: page.result?.balance_check !== undefined }
            Repeater {
                model: page.result?.balance_check ? [
                    {label: i18n("Statement balance"), value: page.result.balance_check.reported},
                    {label: i18n("Books balance"), value: page.result.balance_check.books},
                    {label: i18n("Difference"), value: page.result.balance_check.difference}
                ] : []
                delegate: RowLayout {
                    id: balanceRow
                    required property var modelData
                    Layout.fillWidth: true
                    Controls.Label { text: balanceRow.modelData.label; Layout.fillWidth: true; wrapMode: Text.Wrap }
                    Controls.Label { text: balanceRow.modelData.value }
                }
            }
            Repeater {
                model: page.result?.rows || []
                delegate: ColumnLayout {
                    id: row
                    required property var modelData
                    Layout.fillWidth: true
                    RowLayout {
                        Layout.fillWidth: true
                        Controls.Label { text: row.modelData.description; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere }
                        Controls.Label { text: row.modelData.amount }
                    }
                    Controls.Label { text: row.modelData.date; font: Kirigami.Theme.smallFont }
                    Controls.ComboBox {
                        Layout.fillWidth: true
                        Accessible.name: i18n("Action")
                        textRole: "label"
                        readonly property var choices: (row.modelData.candidates || []).map(id => ({key: "match:" + id, label: i18nc("%1 is a transaction number", "Match transaction %1", id)}))
                            .concat([{key: "add", label: i18n("Import as new")}, {key: "skip", label: i18n("Skip")}])
                        model: choices
                        currentIndex: {
                            const decision = page.decisions[row.modelData.import_id]
                            const key = decision ? (decision.action === "match" ? "match:" + decision.transaction : decision.action)
                                : row.modelData.outcome === "matched" ? "match:" + row.modelData.transaction : row.modelData.outcome === "skipped" ? "skip" : "add"
                            return choices.findIndex(choice => choice.key === key)
                        }
                        onActivated: index => page.reviewChoice(row.modelData, choices[index].key)
                    }
                }
            }
            Controls.Button {
                text: i18n("Load more")
                visible: !!page.result?.next_cursor && !page.needsReview
                enabled: !page.busy
                onClicked: page.importRequest(false, page.result.next_cursor)
            }
        }
    }
}
