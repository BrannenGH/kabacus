// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { catalogs } from "./catalogs";
export default defineConfig(({ command }) => ({
  base: command === "build" ? "./" : "/app/",
  plugins: [react(), catalogs()],
  server: { port: 5173, strictPort: true },
  test: { include: ["src/**/*.test.js"] },
}));
