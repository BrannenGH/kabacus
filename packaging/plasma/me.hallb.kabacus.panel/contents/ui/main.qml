// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasmoid
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: widget

    property var snapshot: ({ status: "closed" })
    property var request: null
    readonly property string available: snapshot.available ?? ""
    readonly property string balanceLabel: i18nd("kabacus", "Available to spend")
    readonly property string statusText: {
        switch (snapshot.status) {
        case "closed": return i18nd("kabacus", "Open a book in Kabacus.")
        case "sign_in": return i18nd("kabacus", "Sign in")
        case "unavailable": return i18nd("kabacus", "Can't load Cash on Hand")
        default: return i18nd("kabacus", "Loading…")
        }
    }

    toolTipMainText: balanceLabel
    toolTipSubText: available || statusText
    preferredRepresentation: compactRepresentation

    function refresh(): void {
        if (request || !app.registered) return
        request = DBus.SessionBus.asyncCall({
            service: app.watchedService,
            path: "/Panel",
            iface: "me.hallb.kabacus.Panel",
            member: "Snapshot",
        })
        const reply = request
        reply.finished.connect(function() {
            if (widget.request !== reply) return
            widget.request = null
            widget.snapshot = reply.isError
                ? { status: "unavailable" }
                : JSON.parse(String(reply.value))
        })
    }

    DBus.DBusServiceWatcher {
        id: app
        watchedService: "me.hallb.kabacus.Panel"
        onRegisteredChanged: {
            if (registered) widget.refresh()
            else {
                widget.request = null
                widget.snapshot = { status: "closed" }
            }
        }
    }
    Timer {
        interval: widget.snapshot.status === "loading" ? 750 : 5000
        running: app.registered
        repeat: true
        triggeredOnStart: true
        onTriggered: widget.refresh()
    }
    onExpandedChanged: if (expanded) refresh()

    compactRepresentation: PlasmaComponents.ToolButton {
        readonly property bool vertical: Plasmoid.formFactor === PlasmaCore.Types.Vertical
        focusPolicy: Qt.StrongFocus
        icon.name: Plasmoid.icon
        text: widget.available
        display: vertical || text === "" ? PlasmaComponents.ToolButton.IconOnly : PlasmaComponents.ToolButton.TextBesideIcon
        Layout.minimumWidth: vertical ? 0 : implicitWidth
        Accessible.name: widget.available
            ? i18ndc("kabacus", "%1 is an amount", "Available to spend %1", widget.available)
            : widget.balanceLabel
        onClicked: widget.expanded = !widget.expanded
    }

    fullRepresentation: PlasmaComponents.ScrollView {
        id: popup
        Layout.preferredWidth: Kirigami.Units.gridUnit * 19
        Layout.preferredHeight: contentHeight + topPadding + bottomPadding
        contentWidth: availableWidth
        contentHeight: details.implicitHeight

        ColumnLayout {
            id: details
            width: popup.availableWidth
            spacing: Kirigami.Units.smallSpacing

            PlasmaComponents.Label {
                Layout.fillWidth: true
                visible: text !== ""
                text: widget.snapshot.book ?? ""
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }
            Kirigami.Heading {
                Layout.fillWidth: true
                level: 3
                text: widget.balanceLabel
                wrapMode: Text.Wrap
            }
            Kirigami.Heading {
                Layout.fillWidth: true
                level: 1
                text: widget.available
                visible: text !== ""
                wrapMode: Text.Wrap
            }
            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: widget.statusText
                visible: widget.available === ""
                wrapMode: Text.Wrap
            }
            Kirigami.Heading {
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.smallSpacing
                visible: widget.available !== ""
                level: 3
                text: i18nd("kabacus", "Transactions")
            }
            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: i18nd("kabacus", "No transactions")
                visible: widget.available !== "" && rows.count === 0
                wrapMode: Text.Wrap
            }
            Repeater {
                id: rows
                model: widget.snapshot.recent ?? []
                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        PlasmaComponents.Label {
                            Layout.fillWidth: true
                            text: modelData.payee
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                        }
                        PlasmaComponents.Label {
                            text: new Date(modelData.date + "T00:00:00").toLocaleDateString(Qt.locale(), Locale.ShortFormat)
                            font: Kirigami.Theme.smallFont
                        }
                    }
                    PlasmaComponents.Label {
                        text: modelData.amount
                        textFormat: Text.PlainText
                    }
                }
            }
        }
    }
}
