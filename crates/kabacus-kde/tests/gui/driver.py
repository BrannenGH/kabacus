# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""AT-SPI driver for tests/gui_e2e.rs. Drives the real UI and exits non-zero
with a message (and the app's accessibility tree) on failure. It works at
every window size: the drawer (wide), the icon rail (tablet) or the bottom
tabs and More (phone).

  driver.py add <payee> <amount> <from> <to>
  driver.py edit <account> <row-payee> <row-date> <new-date> <new-payee> <amount> <to>
  driver.py card-purchase <payee> <amount> <card> <category>
  driver.py set-aside <fund> <amount>
  driver.py fix-reserves
  driver.py conflict <payee> <amount> <card> <category> <sync-file>
  driver.py offline
  driver.py settings <socket>
  driver.py about <commit> <source-url>
  driver.py unchanged <account> <row-payee> <row-date>
  driver.py edit-modes <account> <row-payee> <row-date>
  driver.py discard-prompt <account> <row-payee> <row-date> <amount>
  driver.py new-fund <name> <amount>
  driver.py overdraw-warning <fund> <amount>
  driver.py language <overview> <cash-on-hand> <accounts> <money> <account>
  driver.py overview <account> <fund> <empty-fund>
  driver.py loans <loan> <loan-payee>
  driver.py reconcile <account> <statement-balance> <spending-account>"""

import os
import subprocess
import sys
import time

import pyatspi
from gi.repository import GLib


def fail(msg):
    shot = os.environ.get("KABACUS_GUI_SHOT")
    if shot:
        subprocess.run(["import", "-window", "root", shot], check=False)
    print(f"FAIL: {msg}\nThe app's accessibility tree:", file=sys.stderr)
    frame = app()
    if frame is not None:
        dump(frame)
    sys.exit(1)


def dump(node, depth=0):
    for child in node:
        if child is None:
            continue
        if child.name:
            try:
                text = child.queryText().getText(0, -1)
            except NotImplementedError:
                text = ""
            extra = f" = {text!r}" if text and text != child.name else ""
            print("  " * depth, child.getRoleName(), repr(child.name) + extra, file=sys.stderr)
        if depth < 8:
            dump(child, depth + 1)


def wait_for(what, find, timeout=20):
    deadline = time.time() + timeout
    while time.time() < deadline:
        # On a busy machine the app may not answer AT-SPI in time: a look
        # then fails or finds no window. That's "not yet", not a failure.
        try:
            found = find()
        except (GLib.Error, TypeError, AttributeError) as e:
            print(f"note: the app didn't answer ({e})", file=sys.stderr)
            found = None
        if found:
            return found
        time.sleep(0.2)
    fail(f"timed out waiting for {what}")


def app():
    for a in pyatspi.Registry.getDesktop(0):
        # The application is "kabacus" (its binary); the window is titled
        # with its display name, "Kabacus".
        if a is not None and a.name.lower() == "kabacus":
            for frame in a:
                if frame is not None and (frame.name.lower() == "kabacus" or frame.name.endswith("— Kabacus")):
                    return frame
    return None


def find(root, name, role=None, showing=True):
    # Roles are pyatspi constants: their names differ between AT-SPI
    # versions (ROLE_PUSH_BUTTON is "push button", later "button").
    def match(n):
        if n is None or n.name != name:
            return False
        if role is not None and n.getRole() != role:
            return False
        return not showing or n.getState().contains(pyatspi.STATE_SHOWING)
    return pyatspi.findDescendant(root, match, breadth_first=True)


def press(node):
    # Qt can drop the node's accessible object while the press is being
    # handled, failing the reply; callers wait for the press's effect.
    try:
        node.queryAction().doAction(0)
    except GLib.Error as e:
        print(f"note: press reply lost ({e.message})", file=sys.stderr)


def key(keysym):
    pyatspi.Registry.generateKeyboardEvent(keysym, None, pyatspi.KEY_SYM)
    time.sleep(0.02)


def click(node):
    """A real left click in the middle of `node`, as a mouse user would."""
    x, y, w, h = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    pyatspi.Registry.generateMouseEvent(x + w // 2, y + h // 2, pyatspi.MOUSE_B1C)


def open_list_item(what, find_row, opened, timeout=30):
    """Open a list row with a real click, as a finger or mouse would, and
    wait until `opened()`. The row is scrolled into view first (on a phone
    it can be below the fold, or under the bottom tabs), and found afresh
    on each try: a row found just before its list reloads is gone by the
    time an AT-SPI press reaches it."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            fx, fy, fw, fh = app().queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            node = find_row()
            if node is None:
                time.sleep(0.2)
                continue
            _, y, _, h = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            # Room at the bottom for the tabs and the floating Add button.
            if y < fy + 60 or y + h > fy + fh - 160:
                wheel = "b5c" if y + h > fy + fh - 160 else "b4c"
                pyatspi.Registry.generateMouseEvent(fx + fw // 2, fy + fh // 2, wheel)
                time.sleep(0.3)
                continue
            click(node)
        except (GLib.Error, TypeError, AttributeError) as e:
            print(f"note: the row went away ({e})", file=sys.stderr)
            continue
        end = time.time() + 5
        while time.time() < end:
            try:
                if opened():
                    return
            except (GLib.Error, TypeError, AttributeError):
                pass
            time.sleep(0.2)
    fail(f"timed out opening {what}")


def set_text(node, text):
    # Qt Quick fields don't implement EditableText, so focus, clear and type.
    node.queryComponent().grabFocus()
    wait_for("focus", lambda: node.getState().contains(pyatspi.STATE_FOCUSED), 5)
    try:
        length = node.queryText().characterCount
    except NotImplementedError:  # editable combo boxes
        length = 40
    key(0xFF57)  # End
    for _ in range(length):
        key(0xFF08)  # BackSpace
    for ch in text:
        pyatspi.Registry.generateKeyboardEvent(0, ch, pyatspi.KEY_STRING)
        time.sleep(0.02)


def choose(combo_name, item_name):
    # The pickers are editable: focusing selects all, typing replaces,
    # and Tab moves on, committing the choice.
    set_text(find(app(), combo_name, pyatspi.ROLE_COMBO_BOX), item_name)
    pyatspi.Registry.generateKeyboardEvent(0xFF09, None, pyatspi.KEY_SYM)  # Tab


def choose_phone(combo_name, item_name):
    click(wait_for(f"the {combo_name} field", lambda: find(app(), combo_name, pyatspi.ROLE_COMBO_BOX)))
    search = wait_for("the full-screen account picker", lambda: find(app(), "Search", pyatspi.ROLE_TEXT))
    set_text(search, item_name)
    item = wait_for(f"the {item_name} choice", lambda: find(app(), item_name, pyatspi.ROLE_LIST_ITEM))
    _, _, width, height = item.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    if width < 48 or height < 48:
        fail(f"the {item_name} picker target is only {width} by {height}")
    click(item)
    wait_for("the picker to close", lambda: not find(app(), "Search", pyatspi.ROLE_TEXT))


def row(payee, date=None):
    name = f"{payee} {date}" if date else None
    return pyatspi.findDescendant(
        app(), lambda n: n is not None and n.getRole() == pyatspi.ROLE_LIST_ITEM
        and (n.name == name if name else n.name.startswith(payee + " ")))


def fill(date, payee, amount, to_name):
    frame = app()
    if date is not None:
        set_text(find(frame, "Date", pyatspi.ROLE_TEXT), date)
    set_text(find(frame, "Payee", pyatspi.ROLE_TEXT), payee)
    set_text(find(frame, "Amount", pyatspi.ROLE_TEXT), amount)
    choose("To", to_name)
    press(find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON))


def label_starting(prefix):
    return pyatspi.findDescendant(
        app(), lambda n: n is not None and n.name.startswith(prefix)
        and n.getState().contains(pyatspi.STATE_SHOWING))


def beside_its_field(name):
    """A form label (`name:`) lines up with its field: level with it on
    wide forms, above it where the form stacks them. Never level with what
    shows under the field (its error, a suggestion)."""
    label = wait_for(f"the {name}: label", lambda: find(app(), f"{name}:"), 10)
    field = find(app(), name, pyatspi.ROLE_COMBO_BOX)
    _, ly, _, lh = label.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    _, fy, _, fh = field.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    middle = ly + lh // 2
    if not (fy <= middle <= fy + fh or ly + lh <= fy + 2):
        fail(f"the {name}: label (y {ly}..{ly + lh}) isn't beside or above its field (y {fy}..{fy + fh})")


def find_any(name, roles):
    def match(n):
        return (n is not None and n.name == name and n.getRole() in roles
                and n.getState().contains(pyatspi.STATE_SHOWING))
    return pyatspi.findDescendant(app(), match, breadth_first=True)


def mirroring(first, second):
    """Whether the navigation is laid out right to left, from its first two
    destinations: in a row (a phone's bottom tabs) the first is to the
    right of the second; in a column (the drawer or its icon rail) the
    column is on the window's right side. Also says where they are."""
    fx, _, fw, _ = app().queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    x1, y1, w1, h1 = first.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    x2, y2, w2, h2 = second.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    if abs(y1 - y2) < min(h1, h2) / 2:
        return x1 > x2, f"the first tab is at x {x1} and the second at x {x2}"
    middle = x2 + w2 / 2
    return middle > fx + fw / 2, f"the navigation's middle is at x {middle:.0f} in a window from x {fx} to {fx + fw}"


def goto(name):
    """A destination: a bottom tab on a phone (People, Loans, Reports,
    Activity and Settings are under More there), else the drawer."""
    if find_any("More", {pyatspi.ROLE_PAGE_TAB}):
        tab = find_any(name, {pyatspi.ROLE_PAGE_TAB})
        if tab:
            click(tab)
            return
        click(find_any("More", {pyatspi.ROLE_PAGE_TAB}))
        click(wait_for(f"{name} under More", lambda: find_any(name, {pyatspi.ROLE_LIST_ITEM})))
        return
    click(wait_for(f"the {name} destination", lambda: find_any(name, {pyatspi.ROLE_LIST_ITEM})))


def books_loaded():
    """The books are there: Transactions lists the testkit's account."""
    goto("Transactions")
    wait_for("the accounts list", lambda: find(app(), "Everyday Checking", pyatspi.ROLE_LIST_ITEM))


def open_add(kind):
    """Add (the Overview's group, or the floating button), then a kind."""
    goto("Overview")
    press(wait_for("the Add button", lambda: find(app(), "Add", pyatspi.ROLE_PUSH_BUTTON)))
    click(wait_for(f"the {kind} choice", lambda: find(app(), kind)))
    wait_for("the form", lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))


def save():
    """Save from the form: directly where the preview is live (wide),
    else through Review before saving."""
    review = find(app(), "Review", pyatspi.ROLE_PUSH_BUTTON)
    if review is not None:
        press(review)
        wait_for("the review page", lambda: label_starting("Review before saving") or find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON), 90)
    press(wait_for("the Save button", lambda: find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON)))
    wait_for("the saved notification", lambda: label_starting("Saved as revision"), 90)


def open_row(account, payee, date):
    """Open one transaction in the edit form from its account's register."""
    goto("Transactions")
    click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
    click(wait_for(f"the {payee!r} row", lambda: row(payee, date)))
    wait_for("the edit form", lambda: label_starting("Edit transaction"))


def fill_fields(**fields):
    # Wait for each: a form that just opened may still be laying out.
    for name, value in fields.items():
        set_text(wait_for(name, lambda: find(app(), name, pyatspi.ROLE_TEXT)), value)


def main():
    command, args = sys.argv[1], sys.argv[2:]
    wait_for("the Kabacus window", app)
    if command == "phone-navigation":
        labels = ["Overview", "Transactions", "Reserves", "Credit", "More"]
        tabs = [wait_for(f"the {label} tab", lambda label=label: find_any(label, {pyatspi.ROLE_PAGE_TAB})) for label in labels]
        bounds = [tab.queryComponent().getExtents(pyatspi.DESKTOP_COORDS) for tab in tabs]
        if [b.x for b in bounds] != sorted(b.x for b in bounds):
            fail("phone tabs are out of order")
        if any(b.width < 48 or b.height < 48 for b in bounds):
            fail(f"a phone tab is smaller than 48 px: {bounds}")
        goto("More")
        for label in ["Loans", "People", "Reports", "Activity", "Settings"]:
            wait_for(f"{label} under More", lambda label=label: find_any(label, {pyatspi.ROLE_LIST_ITEM}))
        click(wait_for("Switch Books", lambda: find(app(), "Switch Books…")))
        for label in ["New Books…", "Open Books…", "Save a Copy As…", "Connect to a Server…"]:
            wait_for(label, lambda label=label: find(app(), label))
        for label in ["New Window", "Close Window", "Quit"]:
            if find(app(), label):
                fail(f"{label} offered in phone Books sheet")
        click(wait_for("close Books", lambda: find(app(), "Close", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the Books sheet to close", lambda: not find(app(), "New Books…"))
        books_loaded()
        click(wait_for("the account", lambda: find(app(), "Everyday Checking", pyatspi.ROLE_LIST_ITEM)))
        wait_for("the register", lambda: find(app(), "Everyday Checking", pyatspi.ROLE_LABEL))
        press(wait_for("Back", lambda: find(app(), "Back", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("accounts after Back", lambda: find(app(), "Everyday Checking", pyatspi.ROLE_LIST_ITEM))
        print("ok")
        return
    if command == "about":
        version, source_url = args
        books_loaded()
        goto("Settings")
        wait_for("the application version", lambda: label_starting("Kabacus " + version))
        wait_for("the source button", lambda: find(app(), "Open the source code", pyatspi.ROLE_PUSH_BUTTON))
        print("ok")
        return
    if command == "signin":
        # Signed out at first: the banner's Sign in opens the browser, which
        # comes straight back signed in (tests/gui_e2e.rs's fake browser).
        (username,) = args
        press(wait_for("the Sign in button", lambda: find(app(), "Sign in", pyatspi.ROLE_PUSH_BUTTON)))
        books_loaded()
        goto("Settings")
        wait_for(f"signed in as {username}", lambda: label_starting(username))
        print("ok")
        return
    if command == "offline":
        # What was loaded stays; every write is off until it's back.
        # In a plain sentence; the raw error is behind Details.
        banner = wait_for("the offline banner", lambda: label_starting("Can't reach kabacus-server at "))
        if "reqwest" in banner.name or "Error {" in banner.name:
            fail(f"raw error in the banner: {banner.name!r}")
        press(wait_for("Details", lambda: find(app(), "Details", pyatspi.ROLE_PUSH_BUTTON)))
        details = wait_for("the details", lambda: find(app(), "Error details", pyatspi.ROLE_TEXT))
        if "request failed" not in details.queryText().getText(0, -1):
            fail("the details don't hold the raw error")
        key(0xFF1B)  # Escape
        add = wait_for("the Add button", lambda: find(app(), "Add", pyatspi.ROLE_PUSH_BUTTON))
        if add.getState().contains(pyatspi.STATE_ENABLED):
            fail("Add is enabled while offline")
        print("ok")
        return
    if command == "language":
        # The app in another language: the Overview and its Cash on Hand
        # (QML's i18n), then Transactions and its Money heading (Rust's i18n!),
        # each named as given, and none of it left in English. `direction`
        # (ltr or rtl) is which way the language is written: the app
        # mirrors for rtl.
        overview, cash, accounts, money, account, direction = args
        wait_for(f"the {overview!r} page", lambda: find(app(), overview))
        wait_for(f"the {cash!r} label", lambda: find(app(), cash))
        for english in ("Overview", "Cash on Hand", "Transactions"):
            if find(app(), english):
                fail(f"{english!r} isn't translated")
        # A bottom tab on a phone, else the drawer or its icon rail.
        destinations = {pyatspi.ROLE_PAGE_TAB, pyatspi.ROLE_LIST_ITEM}
        destination = wait_for(f"the {accounts!r} destination",
                               lambda: find_any(accounts, destinations))
        first = wait_for(f"the {overview!r} destination", lambda: find_any(overview, destinations))
        mirrored, where = mirroring(first, destination)
        if mirrored != (direction == "rtl"):
            fail(f"written {direction}, but {where}")
        click(destination)
        wait_for(f"the {money!r} heading", lambda: find(app(), money))
        wait_for(f"the {account!r} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM))
        if find(app(), "Money"):
            fail("'Money' isn't translated")
        print("ok")
        return
    books_loaded()

    if command == "resume":
        if find(app(), "Sign in", pyatspi.ROLE_PUSH_BUTTON):
            fail("asked to sign in again after a restart")
    elif command == "auth-off":
        wait_for("the authentication-off indicator", lambda: label_starting("Authentication off (local mode)"))
        if find(app(), "Sign in", pyatspi.ROLE_PUSH_BUTTON):
            fail("offered sign-in on a no-auth server")
    elif command == "add":
        payee, amount, from_name, to_name = args
        open_add("Postings")
        fill_fields(Payee=payee, Amount=amount)
        choose("From", from_name)
        choose("To", to_name)
        save()
    elif command == "edit":
        account, old_payee, old_date, date, payee, amount, to_name = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        click(wait_for(f"the {old_payee!r} row", lambda: row(old_payee, old_date)))
        wait_for("the edit form", lambda: label_starting("Edit transaction"))
        fill_fields(Date=date, Payee=payee, Amount=amount)
        choose("To", to_name)
        save()
    elif command == "edit-split":
        account, payee, date = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        click(wait_for(f"the {payee!r} row", lambda: row(payee, date)))
        wait_for("the edit form", lambda: label_starting("Edit transaction"))
        for name, value in (("Amount 1", "15.00"), ("Amount 2", "5.00"), ("Amount 3", "-20.00")):
            set_text(wait_for(name, lambda: find(app(), name, pyatspi.ROLE_TEXT, showing=False)), value)
        save()
    elif command == "edit-prefill":
        # The edit form shows the transaction's own accounts, and
        # keeps them when the pickers' account list is reloaded.
        account, payee, date, from_name, to_name, sync = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        click(wait_for(f"the {payee!r} row", lambda: row(payee, date)))
        wait_for("the edit form", lambda: label_starting("Edit transaction"))

        def shows(name, expected):
            picker = find(app(), name, pyatspi.ROLE_COMBO_BOX)
            return picker is not None and picker.description == expected

        def check(when):
            for name, expected in (("From", from_name), ("To", to_name)):
                if not shows(name, expected):
                    picker = find(app(), name, pyatspi.ROLE_COMBO_BOX)
                    fail(f"{when}: {name} shows {picker and picker.description!r}, not {expected!r}")

        wait_for("the pickers", lambda: shows("From", from_name) or None, 10)
        time.sleep(1)
        check("on opening")
        # Another client adds an account; F5 refreshes, reloading the
        # pickers' account list under the open form.
        open(sync + ".ready", "w").close()
        wait_for("the other write", lambda: os.path.exists(sync + ".go"), timeout=60)
        key(0xFFC2)  # F5
        time.sleep(3)
        check("after a reload")
    elif command == "external-edit":
        # A change from another client shows once you navigate back.
        account, payee, new_payee, sync = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        wait_for(f"the {payee!r} row", lambda: row(payee))
        open(sync + ".ready", "w").close()
        wait_for("the change from elsewhere", lambda: os.path.exists(sync + ".go"), timeout=60)
        goto("Overview")
        books_loaded()
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        wait_for(f"the edited {new_payee!r} row", lambda: row(new_payee))
    elif command == "register-add":
        # Add from a register presets its account.
        account, kind, field, expected = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        wait_for("the register", lambda: find(app(), "Add transaction", pyatspi.ROLE_PUSH_BUTTON)
                 or find(app(), "Add", pyatspi.ROLE_PUSH_BUTTON))
        time.sleep(1)
        press(find(app(), "Add transaction", pyatspi.ROLE_PUSH_BUTTON)
              or find(app(), "Add", pyatspi.ROLE_PUSH_BUTTON))
        click(wait_for(f"the {kind} choice", lambda: find(app(), kind)))
        wait_for("the form", lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))

        def shows():
            picker = find(app(), field, pyatspi.ROLE_COMBO_BOX)
            return picker is not None and picker.description == expected
        wait_for(f"{field} preset to {expected}", lambda: shows() or None, 10)
    elif command == "heading-order":
        # Each group's heading comes before its accounts
        # in the accessible tree, as on screen.
        pairs = list(zip(args[::2], args[1::2]))
        wait_for("the accounts", lambda: find(app(), pairs[0][1], pyatspi.ROLE_LIST_ITEM))
        order = []

        def walk(node):
            if node is None:
                return
            if node.getRole() in (pyatspi.ROLE_HEADING, pyatspi.ROLE_LIST_ITEM):
                order.append((node.getRole(), node.name))
            for child in node:
                walk(child)
        walk(app())
        for heading, item in pairs:
            names = [n for _, n in order]
            h = next((i for i, (r, n) in enumerate(order) if r == pyatspi.ROLE_HEADING and n == heading), None)
            a = next((i for i, (r, n) in enumerate(order) if r == pyatspi.ROLE_LIST_ITEM and n == item), None)
            if h is None or a is None or h > a:
                fail(f"heading {heading!r} at {h}, item {item!r} at {a}: {names}")
    elif command == "empty-search":
        # A search with no matches says so and offers to clear it.
        account, payee = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        wait_for(f"the {payee!r} row", lambda: row(payee))
        set_text(find(app(), "Search", pyatspi.ROLE_TEXT), "zzz")
        wait_for("the no-match message", lambda: label_starting("No transactions match"), 10)
        press(wait_for("Clear search", lambda: find(app(), "Clear search", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for(f"the {payee!r} row again", lambda: row(payee), 10)
    elif command == "chip-tap":
        # A tap on a row's chip opens the row, like the rest of it.
        account, chip = args
        click(wait_for(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM)))
        click(wait_for(f"the {chip!r} chip", lambda: find(app(), chip)))
        wait_for("the edit form", lambda: label_starting("Edit transaction"))
    elif command == "amount-precision":
        # The amount's problem shows while typing, before Save.
        (amount,) = args
        open_add("Spent")
        fill_fields(Amount=amount)
        wait_for("the precision problem", lambda: label_starting("Amount problem: Use at most 2 decimal places"), 10)
    elif command == "unchanged":
        # An edit opened and not changed has nothing to save: Save is off,
        # beside the live preview (wide) or on the Review page.
        account, payee, date = args
        open_row(account, payee, date)
        # Review where the preview isn't live (tablet, phone): wait for one
        # or the other, as the form may still be laying out.
        ready = wait_for("Review or the live preview", lambda: find(app(), "Review", pyatspi.ROLE_PUSH_BUTTON)
                         or find(app(), "Preview", pyatspi.ROLE_HEADING))
        if ready.getRole() == pyatspi.ROLE_PUSH_BUTTON:
            press(ready)
            wait_for("the review page", lambda: label_starting("Review before saving") or find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON), 90)
        wait_for("nothing to save", lambda: label_starting("Nothing to save"), 30)
        save_button = wait_for("the Save button", lambda: find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON))
        if save_button.getState().contains(pyatspi.STATE_ENABLED):
            fail("Save is on with nothing to save")
    elif command == "edit-modes":
        # A recorded transaction is edited in place or voided; nothing else.
        account, payee, date = args
        open_row(account, payee, date)
        wait_for("Edit in place", lambda: find(app(), "Edit in place", pyatspi.ROLE_RADIO_BUTTON))
        find(app(), "Void", pyatspi.ROLE_RADIO_BUTTON) or fail("no Void option")
        if find(app(), "Add a correction", pyatspi.ROLE_RADIO_BUTTON):
            fail("a correction is offered")
        field = find(app(), "Date", pyatspi.ROLE_TEXT)
        if not field.getState().contains(pyatspi.STATE_ENABLED):
            fail("an edit's Date can't be changed")
    elif command == "discard-prompt":
        # Going elsewhere with a change typed in asks first; Keep editing
        # keeps it, Discard drops it and goes.
        account, payee, date, amount = args
        open_row(account, payee, date)
        fill_fields(Amount=amount)
        goto("Reserves")
        wait_for("the discard question", lambda: label_starting("Discard this change?"), 10)
        press(find(app(), "Keep editing", pyatspi.ROLE_PUSH_BUTTON))
        wait_for("the form, still open", lambda: label_starting("Edit transaction"), 10)
        typed = find(app(), "Amount", pyatspi.ROLE_TEXT).queryText().getText(0, -1)
        if typed != amount:
            fail(f"the typed amount is {typed!r}, not {amount!r}")
        goto("Reserves")
        wait_for("the discard question again", lambda: label_starting("Discard this change?"), 10)
        press(find(app(), "Discard", pyatspi.ROLE_PUSH_BUTTON))
        wait_for("the Reserves page", lambda: find(app(), "Set aside for Gifts", pyatspi.ROLE_PUSH_BUTTON), 20)
        if label_starting("Edit transaction"):
            fail("the form is still open after Discard")
    elif command == "new-fund":
        # Set aside into a fund that doesn't exist yet: the name typed in
        # To becomes a new fund, made by the save.
        fund, amount = args
        goto("Reserves")
        press(wait_for("Set aside", lambda: find(app(), "Set aside", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the form", lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))
        fill_fields(Amount=amount)
        # A name that can't be a fund: the error shows under To, and To's
        # label stays beside its field, not halfway down beside the error.
        choose("To", "travel:japan")
        review = find(app(), "Review", pyatspi.ROLE_PUSH_BUTTON)
        if review is not None:
            press(review)
        wait_for("the To problem", lambda: label_starting("To problem: "), 90)
        beside_its_field("To")
        choose("To", fund)
        wait_for("the new-fund note", lambda: label_starting(f"{fund} isn't a reserve yet. Saving creates it in "), 10)
        save()
    elif command == "overdraw-refused":
        # Setting aside more than is free: the server refuses it, and the
        # shortfall shows under Amount. Nothing is saved.
        fund, amount = args
        goto("Reserves")
        press(wait_for(f"Set aside for {fund}", lambda: find(app(), f"Set aside for {fund}", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the form", lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))
        fill_fields(Amount=amount)
        wait_for("the shortfall", lambda: label_starting("Amount problem: Checking has only"), 90)
    elif command == "overview":
        # Cash on Hand says what it means behind its info button (a dialog
        # on a click or tap, as a tooltip never shows on touch) and opens
        # into its sum; free money and reserves list only what holds money,
        # an account opening its register and a reserve the set-aside
        # form; KDE's menu bar is there except on a phone.
        account, fund, empty = args
        goto("Overview")
        wait_for("available-to-spend heading", lambda: find(app(), "Available to spend"))
        wait_for("available-by-account heading", lambda: find(app(), "Available by account"))
        meaning = "Free money you can spend right now"
        about = wait_for("Cash on Hand's info button", lambda: find(app(), "About Cash on Hand", pyatspi.ROLE_PUSH_BUTTON))
        if label_starting(meaning):
            fail("what Cash on Hand means shows before it's asked for")
        press(about)
        wait_for("what Cash on Hand means", lambda: label_starting(meaning), 10)
        key(0xFF1B)  # Escape
        wait_for("the dialog to close", lambda: label_starting(meaning) is None, 10)

        toggles = {pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_PUSH_BUTTON, pyatspi.ROLE_CHECK_BOX}
        if label_starting("− Receivables"):
            fail("Money on Hand's sum is open before it's asked for")
        open_list_item("Money on Hand's sum", lambda: find_any("Money on Hand", toggles),
                       lambda: label_starting("− Receivables"))

        wait_for(f"the {fund} fund", lambda: find(app(), fund, pyatspi.ROLE_LIST_ITEM))
        if find(app(), empty, pyatspi.ROLE_LIST_ITEM):
            fail(f"the empty {empty} reserve is on the Overview")
        open_list_item(f"the {fund} reserve", lambda: find(app(), fund, pyatspi.ROLE_LIST_ITEM),
                 lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))

        def to_is_fund():
            picker = find(app(), "To", pyatspi.ROLE_COMBO_BOX)
            return picker is not None and picker.description == fund
        wait_for(f"To preset to {fund}", lambda: to_is_fund() or None, 10)

        goto("Overview")
        open_list_item(f"{account}'s free money", lambda: pyatspi.findDescendant(
            app(), lambda n: n is not None and n.getRole() == pyatspi.ROLE_LIST_ITEM
            and n.name.startswith(f"{account} free ") and n.getState().contains(pyatspi.STATE_SHOWING)),
            lambda: row("Opening"))

        goto("Overview")
        wait_for("the Overview", lambda: find(app(), "About Cash on Hand", pyatspi.ROLE_PUSH_BUTTON))
        # Qt reports a menu bar's titles as "menu bar".
        menus = {pyatspi.ROLE_MENU_BAR, pyatspi.ROLE_MENU, pyatspi.ROLE_MENU_ITEM, pyatspi.ROLE_PUSH_BUTTON}
        if find_any("More", {pyatspi.ROLE_PAGE_TAB}):
            if find_any("File", menus) or find_any("Go", menus):
                fail("a menu bar on a phone")
        else:
            click(wait_for("the Go menu", lambda: find_any("Go", menus), 10))
            click(wait_for("Go → Reserves", lambda: find_any("Reserves", {pyatspi.ROLE_MENU_ITEM, pyatspi.ROLE_PUSH_BUTTON}), 10))
            wait_for("the Reserves page", lambda: find(app(), "Set aside for Gifts", pyatspi.ROLE_PUSH_BUTTON), 20)
    elif command == "all-transactions":
        # All transactions lists real ones across accounts; fund transfers
        # come back with their switch.
        real, transfer = args
        goto("Transactions")
        open_list_item("All transactions", lambda: find(app(), "All transactions", pyatspi.ROLE_LIST_ITEM),
                 lambda: row(real))
        if row(transfer):
            fail(f"the fund transfer {transfer!r} shows by default")
        toggles = {pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_PUSH_BUTTON, pyatspi.ROLE_CHECK_BOX}
        press(wait_for("the Fund transfers switch", lambda: find_any("Fund transfers", toggles)))
        wait_for(f"the {transfer!r} fund transfer", lambda: row(transfer))
        if not row(real):
            fail(f"{real!r} went missing with fund transfers on")
    elif command == "loans":
        # Loans lists each loan; a loan opens its register.
        loan, payee = args
        goto("Loans")
        open_list_item(f"the {loan} loan", lambda: find(app(), loan, pyatspi.ROLE_LIST_ITEM), lambda: row(payee))
    elif command == "reconcile":
        # Reconcile isn't a destination: it's on the register of an
        # account a statement comes for, and not on a spending account's.
        account, balance, spending = args
        if find_any("More", {pyatspi.ROLE_PAGE_TAB}):
            click(find_any("More", {pyatspi.ROLE_PAGE_TAB}))
            wait_for("the More page", lambda: find_any("Activity", {pyatspi.ROLE_LIST_ITEM}))
        if find_any("Reconcile", {pyatspi.ROLE_LIST_ITEM}):
            fail("Reconcile is still a destination")
        reconcile = "Reconcile…"
        goto("Transactions")
        open_list_item(f"the {spending} account", lambda: find(app(), spending, pyatspi.ROLE_LIST_ITEM),
                 lambda: find(app(), "Add transaction", pyatspi.ROLE_PUSH_BUTTON))
        time.sleep(1)
        if find(app(), reconcile, pyatspi.ROLE_PUSH_BUTTON):
            fail(f"Reconcile is offered for {spending}")
        goto("Transactions")
        open_list_item(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM),
                 lambda: find(app(), reconcile, pyatspi.ROLE_PUSH_BUTTON))
        press(find(app(), reconcile, pyatspi.ROLE_PUSH_BUTTON))
        fill_fields(**{"Statement balance": balance})
        press(wait_for("Check", lambda: find(app(), "Check", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the match", lambda: label_starting("Matched: "), 30)
    elif command == "account-type":
        account, target = args
        goto("Transactions")
        open_list_item(f"the {account} account", lambda: find(app(), account, pyatspi.ROLE_LIST_ITEM),
                       lambda: find(app(), "Account type", pyatspi.ROLE_PUSH_BUTTON))
        press(find(app(), "Account type", pyatspi.ROLE_PUSH_BUTTON))
        combo = wait_for("the account type selector", lambda: find(app(), "Account type", pyatspi.ROLE_COMBO_BOX))
        click(combo)
        click(wait_for(f"the {target} option", lambda: find(app(), target)))
        wait_for(f"{target} selected", lambda: combo.description == target)
        save()
    elif command == "card-purchase":
        payee, amount, card, category = args
        open_add("Card purchase")
        _, _, width, _ = app().queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        pick = choose_phone if width < 600 else choose
        pick("Card", card)
        fill_fields(Payee=payee, Amount=amount)
        pick("Category", category)
        save()
    elif command == "set-aside":
        fund, amount = args
        goto("Reserves")
        press(wait_for(f"Set aside for {fund}", lambda: find(app(), f"Set aside for {fund}", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the form", lambda: find(app(), "Amount", pyatspi.ROLE_TEXT))
        fill_fields(Amount=amount)
        save()
    elif command == "fix-reserves":
        goto("Credit")
        press(wait_for("Add reserve lines", lambda: find(app(), "Add reserve lines", pyatspi.ROLE_PUSH_BUTTON)))
        wait_for("the fix form", lambda: label_starting("Fix reserve lines"))
        save()
    elif command == "conflict":
        # The books change between the preview and Save.
        payee, amount, card, category, sync = args
        open_add("Card purchase")
        choose("Card", card)
        fill_fields(Payee=payee, Amount=amount)
        choose("Category", category)
        # On the desktop the preview is live beside the form.
        wait_for("the live preview", lambda: label_starting("The books still check out"), 90)
        open(sync + ".ready", "w").close()
        wait_for("the other write", lambda: os.path.exists(sync + ".go"), timeout=60)
        press(find(app(), "Save", pyatspi.ROLE_PUSH_BUTTON))
        wait_for("the conflict dialog", lambda: label_starting("Someone saved first"))
        press(wait_for("Keep editing", lambda: find(app(), "Keep editing", pyatspi.ROLE_PUSH_BUTTON)))
    else:
        fail(f"unknown command {command!r}")
    print("ok")


if __name__ == "__main__":
    main()
