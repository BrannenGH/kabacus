// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

// The KDE app installs the packaging copy as its theme icon; the web app,
// PWA and Tauri shell use the other two. The website shows the same mark.
describe("app icon", () => {
  const icons = {
    web: read("./icon.svg"),
    pwa: read("../public/icon.svg"),
    kde: read("../../../packaging/desktop/me.hallb.kabacus.svg"),
  };

  it("is the same artwork in every app", () => {
    expect(icons.pwa).toBe(icons.web);
    expect(icons.kde).toBe(icons.web);
  });

  it("uses the red tile and gold rails of the Kabacus website", () => {
    expect(icons.web).toContain('fill="#a53236"');
    expect(icons.web).toContain('fill="#dbb486"');
    expect(icons.web).not.toMatch(/#304e41/i);
  });
});
