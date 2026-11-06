// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test, vi } from "vitest";
import { readFileSync } from "node:fs";
import { messages } from "./messages";
import { catalogs } from "../catalogs";
import { initializeLocale, t } from "./i18n";
test("all static React messages reuse the translated catalog vocabulary", () => {
  for (const file of [
    "main.jsx",
    "auth.js",
    "api.js",
    "reports.jsx",
    "operation-dialog.jsx",
    "operations.js",
    "accounts.jsx",
    "settings.jsx",
    "overview-details.jsx",
    "activity.jsx",
    "card-details.jsx",
    "workspace.jsx",
    "local-books.js",
    "navigation.jsx",
    "overview.jsx",
  ]) {
    const source = readFileSync(new URL(file, import.meta.url), "utf8");
    for (const match of source.matchAll(/\bt\((['"])(.*?)\1/g)) {
      const key = match[2].replace(/\\u([0-9a-f]{4})/gi, (_, hex) =>
        String.fromCharCode(parseInt(hex, 16)),
      );
      expect(messages, `${file}: ${key}`).toContain(key);
    }
  }
});
test("every existing language has complete selected translations", () => {
  const assets = [];
  catalogs().generateBundle.call({ emitFile: (asset) => assets.push(asset) });
  expect(assets).toHaveLength(66);
  for (const asset of assets) {
    const data = JSON.parse(asset.source);
    for (const message of messages)
      expect(data[message], `${asset.fileName}: ${message}`).toBeTruthy();
  }
});

test("native locale startup does not need an HTTP request or a supported system locale", async () => {
  const fetch = vi.fn().mockRejectedValue(new Error("No HTTP transport"));
  vi.stubGlobal("fetch", fetch);
  vi.stubGlobal("navigator", { languages: ["C", "en-US"] });
  try {
    await initializeLocale();
    expect(t("Overview")).toBe("Overview");
    expect(fetch).not.toHaveBeenCalled();
  } finally {
    vi.unstubAllGlobals();
  }
});
