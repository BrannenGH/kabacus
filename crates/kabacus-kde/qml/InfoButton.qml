// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// The ⓘ beside a figure, saying what it means: a tooltip when a mouse
// hovers over it, and a dialog when it's clicked or tapped, since a
// tooltip never shows on touch. The caller names it ("About Cash on
// Hand") as one whole message, so a translation can word it its own way.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Controls.ToolButton {
    id: button

    // The figure explained, the dialog's title.
    required property string topic
    // What it means, in a sentence or two.
    required property string explanation

    icon.name: "help-contextual"
    display: Controls.AbstractButton.IconOnly
    Accessible.name: text
    Accessible.description: explanation
    Controls.ToolTip.text: explanation
    Controls.ToolTip.visible: hovered && !dialog.visible
    Controls.ToolTip.delay: Kirigami.Units.toolTipDelay
    onClicked: dialog.open()

    Kirigami.PromptDialog {
        id: dialog
        // Centred on the window, not on the button.
        parent: Controls.Overlay.overlay
        title: button.topic
        subtitle: button.explanation
        standardButtons: Kirigami.Dialog.Close
    }
}
