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


from pathlib import Path
import tempfile


def require_window(process, expected):
    deadline = time.monotonic() + 40
    last = []
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Native window exited: {process.returncode}")
        last = list(walk(pyatspi.Registry.getDesktop(0)))
        names = {name.strip() for name, _ in last}
        if expected <= names:
            return
        time.sleep(0.25)
    raise RuntimeError(f"Native UI was not accessible: {last!r}")


def stop(process):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


with tempfile.TemporaryDirectory(prefix="kabacus-native-smoke-") as folder:
    root = Path(folder)
    env = dict(os.environ, LANGUAGE="en", LANG="C.UTF-8", RUST_LOG="info", MESA_SHADER_CACHE_DISABLE="true", XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"))
    binary = str(Path(sys.argv[1]).resolve())
    process = subprocess.Popen([binary], env=env)
    try:
        require_window(process, {"Welcome to Kabacus", "New Books…", "Open Books…", "Connect to a Server…"})
    finally:
        stop(process)
    desktop_log = root / "data" / "me.hallb.kabacus.web" / "logs" / "kabacus-desktop.log"
    logs = desktop_log.read_text(encoding="utf-8")
    assert "[INFO]" in logs and "desktop ready" in logs, logs
    assert "starting desktop" not in logs, logs
    sidecars = list((Path(__file__).resolve().parents[1] / "src-tauri" / "binaries").glob("kabacus-cli-*"))
    assert sidecars, "Build the packaged CLI first"
    books = root / "Household.kabacus"
    subprocess.run([str(sidecars[0]), "new", str(books), "--name", "Cedar Household"], check=True, stdout=subprocess.DEVNULL)
    process = subprocess.Popen([binary, str(books)], env=dict(env, RUST_LOG="debug"))
    try:
        require_window(process, {"Overview", "Available to spend", "On this computer"})
        print("Native WebKit Welcome and real local books rendered successfully")
    finally:
        stop(process)
    logs = desktop_log.read_text(encoding="utf-8")
    assert "[DEBUG]" in logs and "starting desktop" in logs, logs
    assert "Household" not in logs, logs

    # Fail startup with a synthetic book path while the file log is unavailable.
    desktop_log.unlink()
    desktop_log.parent.rmdir()
    desktop_log.parent.touch()
    failed = subprocess.run(
        [binary, str(root / "Marina-Lopez.kabacus")],
        env=dict(env, RUST_LOG="warn"),
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
    )
    assert failed.returncode == 1, failed.stderr
    assert "[ERROR] desktop startup failed: open startup books" in failed.stderr, failed.stderr
    assert "Marina-Lopez" not in failed.stderr, failed.stderr
    assert "starting desktop" not in failed.stderr and "desktop ready" not in failed.stderr, failed.stderr
