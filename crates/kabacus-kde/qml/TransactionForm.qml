// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// One form for every kind of change, named after
// what happened. Fields are stacked one under another on every width.
// Wide: the server's dry run updates beside the fields as you type
// and Save commits it. Narrow and medium: Review opens the preview
// first.
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "form"

    // spent, received, transfer, postings, card_purchase, set_aside,
    // pay_card, split, edit, fix_reserves, reconcile.
    property string kind: "spent"
    property var preset: ({})

    required property var app
    readonly property var ledger: app.ledger
    property Fetch pickerFetch: Fetch { ledger: page.ledger; view: "picker" }
    readonly property var accounts: pickerFetch.value ? pickerFetch.value.accounts : []
    // The commodity's decimal places, for checking amounts as typed.
    readonly property int places: pickerFetch.value ? pickerFetch.value.places : 2
    property var preview: null
    readonly property bool unchanged: preview !== null && preview.unchanged === true
    property string previewed: ""
    property var problem: null
    property var fieldErrors: ({})
    // Payee memory, asked for once typing settles.
    property string payeeQuery: ""
    property Fetch payeesFetch: Fetch {
        ledger: page.ledger
        view: page.payeeQuery !== "" ? "payees" : ""
        args: ({ query: page.payeeQuery })
    }
    readonly property var payeeHints: page.payeeQuery !== "" && payeesFetch.value
        ? payeesFetch.value.filter(h => h.payee !== payee.text).slice(0, 3) : []
    // Edit in place or void.
    property string mode: preset.mode || (kind === "set_aside" ? "aside" : kind === "edit" ? "edit" : "")
    // Free money or a fund.
    property string source: "free"

    readonly property bool live: app.wide
    readonly property var titles: ({
        spent: i18nc("@action what happened", "Spent"), received: i18nc("@action what happened", "Received"),
        transfer: i18n("Transfer"), postings: i18n("Postings"),
        card_purchase: i18n("Card purchase"), set_aside: i18n("Set money aside"), pay_card: i18n("Pay a card"),
        split: i18n("Split with someone"), edit: i18n("Edit transaction"), fix_reserves: i18n("Fix reserve lines"),
        reconcile: i18n("Reconcile"), account_type: i18n("Account type")
    })
    title: titles[kind] || i18n("Add")

    function of(kinds) { return accounts.filter(a => kinds.includes(a.kind)) }
    // Likely accounts first, then the rest, so nothing is out of reach.
    function ordered(kinds) { return of(kinds).concat(accounts.filter(a => !kinds.includes(a.kind) && a.kind !== "parent")) }
    function homeOf(account) {
        const a = accounts.find(x => x.account === account)
        return a ? (a.home !== "" ? a.home : a.account) : ""
    }

    readonly property var moneyAndCards: ordered(["money", "revolving-debt"])
    readonly property var categories: ordered(["expense"])
    readonly property var incomes: ordered(["income", "receivable"])
    readonly property var cards: of(["revolving-debt"])
    readonly property var money: of(["money"])
    readonly property var funds: of(["pre-allocation"])
    readonly property var people: of(["receivable"]).map(a => ({ account: a.account, name: a.account.split(":").pop(), kind: a.kind, home: "" }))
    // Only accounts with the same home are offered.
    readonly property var asideFrom: mode === "aside" ? money : funds
    readonly property var asideTo: {
        const home = homeOf(fromPicker.value)
        if (mode === "aside") return funds.filter(f => f.home === fromPicker.value)
        if (mode === "release") return money.filter(m => m.account === home)
        return funds.filter(f => f.home === home && f.account !== fromPicker.value)
    }

    function draft() {
        const base = { date: date.text, amount: amount.text }
        switch (kind) {
        case "account_type":
            return { kind: "account_type", account: preset.account, synthetic: accountType.currentIndex === 1 }
        case "card_purchase":
            return Object.assign(base, { kind: "card_purchase", payee: payee.text, memo: memo.text,
                card: cardPicker.value, category: toPicker.value,
                source: source === "free" ? "free" : fundPicker.value })
        case "set_aside":
            return Object.assign(base, { kind: "set_aside", from: fromPicker.value, to: toPicker.value, description: memo.text })
        case "pay_card":
            return Object.assign(base, { kind: "pay_card", card: cardPicker.value })
        case "split":
            return { kind: "split", date: date.text, payee: payee.text, memo: memo.text, paid_with: fromPicker.value,
                category: toPicker.value, person: personPicker.value.split(":").pop(), total: amount.text,
                their_share: theirShare.text }
        case "edit":
            if (mode === "void") return { kind: "void", id: preset.id }
            return Object.assign(base, { kind: "edit", id: preset.id,
                payee: payee.text, memo: memo.text, from: fromPicker.value, to: toPicker.value })
        case "fix_reserves":
            return { kind: "fix_reserves", card: preset.card, ids: preset.ids || [],
                source: source === "free" ? "free" : fundPicker.value }
        case "reconcile":
            return { kind: "reconcile", account: preset.account, date: preset.date, balance: preset.balance }
        default:
            return Object.assign(base, { kind: "simple", payee: payee.text, memo: memo.text,
                from: fromPicker.value, to: toPicker.value })
        }
    }

    // A form error: under its field when it has one, else above.
    function showError(err) {
        const errors = {}
        problem = null
        // Only this error shows: an earlier one may be fixed by now.
        for (const p of [fromPicker, toPicker, cardPicker, fundPicker, personPicker]) {
            p.error = ""
            p.suggestion = ""
        }
        if (err.kind === "field") {
            errors[err.field] = err.message
            const picker = ({ from: fromPicker, to: toPicker, category: toPicker, card: cardPicker,
                paid_with: fromPicker, source: fundPicker, person: personPicker })[err.field]
            if (picker) {
                picker.error = err.message
                picker.suggestion = err.suggestion || ""
            }
        } else if (err.kind !== "conflict") {
            problem = err
        }
        fieldErrors = errors
    }

    function clearErrors() {
        fieldErrors = ({})
        problem = null
        for (const p of [fromPicker, toPicker, cardPicker, fundPicker, personPicker]) {
            p.error = ""
            p.suggestion = ""
        }
    }

    // The dry run runs off the GUI thread (it takes seconds on big
    // books, and blocking on it froze typing): `previewed` brings it.
    property int previewTicket: 0
    property bool previewing: false
    // The draft the preview under way is for, and what to do once it's
    // in: "", "review" or "save".
    property string asked: ""
    property string afterPreview: ""
    readonly property bool saving: ledger.saving
    // The preview shown is for the draft as it stands.
    function current() { return preview !== null && previewed === JSON.stringify(draft()) }

    function runPreview(then) {
        asked = JSON.stringify(draft())
        // A live preview asked meanwhile keeps a pending Review or Save:
        // edited() drops it if the draft changed.
        if (then) afterPreview = then
        liveTimer.stop()
        previewing = true
        previewTicket = ledger.startPreview(asked)
    }

    Connections {
        target: page.ledger
        function onPreviewed(ticket: int, reply: string) {
            if (ticket !== page.previewTicket) return
            page.previewing = false
            const then = page.afterPreview
            page.afterPreview = ""
            // Changed since it was asked: a newer preview follows.
            if (page.asked !== JSON.stringify(page.draft())) return
            const r = JSON.parse(reply)
            if (!r.ok) {
                page.preview = null
                page.showError(r.error)
                return
            }
            page.clearErrors()
            page.preview = r.ok
            page.previewed = page.asked
            if (then === "review") page.pushReview()
            else if (then === "save") page.commit()
        }
    }

    // "Review again" from the conflict dialog.
    function reviewAgain() { runPreview() }

    function review() {
        if (current()) pushReview()
        else runPreview("review")
    }

    function pushReview() {
        app.pageStack.layers.push(reviewPage, { draft: draft(), preview: preview, form: page })
    }

    function saveNow() {
        if (saving) return
        // Never commit what wasn't shown: preview again if anything changed.
        if (current()) commit()
        else runPreview("save")
    }

    function commit() {
        if (unchanged) return
        app.save(draft(), preview.head, err => page.showError(err))
    }

    // Something was typed or chosen: leaving now asks first (Main.go).
    property bool dirty: false
    // Filling the presets sets the pickers, which report it as an edit.
    property bool presetting: false

    function edited() {
        if (!presetting) dirty = true
        preview = null
        // A Review or Save waiting on the preview would now act on what
        // wasn't shown.
        afterPreview = ""
        if (live) liveTimer.restart()
    }

    Timer {
        id: liveTimer
        interval: 600
        onTriggered: page.runPreview()
    }

    Timer {
        id: payeeTimer
        interval: 200
        onTriggered: page.payeeQuery = payee.text.trim()
    }

    Component.onCompleted: {
        const p = preset
        date.text = p.date || ledger.today()
        // Written to the books as the payee, so in the user's language.
        payee.text = p.payee || (kind === "transfer" ? i18nc("the payee a transfer is given", "Transfer") : "")
        memo.text = p.memo || ""
        amount.text = p.amount ? String(p.amount).replace("-", "") : ""
        if (kind === "set_aside" && p.mode) mode = p.mode
        if (kind === "account_type") accountType.currentIndex = p.synthetic ? 1 : 0
        if (accounts.length > 0) setUp()
        if (kind === "account_type") accountType.forceActiveFocus()
        else payee.forceActiveFocus()
    }

    // The pickers' presets, once the accounts are in (usually at once:
    // refresh preloads them).
    property bool setUpDone: false
    onAccountsChanged: if (!setUpDone && accounts.length > 0) setUp()
    function setUp() {
        setUpDone = true
        presetting = true
        // Only fill a picker the user hasn't touched yet.
        const fill = (picker, account) => { if (picker.value === "") picker.set(account) }
        const p = preset
        if (p.from) fill(fromPicker, p.from)
        else if (["spent", "transfer", "split", "postings"].includes(kind) && money.length > 0) fill(fromPicker, money[0].account)
        else if (kind === "set_aside" && asideFrom.length > 0) fill(fromPicker, asideFrom[0].account)
        if (p.to) fill(toPicker, p.to)
        else if (kind === "received" && money.length > 0) fill(toPicker, money[0].account)
        if (p.card) fill(cardPicker, p.card)
        else if (cards.length > 0) fill(cardPicker, cards[0].account)
        if (funds.length > 0) fill(fundPicker, funds[0].account)
        if (people.length > 0) fill(personPicker, people[0].account)
        presetting = false
        if (kind === "fix_reserves" || kind === "reconcile") runPreview()
    }

    actions: [
        // Not while saving: the save closes the form when it's done.
        Kirigami.Action { text: i18n("Cancel"); icon.name: "dialog-cancel"; enabled: !page.saving; onTriggered: page.app.runFileAction(function() { page.app.pageStack.layers.pop() }) },
        Kirigami.Action {
            text: i18nc("@option cancel a transaction with a reversal", "Void")
            icon.name: "edit-delete"
            visible: page.app.narrow && page.kind === "edit" && page.mode !== "void"
            enabled: !page.saving
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: { page.mode = "void"; page.edited() }
        },
        Kirigami.Action {
            text: page.live ? (page.saving ? i18n("Saving…") : i18n("Save")) : i18nc("@action:button", "Review")
            icon.name: page.live ? "document-save" : (page.LayoutMirroring.enabled ? "go-previous" : "go-next")
            // Nothing to save while the books already say this.
            enabled: page.app.canWrite && !page.saving && !(page.live && page.unchanged)
            shortcut: "Ctrl+Return"
            onTriggered: page.live ? page.saveNow() : page.review()
        }
    ]

    footer: Controls.ToolBar {
        visible: page.app.narrow
        height: visible ? implicitHeight : 0
        padding: 8
        Controls.Button {
            width: parent.width
            text: i18nc("@action:button", "Review")
            highlighted: true
            implicitHeight: 48
            enabled: page.app.canWrite && !page.previewing
            onClicked: page.review()
        }
    }

    RowLayout {
        spacing: Kirigami.Units.gridUnit

        ColumnLayout {
            Layout.alignment: Qt.AlignTop
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 20
            Layout.maximumWidth: page.live ? Kirigami.Units.gridUnit * 24 : -1

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.problem !== null
                type: Kirigami.MessageType.Error
                text: page.problem === null ? ""
                    : page.problem.kind === "assertion"
                        ? i18nc("%1 is an account, %2 the balance it's checked against, %3 the balance it would have",
                            "This would break a balance check: %1 should be %2 but would be %3. Check the date and amount.",
                            page.problem.account, page.problem.asserted, page.problem.calculated)
                        : page.problem.message || ""
            }

            // A press answered: the dry run Review waits on, or the save.
            RowLayout {
                visible: page.saving || (page.previewing && !page.live)
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

            // How a change to a recorded transaction is kept.
            ColumnLayout {
                visible: page.kind === "edit" && !page.app.narrow
                Layout.fillWidth: true
                Controls.ButtonGroup { id: modeGroup }
                Repeater {
                    model: [
                        ["edit", i18n("Edit in place"), i18n("Changes it where it is. History keeps the old version.")],
                        ["void", i18nc("@option cancel a transaction with a reversal", "Void"), i18n("Only the reversal: it didn't happen.")]
                    ]
                    delegate: ColumnLayout {
                        id: modeDelegate
                        required property var modelData
                        spacing: 0
                        Layout.fillWidth: true
                        Controls.RadioButton {
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                            id: modeRadio
                            text: modeDelegate.modelData[1]
                            checked: page.mode === modeDelegate.modelData[0]
                            Controls.ButtonGroup.group: modeGroup
                            Accessible.name: modeDelegate.modelData[1]
                            Accessible.description: modeDelegate.modelData[2]
                            onToggled: {
                                page.mode = modeDelegate.modelData[0]
                                page.edited()
                            }
                        }
                        Controls.Label {
                            text: modeDelegate.modelData[2]
                            wrapMode: Text.Wrap
                            opacity: 0.7
                            font: Kirigami.Theme.smallFont
                            Layout.fillWidth: true
                            Layout.leftMargin: modeRadio.indicator.width + modeRadio.spacing
                        }
                    }
                }
            }

            // Set aside, release, or move between funds.
            Kirigami.NavigationTabBar {
                visible: page.kind === "set_aside"
                Layout.fillWidth: true
                actions: [
                    Kirigami.Action { text: i18n("Set aside"); checked: page.mode === "aside"; onTriggered: { page.mode = "aside"; page.edited() } },
                    Kirigami.Action { text: i18nc("@action money out of a fund", "Release"); checked: page.mode === "release"; onTriggered: { page.mode = "release"; page.edited() } },
                    Kirigami.Action { text: i18nc("@action money between funds", "Move"); checked: page.mode === "move"; onTriggered: { page.mode = "move"; page.edited() } }
                ]
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true
                // Saving: what's shown is what's written.
                enabled: !page.saving && !(page.kind === "edit" && page.mode === "void")

                // Amount, then date: stacked, never side by side.
                Controls.TextField {
                    id: amount
                    font.pixelSize: page.app.narrow ? 32 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 56 : implicitHeight
                    visible: page.app.narrow && !["fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    Kirigami.FormData.label: page.kind === "split" ? i18nc("@label", "Total:") : i18nc("@label", "Amount:")
                    Accessible.name: i18n("Amount")
                    inputMethodHints: Qt.ImhFormattedNumbersOnly
                    onTextEdited: page.edited()
                    onAccepted: page.live ? page.saveNow() : page.review()
                }
                // The amount's problem shows as it's typed.
                Controls.Label {
                    readonly property string typed: amount.text === "" ? "" : page.ledger.checkAmount(amount.text, page.places, true)
                    visible: page.app.narrow && text !== ""
                    text: page.fieldErrors.amount || page.fieldErrors.total || typed
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Accessible.name: i18nc("%1 is the field, %2 what's wrong with it", "%1 problem: %2", i18n("Amount"), text)
                }
                Controls.Label {
                    visible: page.kind === "account_type"
                    font.bold: true
                    text: (page.accounts.find(a => a.account === page.preset.account) || {}).name || page.preset.account || ""
                }
                Controls.ComboBox {
                    font.pixelSize: page.app.narrow ? 16 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    id: accountType
                    visible: page.kind === "account_type"
                    Kirigami.FormData.label: i18n("Account type")
                    Accessible.name: i18n("Account type")
                    Accessible.description: currentText
                    model: [i18n("Real account"), i18n("Synthetic account")]
                    onActivated: page.edited()
                }
                AccountPicker {
                    id: cardPicker
                    visible: ["card_purchase", "pay_card"].includes(page.kind)
                    label: i18nc("a credit card", "Card")
                    accounts: page.cards
                    onEdited: page.edited()
                }
                Controls.TextField {
                    font.pixelSize: page.app.narrow ? 16 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    id: payee
                    visible: !["set_aside", "pay_card", "fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    Kirigami.FormData.label: page.kind === "split" ? i18nc("@label what a shared cost was for", "What:") : i18nc("@label", "Payee:")
                    Accessible.name: i18n("Payee")
                    onTextEdited: {
                        page.edited()
                        payeeTimer.restart()
                    }
                }
                // Payee memory.
                ColumnLayout {
                    visible: page.payeeHints.length > 0 && payee.visible
                    Layout.fillWidth: true
                    Repeater {
                        model: page.payeeHints
                        delegate: Controls.ItemDelegate {
                            required property var modelData
                            Layout.fillWidth: true
                            Accessible.name: modelData.payee
                            text: i18nc("%1 is a payee, %2 the category it usually has, %3 the amount it had last",
                                "%1 — usually %2, last %3", modelData.payee, modelData.category_name, modelData.last_amount.text)
                            icon.name: "view-history"
                            onClicked: {
                                payee.text = modelData.payee
                                if (["spent", "card_purchase", "split", "postings"].includes(page.kind)) toPicker.set(modelData.category)
                                if (amount.text === "") amount.text = modelData.last_amount.exact
                                payeeTimer.stop()
                                page.payeeQuery = ""
                                page.edited()
                            }
                        }
                    }
                }
                AccountPicker {
                    id: fromPicker
                    visible: !["card_purchase", "pay_card", "fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    label: page.kind === "split" ? i18n("Paid with") : i18nc("the account money comes from", "From")
                    accounts: page.kind === "set_aside" ? page.asideFrom
                        : page.kind === "received" ? page.incomes : page.moneyAndCards
                    onEdited: page.edited()
                }
                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    visible: page.kind === "spent" && page.cards.some(c => c.account === fromPicker.value)
                    type: Kirigami.MessageType.Warning
                    text: i18n("That's a card: record a card purchase so its reserve stays covered.")
                    actions: [
                        Kirigami.Action {
                            text: i18n("Card purchase")
                            onTriggered: {
                                page.kind = "card_purchase"
                                cardPicker.set(fromPicker.value)
                            }
                        }
                    ]
                }
                AccountPicker {
                    id: toPicker
                    visible: !["pay_card", "fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    label: ["spent", "card_purchase", "split"].includes(page.kind) ? i18n("Category") : i18nc("the account money goes to", "To")
                    accounts: page.kind === "set_aside" ? page.asideTo
                        : ["spent", "card_purchase", "split"].includes(page.kind) ? page.categories
                        : page.kind === "received" || page.kind === "transfer" ? page.ordered(["money"]) : page.accounts
                    onEdited: page.edited()
                }
                // Setting money aside under a name that isn't a fund yet
                // makes that fund, in the account the money comes from.
                Controls.Label {
                    readonly property bool newFund: page.kind === "set_aside" && page.mode === "aside"
                        && toPicker.value.trim() !== "" && !page.accounts.some(a => a.account === toPicker.value)
                    visible: newFund
                    text: newFund ? i18nc("%1 is a new reserve's name, %2 the account it goes in",
                        "%1 isn't a reserve yet. Saving creates it in %2.",
                        toPicker.value.trim().split(/\s+/).join(" "), fromPicker.nameOf(fromPicker.value)) : ""
                    wrapMode: Text.Wrap
                    // Wraps under the picker rather than widening the form
                    // (a wider form stacks its labels above the fields).
                    Layout.maximumWidth: toPicker.width
                    Layout.fillWidth: true
                    Accessible.name: text
                }
                // Free money or a fund pays for it.
                Controls.ButtonGroup { id: sourceGroup }
                RowLayout {
                    visible: ["card_purchase", "fix_reserves"].includes(page.kind)
                    Kirigami.FormData.label: i18nc("@label", "Paid from:")
                    Controls.RadioButton {
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                        text: i18n("Free money")
                        checked: page.source === "free"
                        Controls.ButtonGroup.group: sourceGroup
                        onToggled: { page.source = "free"; page.edited() }
                    }
                    Controls.RadioButton {
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                        text: i18n("A reserve")
                        checked: page.source === "fund"
                        Controls.ButtonGroup.group: sourceGroup
                        onToggled: { page.source = "fund"; page.edited() }
                    }
                }
                AccountPicker {
                    id: fundPicker
                    visible: ["card_purchase", "fix_reserves"].includes(page.kind) && page.source === "fund"
                    label: i18nc("money set aside", "Reserve")
                    accounts: page.funds
                    onEdited: page.edited()
                }
                AccountPicker {
                    id: personPicker
                    visible: page.kind === "split"
                    label: i18nc("the person a cost is split with", "With")
                    accounts: page.people
                    onEdited: page.edited()
                }
                // Amount, then date: stacked, never side by side.
                Controls.TextField {
                    id: desktopAmount
                    font.pixelSize: page.app.narrow ? 32 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 56 : implicitHeight
                    visible: !page.app.narrow && !["fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    Kirigami.FormData.label: page.kind === "split" ? i18nc("@label", "Total:") : i18nc("@label", "Amount:")
                    Accessible.name: i18n("Amount")
                    inputMethodHints: Qt.ImhFormattedNumbersOnly
                    text: amount.text
                    onTextEdited: { amount.text = text; page.edited() }
                    onAccepted: page.live ? page.saveNow() : page.review()
                }
                // The amount's problem shows as it's typed.
                Controls.Label {
                    readonly property string typed: amount.text === "" ? "" : page.ledger.checkAmount(amount.text, page.places, true)
                    visible: !page.app.narrow && text !== ""
                    text: page.fieldErrors.amount || page.fieldErrors.total || typed
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Accessible.name: i18nc("%1 is the field, %2 what's wrong with it", "%1 problem: %2", i18n("Amount"), text)
                }
                Controls.TextField {
                    font.pixelSize: page.app.narrow ? 16 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    id: theirShare
                    visible: page.kind === "split"
                    Kirigami.FormData.label: i18nc("@label", "Their share:")
                    Accessible.name: i18n("Their share")
                    inputMethodHints: Qt.ImhFormattedNumbersOnly
                    onTextEdited: page.edited()
                }
                Controls.Label {
                    readonly property string typed: theirShare.text === "" ? "" : page.ledger.checkAmount(theirShare.text, page.places, true)
                    visible: page.kind === "split" && text !== ""
                    text: page.fieldErrors.their_share || typed
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Accessible.name: i18nc("%1 is the field, %2 what's wrong with it", "%1 problem: %2", i18n("Their share"), text)
                }
                RowLayout {
                    visible: page.kind === "split"
                    Controls.Button {
                        text: i18nc("their share is half the total", "Half")
                        onClicked: {
                            const t = Number(amount.text.replace(/,/g, ""))
                            if (t > 0) theirShare.text = (Math.round(t * 50) / 100).toFixed(2)
                            page.edited()
                        }
                    }
                    Controls.Button { text: i18nc("their share is the whole total", "All of it"); onClicked: { theirShare.text = amount.text; page.edited() } }
                }
                Controls.TextField {
                    id: date
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    font.pixelSize: page.app.narrow ? 16 : Kirigami.Theme.defaultFont.pixelSize
                    TapHandler { enabled: page.app.narrow; onTapped: datePicker.open() }
                    visible: !["fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    Kirigami.FormData.label: i18nc("@label", "Date:")
                    Accessible.name: i18n("Date")
                    placeholderText: i18nc("the date format to type; dates are always typed this way", "YYYY-MM-DD")
                    onTextEdited: page.edited()
                }
                Controls.Label {
                    visible: page.fieldErrors.date !== undefined
                    text: page.fieldErrors.date || ""
                    color: Kirigami.Theme.negativeTextColor
                }
                Controls.TextField {
                    font.pixelSize: page.app.narrow ? 16 : Kirigami.Theme.defaultFont.pixelSize
                    Layout.minimumHeight: page.app.narrow ? 48 : implicitHeight
                    id: memo
                    visible: !["pay_card", "fix_reserves", "reconcile", "account_type"].includes(page.kind)
                    Kirigami.FormData.label: page.kind === "set_aside" ? i18nc("@label", "Description:") : i18nc("@label", "Memo:")
                    Accessible.name: i18n("Memo")
                    onTextEdited: page.edited()
                }
                Controls.Label {
                    visible: page.kind === "fix_reserves"
                    Kirigami.FormData.label: i18nc("@label", "Transactions:")
                    text: i18ncp("%1 is how many of the chosen transactions", "%1 missing its reserve lines", "%1 missing their reserve lines", (page.preset.ids || []).length)
                }
            }

            // What the server will add, so the preview holds no surprise.
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.kind === "card_purchase"
                type: Kirigami.MessageType.Information
                text: page.source === "free"
                    ? i18n("Also moves the amount from free money into the card's reserve, so the card stays covered.")
                    : i18n("Also moves the amount from the reserve into the card's reserve, so the card stays covered.")
            }


        }

        // The live dry run, beside the fields on wide windows.
        ColumnLayout {
            visible: page.live
            Layout.alignment: Qt.AlignTop
            Layout.fillWidth: true
            Kirigami.Heading { level: 4; text: i18n("Preview") }
            RowLayout {
                visible: page.previewing
                Controls.BusyIndicator {
                    running: parent.visible
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Layout.preferredWidth
                }
                Controls.Label { text: i18n("Checking with the server…"); opacity: 0.7; Accessible.name: text }
            }
            Controls.Label {
                text: page.preview === null ? i18n("Updates as you type. Nothing is written until you save.") : ""
                visible: page.preview === null && !page.previewing
                opacity: 0.7
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            PreviewPane {
                preview: page.preview
                Layout.fillWidth: true
            }
        }
    }

    Controls.Dialog {
        id: datePicker
        parent: Controls.Overlay.overlay
        modal: true
        x: 0; y: 0
        width: parent ? parent.width : 0
        height: parent ? parent.height : 0
        padding: 16
        property date shown: new Date()
        onOpened: { const parsed = new Date(date.text + "T12:00:00"); shown = isNaN(parsed.getTime()) ? new Date() : parsed }
        header: Controls.ToolBar {
            RowLayout {
                anchors.fill: parent
                Controls.ToolButton { text: i18n("Back"); implicitHeight: 48; onClicked: datePicker.close() }
                Kirigami.Heading { text: i18n("Date"); Layout.fillWidth: true }
            }
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 8
            RowLayout {
                Controls.ToolButton {
                    icon.name: "go-previous"
                    implicitWidth: 48; implicitHeight: 48
                    Accessible.name: Qt.formatDate(new Date(datePicker.shown.getFullYear(), datePicker.shown.getMonth() - 1, 1), "MMMM yyyy")
                    onClicked: datePicker.shown = new Date(datePicker.shown.getFullYear(), datePicker.shown.getMonth() - 1, 1)
                }
                Controls.Label { text: Qt.formatDate(datePicker.shown, "MMMM yyyy"); Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter }
                Controls.ToolButton {
                    icon.name: "go-next"
                    implicitWidth: 48; implicitHeight: 48
                    Accessible.name: Qt.formatDate(new Date(datePicker.shown.getFullYear(), datePicker.shown.getMonth() + 1, 1), "MMMM yyyy")
                    onClicked: datePicker.shown = new Date(datePicker.shown.getFullYear(), datePicker.shown.getMonth() + 1, 1)
                }
            }
            Controls.DayOfWeekRow { Layout.fillWidth: true }
            Controls.MonthGrid {
                month: datePicker.shown.getMonth()
                year: datePicker.shown.getFullYear()
                Layout.fillWidth: true
                Layout.preferredHeight: 6 * 48
                onClicked: function(chosen) { date.text = Qt.formatDate(chosen, "yyyy-MM-dd"); page.edited(); datePicker.close() }
            }
            Item { Layout.fillHeight: true }
        }
    }

    Component {
        id: reviewPage
        ReviewPage { app: page.app }
    }
}
