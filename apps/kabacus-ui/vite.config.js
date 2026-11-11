// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { catalogs } from "./catalogs";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { createHash } from "node:crypto";
const appVersion = process.env.KABACUS_VERSION || JSON.parse(readFileSync("package.json", "utf8")).version;
const tokens = JSON.parse(readFileSync(new URL("../../design/tokens.json", import.meta.url), "utf8"));
const phone = Number(tokens.breakpoint.phone.value), desktop = Number(tokens.breakpoint.desktop.value);
function breakpoints() {
  return { name: "kabacus-breakpoints", enforce: "pre", transform(source, id) {
    if (!id.endsWith(".css")) return;
    return source.replaceAll("(--phone)", `(width < ${phone}px)`)
      .replaceAll("(--tablet)", `(${phone}px <= width < ${desktop}px)`)
      .replaceAll("(--compact)", `(width < ${desktop}px)`)
      .replaceAll("(--desktop)", `(width >= ${desktop}px)`);
  } };
}
function pwa() {
 return {name:"kabacus-pwa",closeBundle(){
  const root="dist";
  for(const font of ["inter","playfair-display"]){writeFileSync(join(root,`${font}-LICENSE.txt`),readFileSync(`node_modules/@fontsource/${font}/LICENSE`));}
  function files(dir){return readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?files(join(dir,e.name)):[join(dir,e.name)]);}
  const assets=files(root).filter(path=>!path.endsWith("sw.js")).map(path=>relative(root,path));
  const version=createHash("sha256").update(Buffer.concat(assets.map(path=>readFileSync(join(root,path))))).digest("hex").slice(0,16);
  const source=readFileSync("sw-source.js","utf8").replace("__KABACUS_ASSETS__",JSON.stringify(assets)).replace("__KABACUS_VERSION__",JSON.stringify(version));
  writeFileSync(join(root,"sw.js"),source);
 }};
}
export default defineConfig(({ command }) => ({
  base: command === "build" ? "./" : "/app/",
  define: { __APP_VERSION__: JSON.stringify(appVersion) },
  plugins: [breakpoints(), react(), catalogs(), pwa()],
  server: { port: 5173, strictPort: true },
  test: { include: ["src/**/*.test.js"] },
}));
