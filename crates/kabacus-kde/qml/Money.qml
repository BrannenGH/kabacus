// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// An amount from the model ({text, exact, tone}). Balances are colour
// coded, red below zero and green above, with a slight dark outline so the
// colours stay readable on any background (design/tokens.json: balance*).
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Controls.Label {
    id: money

    property var amount: null
    // Colour by sign; off for amounts that aren't balances.
    property bool colored: true
    property bool outlined: colored
    // On a selected or highlighted row: the selection's text colour, no
    // sign colour (green on blue is 3.4:1).
    property bool highlighted: false

    readonly property var tokens: Theme.of(Kirigami.Theme.backgroundColor)
    readonly property string tone: amount ? amount.tone : "zero"

    text: amount ? amount.text : ""
    color: highlighted ? Kirigami.Theme.highlightedTextColor
        : !colored || tone === "zero" ? Kirigami.Theme.textColor
        : tone === "neg" ? tokens.balanceNegative : tokens.balancePositive
    style: outlined && colored && !highlighted && tone !== "zero" ? Text.Outline : Text.Normal
    styleColor: tokens.balanceOutline
    font.features: { "tnum": 1, "lnum": 1 }
    horizontalAlignment: Text.AlignRight
    Accessible.name: text
}
