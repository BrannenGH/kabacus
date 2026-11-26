// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import catalogs from "virtual:kabacus-locales";

const variantTags = {
  "ca@valencia": "ca-valencia",
  sr: "sr-Cyrl",
  "sr@latin": "sr-Latn",
  "sr@ijekavian": "sr-Cyrl-ijekavsk",
  "sr@ijekavianlatin": "sr-Latn-ijekavsk",
  zh_CN: "zh-Hans",
  zh_TW: "zh-Hant",
};

let catalog = {};
export function t(message, ...values) {
  const key = message;
  const text = catalog[key] || key;
  return text.replace(
    /%([1-9]\d*)/g,
    (match, index) => values[Number(index) - 1] ?? match,
  );
}
function browserLocale() {
  const languages = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of languages) {
    if (!tag) continue;
    let locale;
    try {
      locale = new Intl.Locale(tag);
    } catch {
      continue;
    }
    const { language, region, baseName } = locale;
    const subtags = baseName.split("-");
    if (language === "ca" && subtags.includes("valencia")) return "ca@valencia";
    if (language === "zh") return locale.maximize().script === "Hant" ? "zh_TW" : "zh_CN";
    if (language === "sr") {
      const latin = locale.maximize().script === "Latn";
      if (subtags.includes("ijekavsk")) return latin ? "sr@ijekavianlatin" : "sr@ijekavian";
      return latin ? "sr@latin" : "sr";
    }
    const regional = `${language}_${region}`;
    if (Object.hasOwn(catalogs, regional)) return regional;
    if (language === "no") return "nb";
    if (language === "en" || Object.hasOwn(catalogs, language)) return language;
  }
  return "en";
}

export async function initializeLocale() {
  const id = browserLocale();
  catalog = catalogs[id] || {};
  document.documentElement.lang = variantTags[id] || id.replace("_", "-");
  document.documentElement.dir = t("LTR") === "RTL" ? "rtl" : "ltr";
}
