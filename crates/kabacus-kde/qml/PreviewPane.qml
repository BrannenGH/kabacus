// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// What a write will change, from the server's dry run. Shared by the
// Review page (narrow, medium) and the form's live preview (wide).
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: pane

    // writes::Preview, or null.
    property var preview: null
    property bool showJournal: true

    spacing: Kirigami.Units.largeSpacing

    Controls.Label {
        objectName: "previewSummary"
        visible: pane.preview !== null
        text: pane.preview ? pane.preview.summary : ""
        wrapMode: Text.Wrap
        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.1
        Layout.fillWidth: true
    }

    Kirigami.Heading {
        visible: pane.preview !== null && pane.preview.changes.length > 0
        level: 5
        text: i18n("What changes")
    }

    Repeater {
        model: pane.preview ? pane.preview.changes : []
        delegate: RowLayout {
            id: changeDelegate
            required property var modelData
            Layout.fillWidth: true
            Accessible.name: i18nc("%1 is an account, %2 and %3 its balance before and after", "%1 %2 to %3", modelData.name, modelData.before.text, modelData.after.text)
            Controls.Label {
                text: changeDelegate.modelData.name
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            Money { amount: changeDelegate.modelData.before; colored: false; opacity: 0.7 }
            // Before → after, pointing the way the language reads.
            Kirigami.Icon { source: (pane.LayoutMirroring.enabled ? "go-previous" : "go-next"); Layout.preferredWidth: Kirigami.Units.iconSizes.small; Layout.preferredHeight: Layout.preferredWidth }
            Money { amount: changeDelegate.modelData.after; font.bold: true }
        }
    }

    RowLayout {
        objectName: "cashOnHandChange"
        visible: pane.preview !== null && pane.preview.cash_on_hand !== null
        Layout.fillWidth: true
        Accessible.name: visible ? i18nc("%1 and %2 are amounts", "Cash on Hand %1 to %2", pane.preview.cash_on_hand.before.text,
            pane.preview.cash_on_hand.after.text) : ""
        Controls.Label { text: i18n("Cash on Hand"); font.bold: true; Layout.fillWidth: true }
        Money { amount: pane.preview && pane.preview.cash_on_hand ? pane.preview.cash_on_hand.before : null; colored: false; opacity: 0.7 }
        Kirigami.Icon { source: (pane.LayoutMirroring.enabled ? "go-previous" : "go-next"); Layout.preferredWidth: Kirigami.Units.iconSizes.small; Layout.preferredHeight: Layout.preferredWidth }
        Money { amount: pane.preview && pane.preview.cash_on_hand ? pane.preview.cash_on_hand.after : null; font.bold: true }
        Kirigami.Chip {
            closable: false
            checkable: false
            text: pane.preview && pane.preview.cash_on_hand ? pane.preview.cash_on_hand.delta.text : ""
        }
    }

    // What looks wrong but won't stop the save, as warnings: among the
    // ticked notes below they read as passed checks.
    Repeater {
        model: pane.preview ? pane.preview.warnings : []
        delegate: Kirigami.InlineMessage {
            required property var modelData
            objectName: "previewWarning"
            Layout.fillWidth: true
            visible: true
            type: Kirigami.MessageType.Warning
            text: modelData
            Accessible.name: i18nc("%1 is the warning's text", "Warning: %1", modelData)
        }
    }

    Repeater {
        model: pane.preview ? pane.preview.notes : []
        delegate: RowLayout {
            id: noteDelegate
            required property var modelData
            Layout.fillWidth: true
            Kirigami.Icon { source: "checkmark"; color: Kirigami.Theme.positiveTextColor; Layout.preferredWidth: Kirigami.Units.iconSizes.small; Layout.preferredHeight: Layout.preferredWidth }
            Controls.Label { text: noteDelegate.modelData; wrapMode: Text.Wrap; Layout.fillWidth: true }
        }
    }

    DiffView {
        visible: pane.showJournal && pane.preview !== null && pane.preview.journal.length > 0
        lines: pane.preview ? pane.preview.journal : []
        Layout.fillWidth: true
    }
}
