// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import catalogs from "virtual:kabacus-locales";
let catalog = {};
export function t(message, ...values) {
  const key = message;
  const text = catalog[key] || key;
  return text.replace(
    /%([1-9]\d*)/g,
    (match, index) => values[Number(index) - 1] ?? match,
  );
}
export async function initializeLocale() {
  catalog = {};
  const languages = navigator.languages || [navigator.language];
  for (const language of languages) {
    const locale = language.replace("-", "_");
    for (const candidate of [...new Set([locale, locale.split("_")[0]])]) {
      if (candidate === "en") return;
      if (catalogs[candidate]) {
        catalog = catalogs[candidate];
        document.documentElement.lang = language;
        document.documentElement.dir = t("LTR") === "RTL" ? "rtl" : "ltr";
        return;
      }
    }
  }
}
