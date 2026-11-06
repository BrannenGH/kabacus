// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
QtObject {
    property var windows: []
    property bool restored: false
    function add(window) { if (windows.indexOf(window) < 0) windows = windows.concat([window]) }
    function remove(window) { windows = windows.filter(w => w !== window) }
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
        if (component.status !== Component.Ready) { console.warn(component.errorString()); return null }
        const window = component.createObject(null, { initialState: state || null })
        if (window) { add(window); window.show(); window.raise(); window.requestActivate() }
        return window
    }
}
