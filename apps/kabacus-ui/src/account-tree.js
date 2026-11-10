// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later

export const words = segment => segment.replace(/-/g, " ").replace(/^./, c => c.toLocaleUpperCase());
const parts = value => value.split(":");

// One level of the account tree: the accounts directly in `prefix` and the
// groups below it. Options are { path: account path, label, value }.
export function browse(options, prefix) {
  const rows = [];
  const seen = new Set();
  for (const option of options) {
    const path = parts(option.path);
    if (path.length <= prefix.length || prefix.some((segment, i) => path[i] !== segment)) continue;
    const next = path[prefix.length];
    if (path.length === prefix.length + 1) rows.push({ option });
    else if (!seen.has(next)) {
      seen.add(next);
      rows.push({ group: [...prefix, next], label: words(next) });
    }
  }
  return rows;
}

// Matches at every level, each with the group it lives in.
export function search(options, query) {
  const q = query.toLocaleLowerCase();
  return options
    .filter(option => `${option.label} ${option.path}`.toLocaleLowerCase().includes(q))
    .map(option => ({ option, path: parts(option.path).slice(0, -1).map(words).join(" › ") }));
}
