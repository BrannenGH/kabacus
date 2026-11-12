<!-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me> -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Project skills

The full [upstream skill collection](https://github.com/qa-aman/claude-skills),
[Ponytail](https://github.com/DietrichGebert/ponytail) and
[Bencium's design skills](https://github.com/bencium/bencium-claude-code-design-skill)
are pinned as Git submodules in `vendor/claude-skills`, `vendor/ponytail` and
`vendor/bencium-design`. The UI skills come from four more pinned submodules:

| Submodule | Skills linked |
| --- | --- |
| `vendor/anthropic-skills` ([Anthropic](https://github.com/anthropics/skills)) | `frontend-design` |
| `vendor/vercel-agent-skills` ([Vercel](https://github.com/vercel-labs/agent-skills)) | `web-design-guidelines`, `react-best-practices`, `composition-patterns`, `react-native-skills` |
| `vendor/ui-ux-pro-max` ([UI/UX Pro Max](https://github.com/nextlevelbuilder/ui-ux-pro-max-skill)) | `ui-ux-pro-max` |
| `vendor/accesslint` ([AccessLint](https://github.com/accesslint/claude-marketplace)) | `accessibility-audit`, `-scan`, `-inspect`, `-fix`, `-diff` |

Only those skills are linked from the two general collections. Each
submodule's license (MIT, or Apache-2.0 for `frontend-design`) and supporting
resources stay with it. `skills/` exposes each skill by name;
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
  .agents/vendor/bencium-design .agents/vendor/anthropic-skills \
  .agents/vendor/vercel-agent-skills .agents/vendor/ui-ux-pro-max .agents/vendor/accesslint
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
existing design system, `web-design-guidelines` and the `accessibility-*`
skills for a web UI review, and `react-best-practices` for the web app's React
code. These are instruction files;
syncing them installs no lifecycle hooks or background services.
