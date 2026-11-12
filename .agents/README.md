<!-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me> -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Project skills

The full [upstream skill collection](https://github.com/qa-aman/claude-skills),
[Ponytail](https://github.com/DietrichGebert/ponytail) and
[Bencium's design skills](https://github.com/bencium/bencium-claude-code-design-skill)
are pinned as Git submodules in `vendor/claude-skills`, `vendor/ponytail` and
`vendor/bencium-design`. Their MIT licenses
and supporting resources stay with them. `skills/` exposes each skill by name;
`.claude/skills` points to the same directory. The repository's contribution
rules and the user's instructions take precedence over skill guidance.

After cloning:

```sh
git submodule update --init --recursive
scripts/sync-skills
```

To review an upstream update:

```sh
git submodule update --init --remote .agents/vendor/claude-skills .agents/vendor/ponytail \
  .agents/vendor/bencium-design
scripts/sync-skills
git diff --submodule=log
```

Commit the updated submodule pin and any changed skill links together.
The sync script requires Python 3 and refuses to overwrite local entries.
Use a checkout with symbolic-link support; on Windows enable Developer Mode
and Git's `core.symlinks` before cloning.

Use `ponytail` for small, complete changes, `ponytail-review` for a diff,
and `ponytail-audit` for a repository review. Use
`bencium-controlled-ux-designer` for design reviews that stay within the
existing design system. These are instruction files;
syncing them installs no lifecycle hooks or background services.
