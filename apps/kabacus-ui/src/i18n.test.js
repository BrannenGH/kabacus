// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, expect, test, vi } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { messages } from "./messages";
import { catalogs } from "../catalogs";
import { initializeLocale, t } from "./i18n";

afterEach(() => vi.unstubAllGlobals());

function environment(languages, language) {
  const documentElement = {};
  vi.stubGlobal("navigator", { languages, language });
  vi.stubGlobal("document", { documentElement });
  return documentElement;
}

const assets = new Map();
catalogs().generateBundle.call({
  emitFile: ({ fileName, source }) => assets.set(
    fileName.split("/").at(-1).replace(".json", ""),
    JSON.parse(source),
  ),
});
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
    "phone-picker.jsx",
    "phone-history.js",
    "statement-import.jsx",
    "bank-connections.jsx",
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
test("every KDE catalog is reachable from browser preferences", async () => {
  const browserTags = {
    "ca@valencia": "ca-ES-valencia",
    "sr@ijekavian": "sr-Cyrl-ijekavsk",
    "sr@ijekavianlatin": "sr-Latn-ijekavsk",
    "sr@latin": "sr-Latn",
  };
  const files = readdirSync(new URL("../../../po/", import.meta.url))
    .filter(file => file.endsWith(".po")).map(file => file.slice(0, -3));
  expect([...assets.keys()]).toEqual(files);
  for (const [id, data] of assets) {
    const documentElement = environment([browserTags[id] || id.replace("_", "-")]);
    await initializeLocale();
    for (const message of messages) {
      expect(data[message], `${id}: ${message}`).toBeTruthy();
      expect(t(message), `${id}: ${message}`).toBe(data[message]);
    }
    expect(documentElement.dir, id).toBe(data.LTR.toLowerCase());
  }
});

test.each([
  [["zz", "de-DE"], "de", "de"],
  [["en-US", "ar"], "en", "en"],
  [["en-GB"], "en_GB", "en-GB"],
  [["pt-Latn-br-u-hc-h23"], "pt_BR", "pt-BR"],
  [["pt-PT"], "pt", "pt"],
  [["no-NO"], "nb", "nb"],
  [["ca-ES-valencia"], "ca@valencia", "ca-valencia"],
  [["sr-Latn-RS"], "sr@latin", "sr-Latn"],
  [["sr-ijekavsk"], "sr@ijekavian", "sr-Cyrl-ijekavsk"],
  [["sr-Latn-ME-ijekavsk"], "sr@ijekavianlatin", "sr-Latn-ijekavsk"],
  [["zh"], "zh_CN", "zh-Hans"],
  [["zh-Hans-SG"], "zh_CN", "zh-Hans"],
  [["zh-Hant"], "zh_TW", "zh-Hant"],
  [["zh-HK"], "zh_TW", "zh-Hant"],
])("matches browser preferences %j to %s", async (languages, id, tag) => {
  const documentElement = environment(languages);
  await initializeLocale();
  const translations = assets.get(id);
  expect(t("Welcome to Kabacus")).toBe(translations?.["Welcome to Kabacus"] || "Welcome to Kabacus");
  expect(documentElement.lang).toBe(tag);
});

test("uses the primary language when the browser sends an empty preference list", async () => {
  const documentElement = environment([], "ar-EG");
  await initializeLocale();
  expect(documentElement).toEqual({ lang: "ar", dir: "rtl" });
});

test("native locale startup does not need an HTTP request or a supported system locale", async () => {
  const fetch = vi.fn().mockRejectedValue(new Error("No HTTP transport"));
  vi.stubGlobal("fetch", fetch);
  const documentElement = environment(["C", "zz"]);
  await initializeLocale();
  expect(t("Overview")).toBe("Overview");
  expect(documentElement).toEqual({ lang: "en", dir: "ltr" });
  expect(fetch).not.toHaveBeenCalled();
});
