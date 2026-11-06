// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { t } from "./i18n";
export function OverviewDetails({ api, data }) {
  const [history, setHistory] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => {
    const c = new AbortController();
    const date = new Date();
    const to = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
    const from = `${date.getFullYear() - 1}-${String(date.getMonth() + 1).padStart(2, "0")}-01`;
    api
      .request(
        `/reports/cash-on-hand-history?from=${from}&to=${to}&interval=month`,
        { signal: c.signal },
      )
      .then(setHistory)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => c.abort();
  }, [api, data.version.head]);
  let points = [];
  if (history?.points?.length) {
    const values = history.points.map((p) => {
      const [whole, fraction = ""] = p.cash_on_hand
        .replace(/^-/, "")
        .split(".");
      return {
        negative: p.cash_on_hand.startsWith("-"),
        digits: whole + fraction,
        places: fraction.length,
      };
    });
    const places = Math.max(...values.map((v) => v.places));
    const integers = values.map(
      (v) =>
        BigInt(v.digits) *
        10n ** BigInt(places - v.places) *
        (v.negative ? -1n : 1n),
    );
    const low = integers.reduce((a, b) => (a < b ? a : b));
    const high = integers.reduce((a, b) => (a > b ? a : b));
    const range = high - low || 1n;
    points = integers.map((value, i) => {
      const x = 20 + (i * 560) / Math.max(integers.length - 1, 1);
      return `${document.documentElement.dir === "rtl" ? 600 - x : x},${180 - Number(((value - low) * 160n) / range)}`;
    });
  }
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>{t("Cash on Hand over time")}</h2>
      </div>
      {error && <p className="banner error">{error}</p>}
      {history && (
        <>
          <svg
            role="img"
            aria-label={t("Cash on Hand over time")}
            viewBox="0 0 600 200"
            className="cash-chart"
          >
            <polyline
              points={points.join(" ")}
              fill="none"
              stroke="currentColor"
              strokeWidth="3"
            />
          </svg>
          <details className="home">
            <summary>{t("Details")}</summary>
            <table>
              <thead>
                <tr>
                  <th>{t("Date")}</th>
                  <th>{t("Cash on Hand")}</th>
                </tr>
              </thead>
              <tbody>
                {history.points.map((point) => (
                  <tr key={point.date}>
                    <td>{point.date}</td>
                    <td>
                      {history.commodity} {point.cash_on_hand}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </details>
        </>
      )}
    </section>
  );
}
