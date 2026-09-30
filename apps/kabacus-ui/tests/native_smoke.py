# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Start the real WebKit/Tauri window and require its bundled React UI."""
import os
import subprocess
import sys
import time
import pyatspi


def walk(node, depth=0):
    if depth > 25:
        return
    try:
        yield node.name, node.getRoleName()
        try:
            content = node.queryText().getText(0, -1)
            if content:
                yield content, "text"
        except Exception:
            pass
        for child in node:
            yield from walk(child, depth + 1)
    except Exception:
        return


env = dict(os.environ, LANGUAGE="en", LANG="C.UTF-8", KABACUS_SERVER_URL="")
process = subprocess.Popen([sys.argv[1]], env=env)
try:
    deadline = time.monotonic() + 40
    last = []
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Native window exited: {process.returncode}")
        last = list(walk(pyatspi.Registry.getDesktop(0)))
        names = {name for name, _ in last}
        if {"Connection settings", "Server address", "Sign in"} <= names:
            print("Native WebKit window rendered the bundled connection settings")
            break
        time.sleep(0.25)
    else:
        raise RuntimeError(f"Connection UI was not accessible: {last!r}")
finally:
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
