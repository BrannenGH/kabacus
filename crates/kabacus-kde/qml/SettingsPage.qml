// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "settings"
    title: i18n("Settings")
    globalToolBarStyle: app.narrow ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    required property var app
    readonly property var preferences: JSON.parse(app.ledger.workspaceJson())
    readonly property var about: JSON.parse(app.ledger.aboutJson())
    property string problem: ""
    function save() { problem = app.ledger.savePreferences(reopen.checked, automation.checked, port.value) }
    Kirigami.FormLayout {
        Controls.CheckBox { id: reopen; Kirigami.FormData.label: i18n("When Kabacus starts"); text: i18n("Reopen my windows"); checked: page.preferences.reopen; onToggled: page.save() }
        Controls.Label { text: i18n("Show Welcome when this is off."); wrapMode: Text.Wrap; Layout.fillWidth: true }
        Controls.CheckBox { id: automation; Kirigami.FormData.label: i18n("Automation"); text: i18n("Let agents drive this window"); checked: page.preferences.automation; enabled: page.app.hasBooks; onToggled: page.save() }
        Controls.SpinBox { id: port; Kirigami.FormData.label: i18n("Control port"); from: 1; to: 65535; value: page.preferences.control_port || 7979; enabled: automation.checked; onValueModified: page.save() }
        Controls.Label { text: page.problem; visible: text !== ""; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Kirigami.Separator { Kirigami.FormData.isSection: true }
        Controls.Label { Kirigami.FormData.label: i18n("Account"); text: page.app.auth.username || i18n("On this computer"); wrapMode: Text.Wrap; Layout.fillWidth: true }
        Controls.Button { Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; text: i18n("Sign out"); visible: page.app.auth.status === "signed_in"; onClicked: page.app.signOut() }
        Kirigami.Separator { Kirigami.FormData.isSection: true }
        Controls.Label { Kirigami.FormData.label: i18n("About"); text: "Kabacus " + (page.about.server_version || ""); Layout.fillWidth: true }
        Controls.Button { Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight; text: i18n("Open the source code"); flat: true; onClicked: Qt.openUrlExternally(page.about.source_url || "https://github.com/BrannenGH/kabacus") }
    }
}
