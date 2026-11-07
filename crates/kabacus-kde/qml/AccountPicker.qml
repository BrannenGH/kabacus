// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// An account field: pick from the list or type a name or account path.
// Shows the server's or the form's error under it and, for an account that
// doesn't exist, offers the closest one.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

ColumnLayout {
    id: picker

    readonly property bool phone: picker.Window.window !== null && picker.Window.window.width < Theme.phoneBreakpoint
    property string label: ""
    // [{account, name, kind, home}]
    property var accounts: []
    // The chosen account path.
    property string value: ""
    property string error: ""
    property string suggestion: ""
    // Set while a new account list goes in: the ComboBox then resets its
    // text, which isn't the user's edit.
    property bool restoring: false

    signal edited()

    spacing: Kirigami.Units.smallSpacing
    Kirigami.FormData.label: i18nc("@label a form field's name", "%1:", label)
    // The label sits beside the field, not halfway down the field with
    // its error and suggestion under it.
    Kirigami.FormData.buddyFor: combo

    function nameOf(account) {
        const a = accounts.find(x => x.account === account)
        return a ? a.name : account
    }

    function resolve(text) {
        const t = text.trim().toLowerCase()
        if (t === "") return ""
        const hit = accounts.find(a => a.account.toLowerCase() === t)
            || accounts.find(a => a.name.toLowerCase() === t)
        return hit ? hit.account : text.trim()
    }

    function show(account) {
        const i = accounts.findIndex(a => a.account === account)
        combo.currentIndex = i
        combo.editText = i >= 0 ? accounts[i].name : account
    }

    function set(account) {
        show(account)
        value = account
        error = ""
        suggestion = ""
    }

    // A reloaded account list keeps the chosen account. The model is
    // assigned here, not bound, so its reset happens while `restoring`.
    function reload() {
        restoring = true
        combo.model = accounts
        show(value)
        restoring = false
    }
    onAccountsChanged: reload()
    Component.onCompleted: reload()

    Controls.ComboBox {
        id: combo
        implicitHeight: picker.phone ? 48 : Math.max(contentItem.implicitHeight + topPadding + bottomPadding, background ? background.implicitHeight : 0)
        font.pixelSize: picker.phone ? 16 : Kirigami.Theme.defaultFont.pixelSize
        MouseArea {
            anchors.fill: parent
            enabled: picker.phone
            z: 1
            onClicked: accountSheet.open()
        }
        editable: true
        textRole: "name"
        valueRole: "account"
        currentIndex: -1
        Accessible.name: picker.label
        // What it shows, for a screen reader (and the GUI e2e).
        Accessible.description: editText
        Layout.fillWidth: true
        Layout.minimumWidth: Kirigami.Units.gridUnit * 12
        onActivated: {
            picker.value = currentValue
            picker.error = ""
            picker.edited()
        }
        onEditTextChanged: {
            if (picker.restoring) return
            const v = picker.resolve(editText)
            if (v !== picker.value) {
                picker.value = v
                picker.edited()
            }
        }
        onActiveFocusChanged: if (activeFocus) { contentItem.forceActiveFocus(); (contentItem as TextInput)?.selectAll() }
    Controls.Dialog {
        id: accountSheet
        title: picker.label
        parent: Controls.Overlay.overlay
        modal: true
        x: 0; y: 0
        width: parent ? parent.width : 0
        height: parent ? parent.height : 0
        padding: 16
        onOpened: accountSearch.text = ""
        header: Controls.ToolBar {
            RowLayout {
                anchors.fill: parent
                Controls.ToolButton { text: i18n("Back"); implicitHeight: 48; onClicked: accountSheet.close() }
                Kirigami.Heading { text: picker.label; Layout.fillWidth: true }
            }
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 8
            Controls.TextField {
                id: accountSearch
                Accessible.name: i18n("Search")
                placeholderText: i18n("Search")
                Layout.fillWidth: true
                Layout.minimumHeight: 48
                font.pixelSize: 16
            }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 8
                model: picker.accounts.filter(a => (a.name + " " + a.account).toLowerCase().includes(accountSearch.text.toLowerCase()))
                delegate: Controls.ItemDelegate {
                    required property var modelData
                    width: ListView.view.width
                    implicitHeight: 48
                    text: modelData.name
                    onClicked: { picker.set(modelData.account); picker.edited(); accountSheet.close() }
                }
            }
            Controls.Button {
                visible: accountSearch.text.trim() !== "" && !picker.accounts.some(a => a.name === accountSearch.text || a.account === accountSearch.text)
                text: accountSearch.text
                Layout.fillWidth: true
                Layout.minimumHeight: 48
                onClicked: { picker.set(accountSearch.text.trim()); picker.edited(); accountSheet.close() }
            }
        }
    }
    }
    Controls.Label {
        visible: picker.error !== ""
        text: picker.error
        color: Kirigami.Theme.negativeTextColor
        wrapMode: Text.Wrap
        Layout.fillWidth: true
        Accessible.name: i18nc("%1 is the field, %2 what's wrong with it", "%1 problem: %2", picker.label, text)
    }
    Controls.Button {
        visible: picker.suggestion !== ""
        text: i18nc("%1 is the closest account to what was typed", "Use %1", picker.nameOf(picker.suggestion))
        icon.name: "checkmark"
        onClicked: {
            picker.set(picker.suggestion)
            picker.edited()
        }
    }
}
