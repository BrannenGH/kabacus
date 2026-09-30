// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useState } from "react";
import { t } from "./i18n";
export function Settings({ data, api, auth }) {
  const [server, setServer] = useState(location.origin);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  async function test() {
    setBusy(true);
    setStatus("");
    try {
      const version = await api.request("/version", {
        signal: AbortSignal.timeout(10000),
      });
      setStatus(t("Yes (%1)", version.server_version));
    } catch (e) {
      setStatus(e.message);
    } finally {
      setBusy(false);
    }
  }
  function connect(event) {
    event.preventDefault();
    if (window.__KABACUS_NATIVE__) {
      location.assign("/app/?native-settings=1");
      return;
    }
    try {
      const url = new URL(server);
      if (url.protocol !== "https:" || url.username || url.password)
        throw new Error(t("The server address must start with https://"));
      location.assign(`${url.origin}/app/`);
    } catch (e) {
      setStatus(e.message);
    }
  }
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>{t("Connection settings")}</h2>
      </div>
      <form className="filters" onSubmit={connect}>
        <label>
          {t("Server address")}
          <input
            value={server}
            onChange={(e) => setServer(e.target.value)}
            type="url"
            required
          />
        </label>
        <button>{t("Go")}</button>
        <button
          type="button"
          className="secondary"
          onClick={test}
          disabled={busy}
        >
          {t("Test connection")}
        </button>
      </form>
      {status && (
        <p className="banner" role="status">
          {status}
        </p>
      )}
      <div className="panel-heading">
        <h2>{t("Account")}</h2>
        {auth.signOut && (
          <button onClick={auth.signOut}>{t("Sign out")}</button>
        )}
      </div>
      <div className="list-row">
        <span>{t("Signed in:")}</span>
        {auth.identity}
      </div>
      <p className="empty">
        {data.version.auth.mode === "none"
          ? t("Authentication off (local mode)")
          : data.version.auth.issuer}
      </p>
      <div className="panel-heading">
        <h2>{t("About")}</h2>
      </div>
      <dl className="home">
        <div>
          <dt>{t("Server version:")}</dt>
          <dd>{data.version.server_version}</dd>
        </div>
        <div>
          <dt>{t("Commit:")}</dt>
          <dd>{data.version.commit}</dd>
        </div>
        <div>
          <dt>{t("Source code:")}</dt>
          <dd>
            {data.version.source_url?.startsWith("https://") ? (
              <a
                href={data.version.source_url}
                rel="noreferrer"
                target="_blank"
              >
                {t("Open the source code")}
              </a>
            ) : (
              data.version.source_url
            )}
          </dd>
        </div>
      </dl>
    </section>
  );
}
