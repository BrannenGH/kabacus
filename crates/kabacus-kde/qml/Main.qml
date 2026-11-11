// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// Kabacus: one convergent Kirigami UI for phone, tablet and desktop.
// Wide (≥ 1200 px): the drawer is pinned and pages sit side by side.
// Medium (tablets): the drawer collapses to an icon rail. Narrow (< 600 px): bottom
// tabs and a floating Add button; dialogs become pages.
pragma ComponentBehavior: Bound
import QtQuick
import QtQml.Models
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Qt.labs.platform as Labs
import QtQuick.Dialogs as Dialogs
import org.kde.kirigami as Kirigami
import me.hallb.kabacus

Kirigami.ApplicationWindow {
    id: root
    // The name people see; the binary and the application are "kabacus".
    title: bookSource ? bookSource.name + " — Kabacus" : "Kabacus"
    property var initialState: null
    property string windowId: initialState ? initialState.id : "window-" + Date.now() + "-" + Math.floor(Math.random() * 100000)
    property var bookSource: null
    property string selectedAccount: initialState ? initialState.account || "" : ""
    property string selectedRow: initialState ? initialState.selected || "" : ""
    property bool ready: false
    property bool allowClose: false
    property bool explicitClose: false
    property var recentBooks: []
    property var agent: ({})
    readonly property var connection: JSON.parse(ledger.settingsJson())
    readonly property bool hasBooks: bookSource !== null || connection.socket_path !== ""
    function remember() {
        if (!ready || !bookSource) return
        ledger.rememberWindow(JSON.stringify({ id: windowId, source: bookSource, page: current, account: selectedAccount, selected: selectedRow, width: Math.round(width), height: Math.round(height) }))
        recentBooks = JSON.parse(ledger.workspaceJson()).recent || []
    }
    function newWindow(source) {
        if (source && WindowManager.focus(source, root)) return
        WindowManager.create({ id: "window-" + Date.now(), source: source || null, page: "Overview", account: "", selected: "", width: 1280, height: 800 })
    }
    function openSource(source) {
        source = Object.assign({}, source)
        if (source.kind === "local") source.path = ledger.canonicalBooksPath(source.path)
        if (WindowManager.focus(source, root)) return
        let error = ""
        if (source.kind === "local") error = ledger.openBooks(source.path)
        else error = ledger.saveSettings("remote", "", source.url, source.private_ca || "")
        if (error !== "") { showPassiveNotification(error, "long"); return }
        if (source.kind === "local") { const info = JSON.parse(ledger.localBooksInfo()); if (info && info.name) source.name = info.name }
        bookSource = source
        current = "Overview"
        pageStack.clear(); pageStack.push(overviewPage)
        remember()
    }
    function newBooks() { saveDialog.mode = "new"; saveDialog.open() }
    function openBooks() { openDialog.open() }
    function saveCopy() { saveDialog.mode = "copy"; saveDialog.open() }
    function connectServer() { connectDialog.open() }
    function closeWindow() { runFileAction(function() { explicitClose = true; ledger.forgetWindow(windowId); root.close() }) }
    function runFileAction(action) {
        if (unsavedForm() !== null) { discardDialog.pendingAction = action; discardDialog.open() }
        else action()
    }
    function localPath(url) { return decodeURIComponent(url.toString().replace(/^file:\/\//, "")) }
    onCurrentChanged: remember()
    onSelectedAccountChanged: remember()
    onSelectedRowChanged: remember()
    onClosing: function(close) {
        if (!allowClose && unsavedForm() !== null) { close.accepted = false; runFileAction(function() { allowClose = true; root.close() }); return }
        remember()
        if (explicitClose || WindowManager.windows.length > 1) ledger.forgetWindow(windowId)
        WindowManager.remove(root)
    }
    // A language written right to left mirrors everything: layouts,
    // alignment, the drawers. The catalog says which way its language is
    // written, the same message translation.rs sets Qt's layout direction
    // from (qmllint doesn't know Qt.application.layoutDirection). Pages
    // read their inherited LayoutMirroring.enabled.
    readonly property bool rightToLeft: i18nc("Which way this language is written: translate as RTL if right to left, LTR otherwise", "LTR") === "RTL"
    LayoutMirroring.enabled: rightToLeft
    LayoutMirroring.childrenInherit: true
    // `--size=WxH` opens the window at that size (the GUI e2e tests run the
    // phone, tablet and desktop sizes this way).
    readonly property var requestedSize: {
        const arg = Qt.application.arguments.find(a => a.startsWith("--size="))
        return arg ? arg.slice(7).split("x").map(Number) : null
    }
    width: requestedSize ? requestedSize[0] : 1280
    height: requestedSize ? requestedSize[1] : 800
    minimumWidth: 320

    property alias ledger: ledgerObject
    Ledger {
        id: ledgerObject
    }

    readonly property bool wide: width >= 1200
    readonly property bool narrow: width < 600
    // {status, username, issuer, signing_in, error?}.
    readonly property var auth: JSON.parse(ledger.authJson)
    readonly property bool needsSignIn: auth.status === "signed_out" || auth.status === "expired"
    // Can't reach the server. Pages keep what they last loaded,
    // dimmed, and every write is off until it's back.
    readonly property bool serverBooks: hasBooks && (!bookSource || bookSource.kind === "remote")
    readonly property bool offline: serverBooks && ledger.error !== "" && !needsSignIn
    readonly property bool canWrite: hasBooks && ledger.error === "" && !needsSignIn
    // The current destination: a key of `pages`, never the label shown
    // for it, which is translated.
    property string current: "Overview"
    // Settings is a layer over the current page: only it is highlighted
    // in the sidebar then.
    readonly property bool settingsOpen: pageStack.layers.currentItem !== null && pageStack.layers.currentItem.objectName === "settings"

    Component.onCompleted: Qt.callLater(initialize)
    function initialize() {
        WindowManager.add(root)
        const workspace = JSON.parse(ledger.workspaceJson())
        recentBooks = workspace.recent || []
        let state = initialState
        if (!WindowManager.restored && !initialState) {
            WindowManager.restored = true
            const args = Qt.application.arguments
            const chosen = args.find(a => a.endsWith(".kabacus"))
            if (chosen) state = { id: windowId, source: { kind: "local", path: chosen, name: chosen.split("/").pop().replace(/\.kabacus$/, "") }, page: "Overview" }
            else if (workspace.reopen && workspace.windows.length) {
                state = workspace.windows[0]
                for (let i = 1; i < workspace.windows.length; i++) WindowManager.create(workspace.windows[i])
            }
        }
        if (state && state.source) {
            windowId = state.id
            width = state.width || width; height = state.height || height
            openSource(state.source)
            selectedAccount = state.account || ""; selectedRow = state.selected || ""
            if (state.page && state.page !== "Overview" && pages[state.page]) { leaveFor(state.page, state.page === "Accounts" ? { open: selectedAccount } : {}) }
            showPassiveNotification(i18n("Reopened where you left off"))
        } else if (hasBooks) { pageStack.clear(); pageStack.push(overviewPage) }
        ready = true; remember()
        const control = Qt.application.arguments.findIndex(a => a === "--control")
        if (hasBooks && ((state && state.automation) || control >= 0)) {
            const error = ledger.savePreferences(workspace.reopen, true, control >= 0 ? Number(Qt.application.arguments[control + 1]) : (state ? state.control_port : 7979))
            if (error) showPassiveNotification(error)
        }
        if (hasBooks) ledger.refresh()
    }

    // Sign in through the system browser.
    function signIn() {
        const r = JSON.parse(ledger.beginSignIn())
        if (r.error) showPassiveNotification(r.error)
        else Qt.openUrlExternally(r.url)
    }

    function signOut() {
        const err = ledger.signOut()
        showPassiveNotification(err !== "" ? err : i18n("Signed out"))
    }

    readonly property var pages: ({
        "Overview": overviewPage, "Accounts": accountsPage, "Funds": fundsPage,
        "Loans": loansPage, "Cards": cardsPage, "People": peoplePage, "Reports": reportsPage,
        "Activity": activityPage, "More": morePage,
    })
    // What each destination is called, wherever it's shown (the drawer,
    // the bottom tabs, More, the Go menu, the page's title). The keys
    // are the books' own terms; people see Transactions for the accounts
    // and their registers, Reserves for funds (card reserves are reserves
    // for paying a card), and Credit for the credit cards.
    readonly property var labels: ({
        "Overview": i18n("Overview"),
        "Accounts": i18nc("destination: the accounts and their transactions", "Transactions"),
        "Funds": i18nc("destination: money set aside for later", "Reserves"),
        "Loans": i18nc("destination: loans and mortgages", "Loans"),
        "Cards": i18nc("destination: credit cards", "Credit"),
        "People": i18n("People"),
        "Reports": i18n("Reports"),
        "Activity": i18n("Activity"),
    })

    // An open form with something typed into it, or null.
    function unsavedForm() {
        for (let i = pageStack.layers.depth - 1; i >= 1; i--) {
            const form = pageStack.layers.get(i) as TransactionForm
            if (form && form.dirty) return form
        }
        return null
    }

    // Switch to a top-level destination. Going elsewhere closes any open
    // form, so one with something typed into it asks first.
    function go(name, props) {
        if (unsavedForm() !== null) {
            discardDialog.pendingAction = null
            discardDialog.destination = name
            discardDialog.destinationProps = props || {}
            discardDialog.open()
            return
        }
        leaveFor(name, props)
    }

    function leaveFor(name, props) {
        current = name
        ledger.check()
        while (pageStack.layers.depth > 1) pageStack.layers.pop()
        pageStack.clear()
        pageStack.push(pages[name], props || {})
    }

    // An account's register beside the list (wide) or over it.
    function showRegister(account, name) {
        selectedAccount = account
        ledger.check()
        while (pageStack.depth > 1) pageStack.pop()
        pageStack.push(registerPage, { account: account, accountName: name })
    }

    // Reconcile is something done to one account, so it opens from that
    // account's register rather than being a destination of its own.
    function openReconcile(account, name) {
        pageStack.layers.push(reconcilePage, { account: account, accountName: name })
    }

    function openSettings() {
        if (settingsOpen) return
        const open = () => {
            while (pageStack.layers.depth > 1) pageStack.layers.pop()
            pageStack.layers.push(settingsPage)
        }
        if (unsavedForm() !== null) {
            discardDialog.pendingAction = open
            discardDialog.open()
            return
        }
        open()
    }

    // The Add chooser, then the form for what happened. From a
    // register, its `account` is preset in the form.
    function openChooser(account) {
        if (!canWrite) return
        chooser.account = account || ""
        chooser.open()
    }

    function openForm(kind, preset) {
        chooser.close()
        pageStack.layers.push(formPage, { kind: kind, preset: preset || {} })
    }

    // Clicking a transaction edits it in place; the form offers
    // void too.
    function openEdit(id, mode) {
        const d = JSON.parse(ledger.editDraft(id))
        if (d.error) {
            showPassiveNotification(d.error)
            return
        }
        openForm("edit", { id: id, mode: mode || "edit", date: d.date, payee: d.payee,
            memo: d.memo, from: d.from, to: d.to, amount: d.amount, postings: d.postings })
    }

    // The preview's Save: commit against the preview's head, off the GUI
    // thread (ledger.saving shows it's under way). Once saved, the form
    // closes; otherwise `failed` gets the error object for the page that
    // asked.
    property var saveFailed: null
    function save(draft, head, failed) {
        if (ledger.saving) return
        saveFailed = failed || null
        ledger.startCommit(JSON.stringify(draft), head)
    }

    Connections {
        target: root.ledger
        function onCommitted(reply: string) {
            const r = JSON.parse(reply)
            const failed = root.saveFailed
            root.saveFailed = null
            if (r.ok) {
                while (root.pageStack.layers.depth > 1) root.pageStack.layers.pop()
                root.showPassiveNotification(i18nc("%1 is the revision number", "Saved as revision %1", r.ok.head), "long", i18n("View"), function() { root.go("Activity", {selected: r.ok.head}) })
                return
            }
            if (r.error.kind === "conflict") {
                conflict.who = r.error.who
                conflict.what = r.error.what
                conflict.open()
            }
            if (failed) failed(r.error)
        }
    }

    globalDrawer: Kirigami.GlobalDrawer {
        id: drawer
        title: ""
        topContent: Controls.ToolButton { text: root.bookSource ? root.bookSource.name : "Kabacus"; icon.name: "me.hallb.kabacus"; onClicked: filePopup.open() }
        visible: root.hasBooks
        modal: root.narrow
        drawerOpen: !root.narrow
        handleVisible: false
        collapsible: true
        collapsed: !root.wide
        showHeaderWhenCollapsed: true
        footer: ColumnLayout {
            width: parent.width
            Controls.ToolButton { text: i18n("Settings"); icon.name: "configure"; Layout.fillWidth: true; onClicked: root.openSettings() }
            Controls.Label { text: root.bookSource && root.bookSource.kind === "remote" ? root.bookSource.url : i18n("On this computer"); font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
            Controls.Label { text: root.bookSource && root.bookSource.kind === "local" ? root.bookSource.path.substring(0, root.bookSource.path.lastIndexOf("/")) : root.auth.username || ""; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true; font: Kirigami.Theme.smallFont }
            Controls.Label { text: i18n("All changes saved"); font: Kirigami.Theme.smallFont }
            Controls.Label { text: root.agent.agent ? i18n("%1 is working in these books", root.agent.agent) : ""; visible: text !== ""; wrapMode: Text.Wrap; Layout.fillWidth: true }
        }
        actions: [
            Kirigami.Action { text: root.labels.Overview; icon.name: "go-home"; checked: !root.settingsOpen && root.current === "Overview"; onTriggered: root.go("Overview") },
            Kirigami.Action { text: root.labels.Accounts; icon.name: "wallet-open"; checked: !root.settingsOpen && root.current === "Accounts"; onTriggered: root.go("Accounts") },
            Kirigami.Action { text: root.labels.Funds; icon.name: "folder-text"; checked: !root.settingsOpen && root.current === "Funds"; onTriggered: root.go("Funds") },
            Kirigami.Action { text: root.labels.Cards; icon.name: "view-credit-card"; checked: !root.settingsOpen && root.current === "Cards"; onTriggered: root.go("Cards") },
            Kirigami.Action { text: root.labels.Loans; icon.name: "view-bank"; checked: !root.settingsOpen && root.current === "Loans"; onTriggered: root.go("Loans") },
            Kirigami.Action { text: root.labels.People; icon.name: "system-users"; checked: !root.settingsOpen && root.current === "People"; onTriggered: root.go("People") },
            Kirigami.Action { text: root.labels.Reports; icon.name: "office-chart-bar"; checked: !root.settingsOpen && root.current === "Reports"; onTriggered: root.go("Reports") },
            Kirigami.Action { text: root.labels.Activity; icon.name: "view-history"; checked: !root.settingsOpen && root.current === "Activity"; onTriggered: root.go("Activity") },
            Kirigami.Action { separator: true },
            Kirigami.Action { id: settingsAction; text: i18n("Settings"); icon.name: "configure"; checkable: true; checked: root.settingsOpen; shortcut: StandardKey.Preferences; onTriggered: root.openSettings() }
        ]
    }

    // KDE's menu bar, desktop only: everything in it is also in the drawer
    // or a toolbar. Only Quit has a shortcut here, since the pages' own
    // (Ctrl+N, F5) would be ambiguous if the menu bar had them too.
    // With a global menu (Plasma's applet or panel menu) it is exported
    // there, as a native menu bar; otherwise it's drawn in the window.
    readonly property bool desktopMenus: !root.narrow && !Kirigami.Settings.isMobile && !Kirigami.Settings.tabletMode
    readonly property bool globalMenu: desktopMenus && WindowManager.globalMenuAvailable && Kirigami.Settings.hasPlatformMenuBar

    // Only hide the local menu when Qt can provide a native menu bar.
    Loader {
        active: root.globalMenu
        sourceComponent: Labs.MenuBar {
            window: root
            Labs.Menu {
                title: i18nc("@title:menu", "File")
                Labs.MenuItem { text: i18n("New Window"); shortcut: root.globalMenu ? "Ctrl+Shift+N" : ""; onTriggered: root.newWindow() }
                Labs.MenuItem { text: i18n("New Books…"); onTriggered: root.runFileAction(root.newBooks) }
                Labs.MenuItem { text: i18n("Open Books…"); shortcut: root.globalMenu ? "Ctrl+O" : ""; onTriggered: root.runFileAction(root.openBooks) }
                Labs.Menu {
                    id: nativeRecentMenu
                    title: i18n("Open Recent")
                    Instantiator {
                        model: root.recentBooks
                        delegate: Labs.MenuItem { required property var modelData; text: modelData.name; onTriggered: root.openSource(modelData) }
                        onObjectAdded: function(index, object) { nativeRecentMenu.insertItem(index, object) }
                        onObjectRemoved: function(index, object) { nativeRecentMenu.removeItem(object) }
                    }
                }
                Labs.MenuItem { text: i18n("Save a Copy As…"); shortcut: root.globalMenu ? "Ctrl+Shift+S" : ""; onTriggered: root.saveCopy() }
                Labs.MenuItem { text: i18n("Close Window"); shortcut: root.globalMenu ? "Ctrl+Shift+W" : ""; onTriggered: root.closeWindow() }
                Labs.MenuSeparator {}
                Labs.MenuItem { text: i18n("Connect to a Server…"); onTriggered: root.runFileAction(root.connectServer) }
                Labs.MenuItem { text: i18n("Quit"); shortcut: root.globalMenu ? "Ctrl+Q" : ""; onTriggered: WindowManager.quit() }
            }
            Labs.Menu {
                title: i18nc("@title:menu", "Go")
                enabled: root.hasBooks
                Labs.MenuItem { text: root.labels.Overview; icon.name: "go-home"; onTriggered: root.go("Overview") }
                Labs.MenuItem { text: root.labels.Accounts; icon.name: "wallet-open"; onTriggered: root.go("Accounts") }
                Labs.MenuItem { text: root.labels.Funds; icon.name: "folder-text"; onTriggered: root.go("Funds") }
                Labs.MenuItem { text: root.labels.Cards; icon.name: "view-credit-card"; onTriggered: root.go("Cards") }
                Labs.MenuItem { text: root.labels.Loans; icon.name: "view-bank"; onTriggered: root.go("Loans") }
                Labs.MenuItem { text: root.labels.People; icon.name: "system-users"; onTriggered: root.go("People") }
                Labs.MenuItem { text: root.labels.Reports; icon.name: "office-chart-bar"; onTriggered: root.go("Reports") }
                Labs.MenuItem { text: root.labels.Activity; icon.name: "view-history"; onTriggered: root.go("Activity") }
            }
            Labs.Menu {
                title: i18n("Settings")
                Labs.MenuItem { text: i18n("Configure Kabacus…"); icon.name: "configure"; onTriggered: root.openSettings() }
            }
        }
    }

    menuBar: Loader {
        active: root.desktopMenus && !root.globalMenu
        visible: active
        sourceComponent: Controls.MenuBar {
            Controls.Menu {
                title: i18nc("@title:menu", "File")
                    Controls.MenuItem { text: i18n("New Window"); onTriggered: root.newWindow() }
                Controls.MenuItem { text: i18n("New Books…"); onTriggered: root.runFileAction(root.newBooks) }
                Controls.MenuItem { text: i18n("Open Books…"); onTriggered: root.runFileAction(root.openBooks) }
                Controls.Menu {
                    id: recentMenu
                    title: i18n("Open Recent")
                    Instantiator {
                        model: root.recentBooks
                        delegate: Controls.MenuItem { required property var modelData; text: modelData.name; onTriggered: root.openSource(modelData) }
                        onObjectAdded: function(index, object) { recentMenu.insertItem(index, object) }
                        onObjectRemoved: function(index, object) { recentMenu.removeItem(object) }
                    }
                }
                Controls.MenuItem { text: i18n("Save a Copy As…"); onTriggered: root.saveCopy() }
                Controls.MenuItem { text: i18n("Close Window"); onTriggered: root.closeWindow() }
                Controls.MenuSeparator {}
                Controls.MenuItem { text: i18n("Connect to a Server…"); onTriggered: root.runFileAction(root.connectServer) }
                Controls.MenuItem { text: i18n("Quit"); onTriggered: WindowManager.quit() }
            }
            Controls.Menu {
                title: i18nc("@title:menu", "Go")
            enabled: root.hasBooks
                Controls.MenuItem { text: root.labels.Overview; icon.name: "go-home"; onTriggered: root.go("Overview") }
                Controls.MenuItem { text: root.labels.Accounts; icon.name: "wallet-open"; onTriggered: root.go("Accounts") }
                Controls.MenuItem { text: root.labels.Funds; icon.name: "folder-text"; onTriggered: root.go("Funds") }
                Controls.MenuItem { text: root.labels.Cards; icon.name: "view-credit-card"; onTriggered: root.go("Cards") }
                Controls.MenuItem { text: root.labels.Loans; icon.name: "view-bank"; onTriggered: root.go("Loans") }
                Controls.MenuItem { text: root.labels.People; icon.name: "system-users"; onTriggered: root.go("People") }
                Controls.MenuItem { text: root.labels.Reports; icon.name: "office-chart-bar"; onTriggered: root.go("Reports") }
                Controls.MenuItem { text: root.labels.Activity; icon.name: "view-history"; onTriggered: root.go("Activity") }
            }
            Controls.Menu {
                title: i18n("Settings")
                Controls.MenuItem { text: i18n("Configure Kabacus…"); icon.name: "configure"; onTriggered: root.openSettings() }
            }
        }
    }

    // On a phone, bottom tabs; Credit, Loans, Reports, Activity and
    // Settings are under More.
    footer: Kirigami.NavigationTabBar {
        id: tabBar
        visible: root.narrow
        // The drawer leaves room for the footer even when it's hidden.
        height: visible ? implicitHeight : 0
        actions: [
            Kirigami.Action { text: root.labels.Overview; icon.name: "go-home"; checked: root.current === "Overview"; onTriggered: root.go("Overview") },
            Kirigami.Action { text: root.labels.Accounts; icon.name: "wallet-open"; checked: root.current === "Accounts"; onTriggered: root.go("Accounts") },
            Kirigami.Action { text: root.labels.Funds; icon.name: "folder-text"; checked: root.current === "Funds"; onTriggered: root.go("Funds") },
            Kirigami.Action { text: root.labels.People; icon.name: "system-users"; checked: root.current === "People"; onTriggered: root.go("People") },
            Kirigami.Action {
                text: i18nc("the other destinations", "More"); icon.name: "overflow-menu"
                checked: ["More", "Cards", "Loans", "Reports", "Activity"].includes(root.current)
                onTriggered: root.go("More")
            }
        ]
    }

    // The app's state, above every page.
    // Above the pages, beside the drawer (the window's own header would
    // sit under it).
    ColumnLayout {
        id: banners
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        z: 5
        spacing: 0

        StatusBanner {
            id: authOff
            objectName: "authOff"
            visible: root.serverBooks && root.auth.status === "off"
            text: i18n("Authentication off (local mode)")
        }
        StatusBanner {
            id: signInBanner
            objectName: "signInBanner"
            visible: root.needsSignIn && !root.auth.signing_in
            text: {
                if (root.auth.error) return i18nc("%1 is why sign-in failed", "Sign-in didn't finish: %1", root.auth.error)
                return root.auth.status === "expired"
                    ? i18n("Your session has ended. Sign in again to see and change your books.")
                    : i18nc("%1 is the sign-in provider's address", "Sign in with %1 to see your books.", root.auth.issuer)
            }
            kind: root.auth.status === "expired" || root.auth.error ? "warning" : "info"
            actions: [
                Kirigami.Action {
                    text: root.auth.status === "expired" ? i18n("Sign in again") : i18n("Sign in")
                    icon.name: "im-user-online"
                    onTriggered: root.signIn()
                }
            ]
        }
        StatusBanner {
            id: authWarning
            objectName: "authWarning"
            visible: root.auth.warning !== undefined && root.auth.warning !== ""
            text: root.auth.warning || ""
            kind: "warning"
        }
        StatusBanner {
            id: signingIn
            objectName: "signingIn"
            visible: root.auth.signing_in === true
            text: i18n("Finish signing in in your browser…")
            actions: [
                Kirigami.Action { text: i18n("Cancel"); icon.name: "dialog-cancel"; onTriggered: root.ledger.cancelSignIn() }
            ]
        }
        StatusBanner {
            id: offlineBanner
            objectName: "offline"
            visible: root.offline
            kind: "error"
            // One plain sentence; the raw error is behind Details.
            text: i18nc("%1 is why the server can't be reached, as one sentence", "%1 Showing what was loaded; changes are off until it's back.", root.ledger.error)
            actions: [
                Kirigami.Action { text: i18n("Retry"); icon.name: "view-refresh"; onTriggered: root.ledger.refresh() },
                Kirigami.Action { text: i18n("Details"); icon.name: "documentinfo"; onTriggered: errorDetails.open() },
                Kirigami.Action { text: i18n("Connection settings"); icon.name: "configure"; onTriggered: root.openSettings() }
            ]
        }
    }

    Kirigami.PromptDialog {
        id: errorDetails
        title: i18n("Details")
        standardButtons: Kirigami.Dialog.Close
        Controls.TextArea {
            objectName: "errorDetails"
            Accessible.name: i18n("Error details")
            text: root.ledger.errorDetails
            readOnly: true
            selectByMouse: true
            wrapMode: Text.WrapAnywhere
        }
    }

    // What was last loaded stays, dimmed. Layers (Settings, forms)
    // stay bright: Settings is where the connection gets fixed.
    Binding {
        target: root.pageStack.columnView
        property: "opacity"
        value: root.offline ? 0.6 : 1
    }

    // Retry with backoff while offline; never queue writes.
    Timer {
        property int backoff: 2000
        interval: backoff
        running: root.offline
        repeat: true
        onTriggered: {
            root.ledger.refresh()
            backoff = Math.min(backoff * 2, 60000)
        }
        onRunningChanged: if (!running) backoff = 2000
    }

    // Pick up changes made elsewhere (a hand edit, MCP, another
    // device) on return to the window and every minute while it's active.
    onActiveChanged: if (active && hasBooks) ledger.check()
    Timer {
        interval: 60000
        running: root.active && root.hasBooks && !root.offline
        repeat: true
        onTriggered: root.ledger.check()
    }

    // The floating Add button on a phone.
    Controls.RoundButton {
        objectName: "fab"
        visible: root.narrow && root.canWrite && root.pageStack.layers.depth === 1
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: Kirigami.Units.gridUnit
        width: Kirigami.Units.gridUnit * 3
        height: width
        z: 10
        icon.name: "list-add"
        icon.width: Kirigami.Units.iconSizes.medium
        icon.height: Kirigami.Units.iconSizes.medium
        Kirigami.Theme.colorSet: Kirigami.Theme.Selection
        Kirigami.Theme.inherit: false
        highlighted: true
        text: i18n("Add")
        display: Controls.AbstractButton.IconOnly
        Accessible.name: i18n("Add")
        onClicked: {
            const register = root.pageStack.currentItem as RegisterPage
            root.openChooser(register ? register.account : "")
        }
    }

    pageStack.initialPage: welcomePage
    pageStack.anchors.topMargin: banners.height
    pageStack.defaultColumnWidth: Kirigami.Units.gridUnit * 20
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.ToolBar

    Component { id: welcomePage; WelcomePage { app: root } }
    Component { id: revisionPage; CommitPage { app: root } }
    Component { id: overviewPage; OverviewPage { app: root } }
    Component { id: accountsPage; AccountsPage { app: root } }
    Component { id: registerPage; RegisterPage { app: root } }
    Component { id: fundsPage; FundsPage { app: root } }
    Component { id: loansPage; LoansPage { app: root } }
    Component { id: cardsPage; CardsPage { app: root } }
    Component { id: peoplePage; PeoplePage { app: root } }
    Component { id: reportsPage; ReportsPage { app: root } }
    Component { id: reconcilePage; ReconcilePage { app: root } }
    Component { id: activityPage; ActivityPage { app: root } }
    Component { id: morePage; MorePage { app: root } }
    Component { id: settingsPage; SettingsPage { app: root } }
    Component { id: formPage; TransactionForm { app: root } }

    AddChooser {
        id: chooser
        app: root
    }

    // Leaving a form with unsaved changes.
    Kirigami.PromptDialog {
        id: discardDialog
        objectName: "discardDialog"
        property var pendingAction: null
        property string destination: ""
        property var destinationProps: ({})
        title: i18n("Discard this change?")
        subtitle: i18n("What you typed hasn't been saved.")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: i18n("Keep editing")
                icon.name: "document-edit"
                onTriggered: {
                    discardDialog.close()
                    // The tapped tab or drawer entry may have marked
                    // itself; point the highlight back at where we are.
                    const here = root.current
                    root.current = ""
                    root.current = here
                    settingsAction.checked = Qt.binding(() => root.settingsOpen)
                }
            },
            Kirigami.Action {
                text: i18n("Discard")
                icon.name: "edit-delete"
                onTriggered: {
                    discardDialog.close()
                    if (discardDialog.pendingAction) { const action = discardDialog.pendingAction; discardDialog.pendingAction = null; action() }
                    else root.leaveFor(discardDialog.destination, discardDialog.destinationProps)
                }
            }
        ]
    }

    // The books changed while you were reviewing.
    Kirigami.PromptDialog {
        id: conflict
        property string who: ""
        property string what: ""
        property Fetch latest: Fetch { ledger: root.ledger; view: conflict.visible ? "activity" : "" }
        title: i18n("Someone saved first")
        subtitle: what !== ""
            ? i18nc("%1 is who, %2 what they changed", "%1 saved a change while you were reviewing: %2. Yours hasn't been written.", who, what)
            : i18nc("%1 is who", "%1 saved a change while you were reviewing. Yours hasn't been written.", who)
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: i18n("See their change")
                icon.name: "view-history"
                enabled: conflict.latest.value !== null && conflict.latest.value.length > 0
                onTriggered: { conflict.close(); root.pageStack.layers.push(revisionPage, {commit:conflict.latest.value[0]}) }
            },
            Kirigami.Action {
                text: i18n("Keep editing")
                icon.name: "document-edit"
                onTriggered: conflict.close()
            },
            Kirigami.Action {
                text: i18n("Preview again")
                icon.name: "view-refresh"
                onTriggered: {
                    conflict.close()
                    const top = root.pageStack.layers.currentItem
                    const form = top as TransactionForm
                    const review = top as ReviewPage
                    if (form) form.reviewAgain()
                    else if (review) review.reviewAgain()
                }
            }
        ]
    }

    Shortcut { sequence: "Ctrl+Shift+N"; enabled: !root.globalMenu; onActivated: root.newWindow() }
    Shortcut { sequence: "Ctrl+O"; enabled: !root.globalMenu; onActivated: root.runFileAction(root.openBooks) }
    Shortcut { sequence: "Ctrl+Shift+S"; enabled: root.hasBooks && !root.globalMenu; onActivated: root.saveCopy() }
    Shortcut { sequence: "Ctrl+Shift+W"; enabled: !root.globalMenu; onActivated: root.closeWindow() }
    Shortcut { sequence: "Ctrl+Q"; enabled: !root.globalMenu; onActivated: WindowManager.quit() }
    Instantiator {
        model: ["Overview", "Accounts", "Funds", "Cards", "Loans", "People", "Reports", "Activity"]
        delegate: Shortcut { required property int index; required property string modelData; sequence: "Ctrl+" + (index + 1); enabled: root.hasBooks; onActivated: root.go(modelData) }
    }
    Controls.Menu {
        id: filePopup
        Controls.MenuItem { text: i18n("New Window"); onTriggered: root.newWindow() }
        Controls.MenuItem { text: i18n("New Books…"); onTriggered: root.runFileAction(root.newBooks) }
        Controls.MenuItem { text: i18n("Open Books…"); onTriggered: root.runFileAction(root.openBooks) }
        Controls.Menu { id: popupRecent; title: i18n("Open Recent")
            Instantiator { model: root.recentBooks
                delegate: Controls.MenuItem { required property var modelData; text: modelData.name; onTriggered: root.runFileAction(function() { root.openSource(modelData) }) }
                onObjectAdded: function(index, object) { popupRecent.insertItem(index, object) }
                onObjectRemoved: function(index, object) { popupRecent.removeItem(object) }
            }
        }
        Controls.MenuItem { text: i18n("Save a Copy As…"); enabled: root.hasBooks; onTriggered: root.saveCopy() }
        Controls.MenuItem { text: i18n("Close Window"); onTriggered: root.closeWindow() }
        Controls.MenuItem { text: i18n("Connect to a Server…"); onTriggered: root.runFileAction(root.connectServer) }
        Controls.MenuItem { text: i18n("Quit"); onTriggered: WindowManager.quit() }
    }
    Dialogs.FileDialog {
        id: openDialog
        title: i18n("Open Books…")
        nameFilters: ["Kabacus (*.kabacus)"]
        onAccepted: { const path = root.localPath(selectedFile); root.openSource({ kind: "local", path: path, name: path.split("/").pop().replace(/\.kabacus$/, "") }) }
    }
    Dialogs.FileDialog {
        id: saveDialog
        property string mode: "new"
        title: mode === "new" ? i18n("New Books…") : i18n("Save a Copy As…")
        fileMode: Dialogs.FileDialog.SaveFile
        defaultSuffix: "kabacus"
        nameFilters: ["Kabacus (*.kabacus)"]
        onAccepted: {
            if (mode === "copy") { const error = root.ledger.saveCopy(root.localPath(selectedFile)); if (error) root.showPassiveNotification(error) }
            else { setupDialog.path = root.localPath(selectedFile); setupDialog.open() }
        }
    }
    Kirigami.Dialog {
        id: setupDialog
        property string problem: ""
        property string path: ""
        title: i18n("New Books…")
        standardButtons: Kirigami.Dialog.NoButton
        preferredWidth: Kirigami.Units.gridUnit * 28
        ColumnLayout {
            Controls.Label { text: setupDialog.path; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true }
            Controls.Label { text: setupDialog.problem; visible: text !== ""; wrapMode: Text.Wrap; Layout.fillWidth: true }
            Kirigami.FormLayout {
                Controls.TextField { id: booksName; Accessible.name: i18n("Name"); Kirigami.FormData.label: i18n("Name"); text: "Household"; Layout.fillWidth: true }
                Controls.TextField { id: currency; Accessible.name: i18n("Currency"); Kirigami.FormData.label: i18n("Currency"); text: "USD"; Layout.fillWidth: true }
                Controls.TextField { id: firstAccount; Accessible.name: i18n("First account"); Kirigami.FormData.label: i18n("First account"); text: "assets:bank:checking"; Layout.fillWidth: true }
            }
            Controls.Button { text: i18n("Create books"); enabled: booksName.text !== "" && currency.text !== "" && firstAccount.text !== ""; onClicked: {
                const error = root.ledger.newBooks(setupDialog.path, booksName.text, currency.text, firstAccount.text)
                if (error) { setupDialog.problem = error; return }
                setupDialog.problem = ""; setupDialog.close(); root.bookSource = { kind: "local", path: setupDialog.path, name: booksName.text }; root.current = "Overview"; root.pageStack.clear(); root.pageStack.push(overviewPage); root.remember()
            } }
        }
    }
    Kirigami.Dialog {
        id: connectDialog
        title: i18n("Connect to a Server…")
        standardButtons: Kirigami.Dialog.NoButton
        preferredWidth: Kirigami.Units.gridUnit * 30
        ColumnLayout {
            Kirigami.FormLayout {
                Controls.TextField { id: serverAddress; Accessible.name: i18n("Server address"); Kirigami.FormData.label: i18n("Server address"); placeholderText: "https://books.example.org"; Layout.fillWidth: true }
                Controls.TextField { id: privateCa; Accessible.name: i18n("Private CA"); Kirigami.FormData.label: i18n("Private CA"); Layout.fillWidth: true }
            }
            Controls.Button { text: i18n("Test connection"); onClicked: { const result = JSON.parse(root.ledger.testConnection("remote", "", serverAddress.text, privateCa.text)); root.showPassiveNotification(result.error || result.message) } }
            Controls.Button { text: i18n("Connect"); enabled: serverAddress.text.startsWith("https://"); onClicked: { root.openSource({ kind: "remote", url: serverAddress.text, name: serverAddress.text.replace("https://", ""), private_ca: privateCa.text }); connectDialog.close() } }
        }
    }
    Timer {
        interval: 500; repeat: true; running: root.hasBooks
        onTriggered: { const next = JSON.parse(root.ledger.agentActivity()) || {}; if (next.expires_at !== root.agent.expires_at) root.ledger.check(); root.agent = next }
    }
}
