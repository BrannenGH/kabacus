// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { t } from "./i18n";
export function Activity({ api, data, onOpen }) {
  const [actor, setActor] = useState("");
  const [selected, setSelected] = useState("");
  const [diff, setDiff] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    setDiff("");
    setError("");
    if (!selected) return;
    const c = new AbortController();
    api
      .text(`/history/${encodeURIComponent(selected)}`, { signal: c.signal })
      .then(setDiff)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => c.abort();
  }, [api, selected]);
  const changes = data.history.filter((change) => !actor || change.actor === actor);
  return (
    <section className="panel">
      <div className="filters">
        <label>
          {t("Activity")}
          <select value={actor} onChange={(e) => setActor(e.target.value)}>
            {[
              ["", "All"],
              ["app", "This app"],
              ["cli", "kabacus-cli"],
              ["mcp", "MCP"],
              ["external", "External edit"],
            ].map(([value, label]) => (
              <option key={value} value={value}>
                {value === "cli" ? label : t(label)}
              </option>
            ))}
          </select>
        </label>
      </div>
      {changes.map((change) => (
          <div className="list-row" key={change.hash}>
            <button className="quiet" onClick={() => setSelected(change.hash)}>
              {change.revision}
            </button>
            <div className="grow">
              <h3>
                <button
                  className="quiet"
                  onClick={() => setSelected(change.hash)}
                >
                  {change.summary}
                </button>
              </h3>
              <small>
                {change.agent && <span className="agent-badge">{t("Agent")}: {change.agent} · </span>}
                {t("%1 · %2", change.user || change.actor || "", localTime(change.date))}
              </small>
            </div>
            {/\[#(\d+)\]/.test(change.summary) && (
              <button
                className="secondary"
                onClick={() => {
                  const id = Number(change.summary.match(/\[#(\d+)\]/)[1]);
                  const tx = data.transactions.find((tx) => tx.id === id);
                  if (tx) onOpen({ kind: "edit", tx });
                }}
              >
                {t("Open transaction")}
              </button>
            )}
          </div>
        ))}
      {!changes.length && <p className="empty">{t("No changes")}</p>}
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {selected && (
        <div className="preview">
          <h3>{t("What changes")}</h3>
          <pre>{diff || t("Loading…")}</pre>
        </div>
      )}
    </section>
  );
}
// Matches the KDE app: the viewer's local time, to the minute.
function localTime(date) {
  const d = new Date(date);
  if (isNaN(d)) return date;
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
