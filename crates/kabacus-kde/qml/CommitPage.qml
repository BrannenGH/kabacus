// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// One revision's changes. A wrong change is answered with an ordinary,
// reviewed edit or void; there is no "revert", so history only grows.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "commit"
    title: commit ? commit.short : ""

    property var commit: null
    property Fetch fetch: Fetch {
        ledger: page.ledger
        view: page.commit ? "diff" : ""
        args: ({ hash: page.commit ? page.commit.hash : "" })
        owner: page
    }
    readonly property var lines: fetch.value || []
    readonly property string problem: fetch.problem

    required property var app
    readonly property var ledger: app.ledger
    readonly property bool hasTransaction: commit !== null && commit.transaction_id !== ""

    actions: [
        Kirigami.Action { text: i18n("Open transaction"); icon.name: "document-open"; visible: page.hasTransaction; enabled: page.app.canWrite; onTriggered: page.app.openEdit(page.commit.transaction_id, "edit") },
        Kirigami.Action { text: i18n("Void…"); icon.name: "edit-undo"; visible: page.hasTransaction; enabled: page.app.canWrite; onTriggered: page.app.openEdit(page.commit.transaction_id, "void") }
    ]

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Controls.Label { text: page.commit ? page.commit.summary : ""; font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Controls.Label { text: page.commit ? i18nc("%1 is who made a change, %2 when", "%1 · %2", page.commit.actor_label, page.commit.date) : ""; opacity: 0.7 }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.problem !== ""
            type: Kirigami.MessageType.Error
            text: page.problem
        }

        DiffView {
            lines: page.lines
            Layout.fillWidth: true
        }
    }
}
