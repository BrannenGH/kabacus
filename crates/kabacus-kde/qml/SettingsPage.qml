// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Where the server is, and who's signed in. A full-
// window layer on every form factor, with the usual back button.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs as Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami


Kirigami.ScrollablePage {
    id: settingsPage
    objectName: "settings"
    title: i18n("Settings")

    required property var app
    readonly property var ledger: app.ledger

    readonly property var saved: JSON.parse(settingsPage.ledger.settingsJson())
    // The server's version, commit and source: {server_version, commit,
    // source_url} or {error}.
    readonly property var about: JSON.parse(settingsPage.ledger.aboutJson())
    // The last test or save: {status, message} or {error}.
    property var result: null

    function args() {
        return [remoteChoice.checked ? "remote" : "local",
            socketField.text, urlField.text, caField.text]
    }

    actions: [
        Kirigami.Action {
            text: i18n("Test connection")
            icon.name: "network-connect"
            onTriggered: settingsPage.result = JSON.parse(settingsPage.ledger.testConnection(...settingsPage.args()))
        },
        Kirigami.Action {
            text: i18n("Save")
            icon.name: "document-save"
            onTriggered: {
                const err = settingsPage.ledger.saveSettings(...settingsPage.args())
                if (err !== "") {
                    settingsPage.result = { error: err }
                    return
                }
                settingsPage.app.showPassiveNotification(i18n("Connection settings saved"))
                settingsPage.app.pageStack.layers.pop()
            }
        }
    ]

    header: Kirigami.InlineMessage {
        visible: settingsPage.result !== null
        position: Kirigami.InlineMessage.Position.Header
        text: {
            const r = settingsPage.result
            return r === null ? "" : (r.error || r.message)
        }
        type: {
            const r = settingsPage.result
            if (r === null || r.error) return Kirigami.MessageType.Error
            return r.status === "connected" ? Kirigami.MessageType.Positive : Kirigami.MessageType.Warning
        }
    }

    Kirigami.FormLayout {
        Controls.RadioButton {
            id: localChoice
            Kirigami.FormData.label: i18nc("@label", "Server:")
            text: i18n("On this computer")
            checked: settingsPage.saved.kind !== "remote"
            onToggled: settingsPage.result = null
        }
        Controls.RadioButton {
            id: remoteChoice
            text: i18n("On another computer (HTTPS)")
            checked: settingsPage.saved.kind === "remote"
            onToggled: settingsPage.result = null
        }
        Controls.TextField {
            id: socketField
            visible: localChoice.checked
            Accessible.name: i18nc("the server's unix socket", "Socket")
            Kirigami.FormData.label: i18nc("@label the server's unix socket", "Socket:")
            placeholderText: settingsPage.saved.defaultSocket
            text: settingsPage.saved.socket_path
            Layout.fillWidth: true
            // A long path shows from its start, whole on hover.
            Component.onCompleted: cursorPosition = 0
            onEditingFinished: cursorPosition = 0
            Controls.ToolTip.visible: hovered && (text || placeholderText) !== ""
            Controls.ToolTip.text: text !== "" ? text : placeholderText
        }
        Controls.TextField {
            id: urlField
            visible: remoteChoice.checked
            Accessible.name: i18n("Server address")
            Kirigami.FormData.label: i18nc("@label", "Server address:")
            placeholderText: "https://books.lan:8443"
            text: settingsPage.saved.server_url
            inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            Layout.fillWidth: true
        }
        RowLayout {
            visible: remoteChoice.checked
            Kirigami.FormData.label: i18nc("@label a private certificate authority", "Private CA:")
            Layout.fillWidth: true
            Controls.TextField {
                id: caField
                Accessible.name: i18nc("a private certificate authority", "Private CA")
                placeholderText: i18n("System certificates only")
                text: settingsPage.saved.private_ca
                Layout.fillWidth: true
            }
            Controls.Button {
                text: i18n("Choose…")
                icon.name: "document-open"
                onClicked: caDialog.open()
            }
        }
        Controls.Label {
            visible: remoteChoice.checked
            text: i18n("Only needed when the server's certificate comes from your own CA.")
            wrapMode: Text.Wrap
            opacity: 0.7
            Layout.fillWidth: true
        }

        // Who the app is signed in as, for the saved server.
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title the signed-in user", "Account")
        }
        Controls.Label {
            objectName: "accountState"
            Kirigami.FormData.label: i18nc("@label", "Signed in:")
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            text: {
                switch (settingsPage.app.auth.status) {
                case "off": return i18n("Not needed: authentication is off (local mode)")
                case "signed_in": return settingsPage.app.auth.username !== ""
                    ? i18nc("%1 is the user, %2 the sign-in provider", "%1 (%2)", settingsPage.app.auth.username, settingsPage.app.auth.issuer)
                    : i18nc("signed in; %1 is the sign-in provider", "Yes (%1)", settingsPage.app.auth.issuer)
                case "expired": return i18n("Session ended; sign in again")
                case "signed_out": return i18nc("not signed in", "No")
                default: return i18n("Unknown: can't reach the server")
                }
            }
        }
        Controls.Button {
            visible: settingsPage.app.needsSignIn
            enabled: !settingsPage.app.auth.signing_in
            text: settingsPage.app.auth.signing_in ? i18n("Waiting for the browser…") : i18n("Sign in")
            icon.name: "im-user-online"
            onClicked: settingsPage.app.signIn()
        }
        Controls.Button {
            visible: settingsPage.app.auth.status === "signed_in"
            text: i18n("Sign out")
            icon.name: "system-log-out"
            onClicked: settingsPage.app.signOut()
        }

        // What the server is running, and where its source is.
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title about the server", "About")
        }
        Controls.Label {
            objectName: "aboutUnavailable"
            visible: settingsPage.about.error !== undefined
            text: i18n("Can't reach the server to read its version")
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Controls.Label {
            objectName: "aboutVersion"
            visible: settingsPage.about.error === undefined
            Kirigami.FormData.label: i18nc("@label the server's version", "Server version:")
            text: settingsPage.about.server_version ?? ""
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Controls.Label {
            objectName: "aboutCommit"
            visible: settingsPage.about.error === undefined && (settingsPage.about.commit ?? "") !== ""
            Kirigami.FormData.label: i18nc("@label the commit the server was built from", "Commit:")
            text: settingsPage.about.commit ?? ""
            wrapMode: Text.WrapAnywhere
            Layout.fillWidth: true
        }
        Controls.Button {
            objectName: "aboutSource"
            visible: settingsPage.about.error === undefined && (settingsPage.about.source_url ?? "") !== ""
            Kirigami.FormData.label: i18nc("@label where the server's source code is", "Source code:")
            text: i18n("Open the source code")
            icon.name: "internet-services"
            Accessible.description: settingsPage.about.source_url ?? ""
            onClicked: Qt.openUrlExternally(settingsPage.about.source_url)
        }
        Controls.Label {
            objectName: "aboutSourceUrl"
            visible: settingsPage.about.error === undefined && (settingsPage.about.source_url ?? "") !== ""
            text: settingsPage.about.source_url ?? ""
            wrapMode: Text.WrapAnywhere
            opacity: 0.7
            Layout.fillWidth: true
        }
    }

    Dialogs.FileDialog {
        id: caDialog
        title: i18n("Choose the private CA")
        nameFilters: [i18n("Certificates (*.pem *.crt)"), i18n("All files (*)")]
        onAccepted: caField.text = decodeURIComponent(selectedFile.toString().replace(/^file:\/\//, ""))
    }
}
