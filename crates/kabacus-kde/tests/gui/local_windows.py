# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise local creation and restoration in real desktop windows through AT-SPI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import pyatspi
import driver


def frame(name):
    return pyatspi.findDescendant(pyatspi.Registry.getDesktop(0), lambda n: n is not None and n.name == name and n.getRole() == pyatspi.ROLE_FRAME and n.getState().contains(pyatspi.STATE_SHOWING))


def showing(node, role):
    return pyatspi.findDescendant(node, lambda n: n is not None and n.getRole() == role and n.getState().contains(pyatspi.STATE_SHOWING))


def create(name, path):
    try:
        welcome = driver.wait_for('Welcome window', lambda: frame('Kabacus'))
    except SystemExit:
        driver.dump(pyatspi.Registry.getDesktop(0));raise
    driver.click(driver.wait_for('New Books', lambda: driver.find(welcome, 'New Books…', pyatspi.ROLE_PUSH_BUTTON)))
    dialog = driver.wait_for('file chooser', lambda: frame('New Books…'))
    field = driver.wait_for('file name field', lambda: showing(dialog, pyatspi.ROLE_TEXT))
    driver.click(field)
    driver.set_text(field, path.name)
    driver.key(0xFF09)
    driver.click(driver.find(dialog, 'Save', pyatspi.ROLE_PUSH_BUTTON))
    try:
        driver.wait_for('book setup', lambda: driver.find(welcome, 'Create books', pyatspi.ROLE_PUSH_BUTTON))
    except SystemExit:
        driver.dump(pyatspi.Registry.getDesktop(0));raise
    name_field=driver.find(welcome, 'Name', pyatspi.ROLE_TEXT)
    driver.click(name_field)
    driver.set_text(name_field, name)
    driver.click(driver.find(welcome, 'Create books', pyatspi.ROLE_PUSH_BUTTON))
    result = driver.wait_for(name, lambda: frame(name + ' — Kabacus'))
    driver.wait_for('Overview balance', lambda: driver.find(result, 'Cash on Hand'))
    assert path.is_file(), path
    return result


with tempfile.TemporaryDirectory(prefix='kabacus-local-windows-') as folder:
    root = Path(folder)
    env = dict(os.environ, XDG_CONFIG_HOME=str(root / 'config'), XDG_CACHE_HOME=str(root / 'cache'), QT_LINUX_ACCESSIBILITY_ALWAYS_ON='1', LANGUAGE='en')
    env.pop('KABACUS_SOCKET', None)
    binary = str(Path(sys.argv[1]).resolve())
    process = subprocess.Popen([binary, '--size=1280x800'], env=env, cwd=root)
    try:
        first = create('Cedar Household', root / 'Household.kabacus')
        driver.click(driver.wait_for('book File menu', lambda: driver.find(first, 'Cedar Household', pyatspi.ROLE_PUSH_BUTTON)))
        menu=driver.wait_for('New Window', lambda: driver.find(first, 'New Window'))
        driver.click(menu)
        second = create('Cedar Studio', root / 'Studio.kabacus')
        driver.press(driver.find(second, 'Transactions', pyatspi.ROLE_LIST_ITEM))
        state = root / 'config' / 'kabacus-windows.json'
        driver.wait_for('two saved windows', lambda: state.exists() and len(json.loads(state.read_text())['windows']) == 2)
    finally:
        process.terminate()
        process.wait(timeout=10)
    process = subprocess.Popen([binary, '--size=1280x800'], env=env, cwd=root)
    try:
        driver.wait_for('restored household', lambda: frame('Cedar Household — Kabacus'))
        restored = driver.wait_for('restored studio', lambda: frame('Cedar Studio — Kabacus'))
        driver.wait_for('restored Transactions page', lambda: driver.find(restored, 'Transactions', pyatspi.ROLE_PAGE_TAB))
        print('Local Welcome → New Books → Overview and two-window restoration passed')
    finally:
        process.terminate()
        process.wait(timeout=10)
