// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// The preview on narrow and medium widths: what the write will change, from the
// server's dry run, then Save on the same head. If an input changed
// since, the form previews again first.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "review"
    title: app.narrow ? i18nc("@action:button", "Review") : i18n("Review before saving")

    property var draft: null
    property var preview: null
    // The form to send field errors back to.
    property var form: null
    property var problem: null

    globalToolBarStyle: app.narrow ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    required property var app
    readonly property var ledger: app.ledger

    readonly property bool saving: ledger.saving
    readonly property bool canSave: preview !== null && !preview.unchanged && app.canWrite && !saving && !checking

    // "Review again" re-runs the dry run on the new head, off the GUI
    // thread like the form's.
    property int ticket: 0
    property bool checking: false
    function reviewAgain() {
        problem = null
        checking = true
        ticket = ledger.startPreview(JSON.stringify(draft))
    }

    Connections {
        target: page.ledger
        function onPreviewed(ticket: int, reply: string) {
            if (ticket !== page.ticket) return
            page.checking = false
            const r = JSON.parse(reply)
            if (r.ok) page.preview = r.ok
            else page.problem = r.error
        }
    }

    function saveIt() {
        app.save(draft, preview.head, err => {
            if (err.kind === "field" && page.form) {
                page.app.pageStack.layers.pop()
                page.form.showError(err)
            } else if (err.kind !== "conflict") {
                page.problem = err
            }
        })
    }

    actions: [
        Kirigami.Action { text: i18n("Back to edit"); icon.name: (page.LayoutMirroring.enabled ? "go-next" : "go-previous"); enabled: !page.saving; onTriggered: page.app.pageStack.layers.pop() },
        Kirigami.Action { text: page.saving ? i18n("Saving…") : i18n("Save"); icon.name: "document-save"; enabled: page.canSave; onTriggered: page.saveIt() }
    ]

    footer: Controls.ToolBar {
        visible: page.app.narrow
        height: visible ? implicitHeight : 0
        padding: 8
        Controls.Button {
            width: parent.width
            text: page.saving ? i18n("Saving…") : i18n("Save")
            icon.name: "document-save"
            enabled: page.canSave
            highlighted: true
            implicitHeight: 48
            Accessible.name: text
            onClicked: page.saveIt()
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        RowLayout {
            visible: page.saving || page.checking
            Layout.fillWidth: true
            Controls.BusyIndicator {
                running: parent.visible
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Layout.preferredWidth
            }
            Controls.Label {
                text: page.saving ? i18n("Saving…") : i18n("Checking with the server…")
                Layout.fillWidth: true
                Accessible.name: text
            }
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.problem !== null
            type: Kirigami.MessageType.Error
            text: page.problem === null ? ""
                : page.problem.kind === "assertion"
                    ? i18nc("%1 is an account, %2 the balance it's checked against, %3 the balance it would have",
                        "This would break a balance check: %1 should be %2 but would be %3. Check the date and amount; never move the date just to get past it.",
                        page.problem.account, page.problem.asserted, page.problem.calculated)
                    : page.problem.message || ""
            actions: page.problem !== null && page.problem.kind === "assertion" ? [backAction] : []
            Kirigami.Action {
                id: backAction
                text: i18n("Change the transaction")
                onTriggered: page.app.pageStack.layers.pop()
            }
        }

        PreviewPane {
            preview: page.preview
            Layout.fillWidth: true
        }


    }
}
