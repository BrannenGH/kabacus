// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// One screen's data from Ledger.load, without blocking: the last reply at
// once, the current one when it arrives, and again whenever the books
// change (revision). For the same view and args, the last good value stays
// while a newer one loads or if loading fails; new args with nothing
// cached, or a sign-out or server change, clear it. An identical reply
// leaves `value` alone, so lists keep their delegates and scroll position.
pragma ComponentBehavior: Bound
import QtQuick

QtObject {
    id: fetch

    required property var ledger
    // Empty: load nothing (yet).
    property string view: ""
    // The view's arguments; changing them loads again.
    property var args: ({})
    // A Kirigami.ScrollablePage: its spinner shows while this loads, and
    // pulling it down (touch) refreshes.
    property var owner: null

    // The last good value (the reply's `ok`), and the last problem.
    property var value: null
    property string problem: ""
    property bool pending: false
    readonly property bool busy: pending || ledger.refreshing

    readonly property string argsText: JSON.stringify(args)
    readonly property string key: view + "|" + argsText
    property string lastOk: ""
    property bool ready: false

    function reload() {
        if (view === "") {
            pending = false
            return
        }
        const r = JSON.parse(ledger.load(view, argsText))
        pending = r.pending
        if (r.reply === null) {
            // Never loaded, or forgotten (signed out, another server).
            value = null
            lastOk = ""
        } else {
            take(r.reply)
        }
    }

    function take(reply) {
        if (reply.error !== undefined) {
            // Pages show `problem` where there's nothing to show; over the
            // last good value, say it couldn't be updated.
            if (reply.error !== problem && value !== null && owner && owner.app)
                owner.app.showPassiveNotification(i18nc("%1 is a screen, %2 the problem", "Couldn't update %1: %2", owner.title, reply.error))
            problem = reply.error
            return
        }
        problem = ""
        // Compared as JS sees it, so a cached and a fresh copy match.
        const text = JSON.stringify(reply.ok)
        if (text === lastOk) return
        lastOk = text
        value = reply.ok
    }

    onKeyChanged: if (ready) reload()
    onBusyChanged: if (owner) owner.refreshing = busy
    onOwnerChanged: if (owner) owner.refreshing = busy
    Component.onCompleted: {
        if (owner) owner.supportsRefreshing = true
        ready = true
        reload()
    }

    property Connections connections: Connections {
        target: fetch.ledger
        function onLoaded(key: string, reply: string) {
            if (key !== fetch.key) return
            fetch.pending = false
            fetch.take(JSON.parse(reply))
        }
        function onRevisionChanged() { fetch.reload() }
    }

    property Connections pull: Connections {
        target: fetch.owner
        ignoreUnknownSignals: true
        function onRefreshingChanged() {
            if (fetch.owner.refreshing && !fetch.busy) fetch.ledger.refresh()
        }
    }
}
