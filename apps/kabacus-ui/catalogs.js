// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import gettext from "gettext-parser";
import { messages } from "./src/messages.js";
export function catalogs() {
  const root = fileURLToPath(new URL("../../po/", import.meta.url));
  const assets = new Map();
  for (const file of readdirSync(root).filter((file) => file.endsWith(".po"))) {
    const po = gettext.po.parse(readFileSync(`${root}/${file}`));
    const entries = Object.values(po.translations).flatMap((context) =>
      Object.values(context),
    );
    const translated = {};
    for (const message of messages) {
      const entry = entries.find(
        (entry) =>
          entry.msgid === message &&
          !entry.comments?.flag?.includes("fuzzy") &&
          entry.msgstr[0],
      );
      if (!entry)
        throw new Error(`${file}: missing translation for ${message}`);
      translated[message] = entry.msgstr[0];
    }
    assets.set(
      `/app/assets/i18n/${file.replace(/\.po$/, ".json")}`,
      JSON.stringify(translated),
    );
  }
  return {
    name: "kabacus-catalogs",
    resolveId(id) {
      if (id === "virtual:kabacus-locales") return "\0kabacus-locales";
    },
    load(id) {
      if (id !== "\0kabacus-locales") return;
      const catalogs = Object.fromEntries(
        [...assets].map(([path, source]) => [
          path.split("/").at(-1).replace(".json", ""),
          JSON.parse(source),
        ]),
      );
      return `export default ${JSON.stringify(catalogs)};`;
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (!req.url.startsWith("/app/assets/i18n/")) return next();
        const content = assets.get(req.url);
        res.statusCode = content ? 200 : 404;
        res.setHeader("Content-Type", "application/json");
        res.end(content || "{}");
      });
    },
    generateBundle() {
      for (const [path, source] of assets)
        this.emitFile({
          type: "asset",
          fileName: path.replace("/app/", ""),
          source,
        });
    },
  };
}
