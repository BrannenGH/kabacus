// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
import QtQuick.Window
QtObject {
    readonly property LoggingCategory diagnostics: LoggingCategory {
        name: "me.hallb.kabacus"
        defaultLogLevel: LoggingCategory.Info
    }
    property var windows: []
    property var lastActive: null
    function panelSnapshot(): string {
        const book = lastActive && lastActive.ready && lastActive.hasBooks ? lastActive
            : windows.find(w => w.ready && w.hasBooks)
        return JSON.stringify(book ? book.panelSnapshot() : { status: "closed" })
    }
    property bool restored: false
    property bool globalMenuAvailable: false
    function add(window) { if (windows.indexOf(window) < 0) windows = windows.concat([window]) }
    function remove(window) {
        windows = windows.filter(w => w !== window)
        if (lastActive === window) lastActive = null
    }
    function focus(source, except) {
        const other = windows.find(w => w !== except && w.bookSource && w.bookSource.kind === source.kind && (source.kind === "local" ? w.bookSource.path === source.path : w.bookSource.url === source.url))
        if (!other) return false
        other.show(); other.raise(); other.requestActivate(); return true
    }
    function quit() {
        const open = windows.slice()
        function next(index) {
            if (index >= open.length) { Qt.quit(); return }
            open[index].runFileAction(function() { next(index + 1) })
        }
        next(0)
    }
    function create(state) {
        const component = Qt.createComponent("Main.qml")
        if (component.status !== Component.Ready) {
            console.warn(diagnostics, "could not load desktop window component")
            return null
        }
        const window = component.createObject(null, { initialState: state || null }) as Window
        if (window) { add(window); window.show(); window.raise(); window.requestActivate() }
        else console.warn(diagnostics, "could not create desktop window")
        return window
    }
}
