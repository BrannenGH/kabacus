#!/usr/bin/env python3

# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""Start a Secret Service for the GUI e2e session, then run a command.

    secret-service.py COMMAND...

The app keeps its refresh token in the platform secret store: KWallet on
Plasma, the only desktop it targets. It serves org.freedesktop.secrets;
this starts it, unlocked and with a fresh wallet in a temporary directory,
on the session bus the tests run on. For tests only: the wallet "password"
is fixed.
"""
import os
import socket
import subprocess
import sys
import tempfile
import time


def fresh_dirs():
    """Its own config and data directories, so no run sees an old wallet."""
    root = tempfile.mkdtemp(prefix="kabacus-secrets-")
    env = dict(os.environ,
               XDG_CONFIG_HOME=os.path.join(root, "config"),
               XDG_DATA_HOME=os.path.join(root, "data"))
    os.makedirs(env["XDG_CONFIG_HOME"])
    os.makedirs(env["XDG_DATA_HOME"])
    return env


def start_kwallet():
    """ksecretd, with its wallet opened the way pam_kwallet opens it at
    login: `--pam-login <pipe> <socket>`, the password hash on the pipe,
    then the session environment over the socket."""
    env = fresh_dirs()
    with open(os.path.join(env["XDG_CONFIG_HOME"], "kwalletrc"), "w") as f:
        f.write("[Wallet]\nFirst Use=false\nEnabled=true\nUse One Wallet=true\n")
    # Never a dialog: one would wait forever on the test display.
    env["QT_QPA_PLATFORM"] = "offscreen"
    hash_size = 56  # PBKDF2_SHA512_KEYSIZE in kwalletbackend
    read_end, write_end = os.pipe()
    os.write(write_end, b"kabacus-test-wallet".ljust(hash_size, b"\0"))
    os.close(write_end)
    path = os.path.join(env["XDG_DATA_HOME"], "pam.sock")
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(path)
    listener.listen(1)
    env["PAM_KWALLET5_LOGIN"] = path
    subprocess.Popen(
        ["ksecretd", "--pam-login", str(read_end), str(listener.fileno())],
        pass_fds=(read_end, listener.fileno()), env=env)
    os.close(read_end)
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(path)
        s.sendall(b"".join(f"{k}={os.environ[k]}\n".encode()
                           for k in ("DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR")
                           if k in os.environ))
    listener.close()


def wait_for_service():
    for _ in range(100):
        if subprocess.run(["busctl", "--user", "status", "org.freedesktop.secrets"],
                          capture_output=True).returncode == 0:
            return
        time.sleep(0.1)
    sys.exit("secret-service.py: no Secret Service came up")


def main():
    if len(sys.argv) < 2:
        sys.exit("usage: secret-service.py COMMAND...")
    start_kwallet()
    wait_for_service()
    os.execvp(sys.argv[1], sys.argv[1:])


main()
