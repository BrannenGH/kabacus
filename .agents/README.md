<!-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me> -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Project skills

The full [upstream skill collection](https://github.com/qa-aman/claude-skills)
is pinned as a Git submodule in `vendor/claude-skills`. Its MIT license and
supporting resources stay with it. `skills/` exposes each skill by name;
`.claude/skills` points to the same directory. The repository's contribution
rules and the user's instructions take precedence over skill guidance.

After cloning:

```sh
git submodule update --init --recursive
scripts/sync-skills
```

To review an upstream update:

```sh
git submodule update --init --remote .agents/vendor/claude-skills
scripts/sync-skills
git diff --submodule=log
```

Commit the updated submodule pin and any changed skill links together.
The sync script requires Python 3 and refuses to overwrite local entries.
Use a checkout with symbolic-link support; on Windows enable Developer Mode
and Git's `core.symlinks` before cloning.
