<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Contributing

`README.md` covers building, running and testing. `CLAUDE.md` holds the
rules every change follows.

## Licence of contributions

Code is licensed as described in `REUSE.toml` and each file's SPDX header:
`kabacus-server` under AGPL-3.0-or-later; everything else, the shared
libraries the apps link included, under GPL-3.0-or-later; docs under
CC-BY-SA-4.0.

By submitting a contribution you certify the Developer Certificate of
Origin 1.1 (<https://developercertificate.org/>) and, in addition:

1. You license your contribution to the project under the licence of the
   files it touches, and
2. you grant Brannen Hall a perpetual, worldwide, non-exclusive,
   irrevocable, royalty-free licence to use, modify, sublicense and
   distribute your contribution under any licence, including a
   proprietary one. The project keeps the right to relicense in this way;
   you keep the copyright in your work.

## Signing off

Every commit carries a `Signed-off-by: Your Name <you@example.org>` line
whose email matches the commit's author. `git commit -s` adds it. Signing
off is your statement that both of the above apply. Only a person signs
off; AI agents never do.

`scripts/check-signoff` checks this, and CI runs it on every pull
request. Maintainers listed in `MAINTAINERS` are exempt. Merge commits
are not checked.
