// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ScrollablePage {
    id: page
    objectName: "bankConnections"
    title: i18n("Bank connections")
    required property var app
    readonly property var ledger: app.ledger
    readonly property var status: app.bankStatus
    readonly property var targets: (accounts.value?.accounts || []).filter(account => !account.synthetic && ["money", "revolving-debt"].includes(account.kind))
    readonly property bool busy: phase !== ""
    property var listed: null
    property var confirmation: null
    property string problem: ""
    property string notice: ""
    property string phase: ""
    property int ticket: 0
    property bool replacing: false
    globalToolBarStyle: app.narrow ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    property Fetch accounts: Fetch { ledger: page.ledger; view: "picker" }

    function send(method, path, body, nextPhase, head) {
        phase = nextPhase
        ticket = ledger.startRequest(JSON.stringify({method, path, body, head: head || ""}))
    }
    function request(method, path, body, nextPhase) {
        problem = ""
        notice = ""
        send(method, path, body, nextPhase)
    }
    property var linkChange: null
    function link(remote, target, create) {
        linkChange = { account: remote, target, path: target || remote.linked, create }
        request("GET", "/version", null, "head")
    }
    function name(account) {
        const target = targets.find(item => item.account === account)
        return target?.name || account
    }

    Connections {
        target: page.ledger
        function onRequestFinished(ticket: int, reply: string) {
            if (ticket !== page.ticket) return
            const operation = page.phase
            page.phase = ""
            const response = JSON.parse(reply)
            if (response.status >= 400) {
                if (operation === "connected" && response.body.code === "simplefin_connected") {
                    page.confirmation = {host: page.confirmation.host, connected: true}
                    page.replacing = true
                    page.app.loadBankStatus()
                    return
                }
                if (response.body.code === "simplefin_payment") {
                    page.problem = i18n("Sorry, SimpleFIN is showing that your account with them is not paid. A paid SimpleFIN account is required")
                    if (operation === "connected") {
                        page.listed = null
                        page.confirmation = null
                        token.text = ""
                        page.replacing = false
                    }
                } else {
                    page.problem = response.body.message || ""
                }
                if (operation === "connected" || operation === "list") page.app.loadBankStatus()
                return
            }
            if (operation === "head") {
                const change = page.linkChange
                const remote = change.target ? {conn_id: change.account.conn_id, id: change.account.id} : null
                page.send("PUT", "/accounts/" + encodeURIComponent(change.path) + "/simplefin-link", {remote, create: change.create}, "link", response.body.head)
            } else if (operation === "preview") {
                page.confirmation = response.body
            } else if (operation === "connected") {
                page.confirmation = null
                token.text = ""
                page.replacing = false
                page.listed = response.body
                page.app.loadBankStatus()
            } else if (operation === "disconnected") {
                page.listed = null
                page.notice = response.body.message
                page.app.loadBankStatus()
            } else if (operation === "link") {
                const change = page.linkChange
                const updated = page.listed.accounts.map(account =>
                    account.conn_id === change.account.conn_id && account.id === change.account.id
                        ? Object.assign({}, account, {linked: change.target || null}) : account)
                page.listed = Object.assign({}, page.listed, {accounts: updated})
                page.ledger.refresh()
            } else {
                page.listed = response.body
                page.app.loadBankStatus()
            }
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing
        Kirigami.Heading { text: i18n("Your bank connection"); visible: page.app.serverBooks; level: 2 }
        Controls.Label { text: page.problem || page.app.bankSyncProblem; visible: text !== ""; textFormat: Text.PlainText; color: Kirigami.Theme.negativeTextColor; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Controls.Label { text: page.notice; visible: text !== ""; textFormat: Text.PlainText; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Controls.Label { text: i18n("Checking with the server…"); visible: page.busy }
        ColumnLayout {
            visible: page.status?.connected === true && !page.replacing
            Layout.fillWidth: true
            Controls.Label { text: page.status?.host || ""; textFormat: Text.PlainText }
            Controls.Label { text: page.status ? i18nc("%1 is the remaining request count", "Requests remaining today: %1", page.status.remaining ?? 0) : "" }
            Flow {
                spacing: Kirigami.Units.smallSpacing
                Layout.fillWidth: true
                Layout.preferredHeight: implicitHeight
                Controls.Button { height: page.app.narrow ? 48 : implicitHeight; text: i18n("Sync"); enabled: !page.busy; highlighted: true; onClicked: page.app.openBankSync() }
                Controls.Button { height: page.app.narrow ? 48 : implicitHeight; text: i18n("Refresh accounts"); enabled: !page.busy; onClicked: page.request("GET", "/simplefin/accounts?refresh=true", null, "list") }
            }
            Flow {
                spacing: Kirigami.Units.smallSpacing
                Layout.fillWidth: true
                Layout.preferredHeight: implicitHeight
                Controls.Button { height: page.app.narrow ? 48 : implicitHeight; text: i18n("Replace connection"); enabled: !page.busy; flat: true; onClicked: page.replacing = true }
                Controls.Button { height: page.app.narrow ? 48 : implicitHeight; text: i18n("Disconnect"); enabled: !page.busy; flat: true; onClicked: page.request("DELETE", "/simplefin/connection", null, "disconnected") }
            }
        }
        ColumnLayout {
            visible: !page.status?.connected || page.replacing
            Layout.fillWidth: true
            Controls.Button { Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; text: i18n("Get a SimpleFIN setup token"); flat: true; onClicked: Qt.openUrlExternally("https://bridge.simplefin.org/simplefin/create") }
            Controls.Label { text: i18n("Setup token") }
            Controls.TextField {
                id: token
                echoMode: TextInput.Password
                Accessible.role: Accessible.EditableText
                Accessible.passwordEdit: true
                Accessible.name: i18n("Setup token")
                enabled: !page.busy
                Layout.fillWidth: true
                Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                onTextEdited: { page.confirmation = null; page.problem = "" }
            }
            Controls.Label { text: page.confirmation ? i18nc("%1 is the hostname", "Connect to %1?", page.confirmation.host) : ""; visible: page.confirmation !== null; wrapMode: Text.Wrap; Layout.fillWidth: true }
            Controls.Label { text: i18n("Anyone with this file can read your connected bank transactions until you revoke access at your SimpleFIN server."); visible: page.confirmation !== null && !page.app.serverBooks; Layout.fillWidth: true; wrapMode: Text.Wrap }
            Controls.Label { text: i18n("This replaces your current bank connection."); visible: page.confirmation?.connected === true; Layout.fillWidth: true; wrapMode: Text.Wrap }
            RowLayout {
                Controls.Button { text: i18n("Cancel"); visible: page.replacing || page.confirmation !== null; Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; enabled: !page.busy; onClicked: { page.replacing = false; page.confirmation = null; page.problem = ""; token.text = "" } }
                Controls.Button {
                    text: !page.confirmation ? i18n("Connect SimpleFIN")
                        : page.confirmation.connected ? i18n("Replace connection") : i18n("Connect")
                    enabled: !page.busy && token.text.trim() !== "" && page.app.canWrite
                    highlighted: true
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    onClicked: page.request("PUT", "/simplefin/connection", {token:token.text, dry_run:!page.confirmation, replace:page.confirmation?.connected === true}, page.confirmation ? "connected" : "preview")
                }
            }
        }
        Repeater {
            model: page.listed?.warnings || page.status?.warnings || []
            delegate: Controls.Label { required property var modelData; text: modelData.message; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.Wrap }
        }
        Repeater {
            model: page.replacing ? [] : page.listed?.connections || []
            delegate: ColumnLayout {
                id: institution
                required property var modelData
                Layout.fillWidth: true
                Kirigami.Heading { level: 3; text: institution.modelData.name; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.Wrap }
                Repeater {
                    model: page.listed.accounts.filter(item => item.conn_id === institution.modelData.conn_id)
                    delegate: ColumnLayout {
                        id: remote
                        required property var modelData
                        Layout.fillWidth: true
                        Controls.Label { text: remote.modelData.name; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.Wrap }
                        Controls.Label { text: remote.modelData.balance + " " + remote.modelData.currency; textFormat: Text.PlainText }
                        property string createKind: ""
                        readonly property var choices: {
                            const skip = {label: i18n("Don’t import"), account: ""}
                            if (remote.modelData.linked)
                                return [skip, {label: page.name(remote.modelData.linked), account: remote.modelData.linked}]
                            const occupied = new Set((page.listed.other_links || []).concat(page.listed.accounts.map(item => item.linked)))
                            return [skip].concat(page.targets.filter(item => !occupied.has(item.account))
                                .map(item => ({label: item.name || item.account, account: item.account})),
                                [{label: i18n("New bank account…"), account: ":money"},
                                 {label: i18n("New credit card…"), account: ":revolving-debt"}])
                        }
                        function selectedIndex() {
                            const selected = createKind ? ":" + createKind : modelData.linked || ""
                            return choices.findIndex(item => item.account === selected)
                        }
                        Controls.ComboBox {
                            id: mapping
                            Layout.fillWidth: true
                            Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                            Accessible.name: remote.modelData.name
                            model: remote.choices
                            textRole: "label"
                            currentIndex: remote.selectedIndex()
                            enabled: !page.busy && page.app.canWrite
                            onActivated: index => {
                                const choice = remote.choices[index].account
                                if (choice.startsWith(":")) {
                                    remote.createKind = choice.slice(1)
                                    accountName.forceActiveFocus()
                                } else {
                                    remote.createKind = ""
                                    if (choice || remote.modelData.linked)
                                        page.link(remote.modelData, choice)
                                }
                                currentIndex = Qt.binding(() => remote.selectedIndex())
                            }
                        }
                        ColumnLayout {
                            visible: remote.createKind !== ""
                            Layout.fillWidth: true
                            Controls.Label { text: i18n("Name") }
                            Controls.TextField {
                                id: accountName
                                Accessible.name: i18n("Name")
                                Layout.fillWidth: true
                                Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                                enabled: !page.busy
                            }
                            RowLayout {
                                Controls.Button {
                                    text: i18n("Cancel")
                                    enabled: !page.busy
                                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                                    onClicked: { remote.createKind = ""; accountName.clear(); mapping.forceActiveFocus() }
                                }
                                Controls.Button {
                                    text: i18n("Add")
                                    highlighted: true
                                    enabled: !page.busy && accountName.text.trim() !== "" && !accountName.text.includes(":")
                                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                                    onClicked: {
                                        const prefix = remote.createKind === "money" ? "assets:bank:" : "liabilities:credit-card:"
                                        page.link(remote.modelData, prefix + accountName.text.trim(), remote.createKind)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Kirigami.Heading { visible: (page.listed?.other_links || []).length > 0; text: i18n("Linked through another member’s connection"); level: 3; Layout.fillWidth: true; wrapMode: Text.Wrap }
        Repeater {
            model: page.listed?.other_links || []
            delegate: Controls.Label { required property string modelData; text: page.name(modelData); textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.Wrap }
        }
    }
}
