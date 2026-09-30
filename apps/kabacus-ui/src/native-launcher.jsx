// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useState } from "react";
import { t } from "./i18n";
export function NativeLauncher() {
  const [server, setServer] = useState(
    window.__KABACUS_DEFAULT_SERVER__ ||
      localStorage.getItem("kabacus.server") ||
      "",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function connect(event) {
    event.preventDefault();
    setError("");
    setBusy(true);
    try {
      const url = new URL(server);
      if (
        url.protocol !== "https:" ||
        url.username ||
        url.password ||
        url.pathname !== "/" ||
        url.search ||
        url.hash
      )
        throw new Error(t("The server address must start with https://"));
      await window.__TAURI_INTERNALS__.invoke("connect_server", {
        origin: url.origin,
      });
      localStorage.setItem("kabacus.server", url.origin);
    } catch (error) {
      setError(String(error.message || error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="signin">
      <p className="eyebrow">Kabacus</p>
      <h1>{t("Connection settings")}</h1>
      <form onSubmit={connect}>
        <label>
          {t("Server address")}
          <input
            type="url"
            value={server}
            onChange={(e) => setServer(e.target.value)}
            placeholder="https://books.example.org"
            required
          />
        </label>
        <button disabled={busy}>
          {t(busy ? "Checking with the server…" : "Sign in")}
        </button>
      </form>
      {error && (
        <div className="banner error" role="alert">
          <p>{t("Can't reach kabacus-server")}</p>
          <details>
            <summary>{t("Details")}</summary>
            <pre>{error}</pre>
          </details>
        </div>
      )}
    </main>
  );
}
