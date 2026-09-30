# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# Lets the host's gdb attach to kabacus or kabacus-server while they run
# in a podman container (scripts/dev). Host gdb can ptrace them
# (both run in user namespaces this user owns), but it can't read their
# libraries through its default `target:` sysroot, which needs to enter
# their mount namespace. /proc/PID/root reaches the same files. Source it
# before attaching:
#   gdb -iex 'source scripts/gdb-attach.py' -p PID
import gdb


def _use_process_root(event):
    gdb.events.stop.disconnect(_use_process_root)
    root = f"/proc/{gdb.selected_inferior().pid}/root"
    gdb.execute(f"add-auto-load-safe-path {root}")
    gdb.execute(f"set sysroot {root}")
    gdb.execute("sharedlibrary")


gdb.events.stop.connect(_use_process_root)
