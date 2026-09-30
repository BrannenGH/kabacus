// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { t } from "./i18n";
const kinds = {
  "cash-flow": "Cash flow",
  "income-statement": "Income statement",
  categories: "Categories",
};
export function Reports({ api }) {
  const [month, setMonth] = useState(
    () =>
      `${new Date().getFullYear()}-${String(new Date().getMonth() + 1).padStart(2, "0")}`,
  );
  const [kind, setKind] = useState("cash-flow");
  const [principal, setPrincipal] = useState(true);
  const [result, setResult] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    setResult(null);
    setError("");
    const [year, number] = month.split("-").map(Number);
    if (!year || !number) return () => controller.abort();
    const to = new Date(year, number, 0).getDate();
    const query = new URLSearchParams({
      from: `${month}-01`,
      to: `${month}-${to}`,
      equity_conversion: String(principal),
      compare_previous: "true",
    });
    api
      .request(`/reports/${kind}?${query}`, { signal: controller.signal })
      .then(setResult)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => controller.abort();
  }, [api, month, kind, principal]);
  const value = (text) => (
    <span className="amount">
      {result?.commodity} {String(text ?? "—")}
    </span>
  );
  const rows = (list, columns) => (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>{t("Account")}</th>
            {columns.map(([key, label]) => (
              <th key={key}>
                {t(label, result?.previous_period?.label || "—")}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {list.map((row, index) => (
            <tr key={row.account || index}>
              <td>{row.display_name || row.account || row.group}</td>
              {columns.map(([key]) => (
                <td key={key}>{value(row[key])}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {!list.length && <p className="empty">{t("No transactions")}</p>}
    </div>
  );
  return (
    <section className="panel">
      <div className="filters">
        <label>
          {t("Reports")}
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            {Object.entries(kinds).map(([key, label]) => (
              <option value={key} key={key}>
                {t(label)}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("Date")}
          <input
            type="month"
            required
            value={month}
            onChange={(e) => setMonth(e.target.value)}
          />
        </label>
        {kind === "income-statement" && (
          <label className="checkbox">
            <input
              type="checkbox"
              checked={principal}
              onChange={(e) => setPrincipal(e.target.checked)}
            />
            {t("Count loan principal as money spent")}
          </label>
        )}
      </div>
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {!result ? (
        !error && <p className="empty">{t("Loading…")}</p>
      ) : (
        <>
          {kind === "cash-flow" && (
            <>
              <div className="list-row">
                <span>{t("Opening")}</span>
                {value(result.opening_cash)}
              </div>
              {rows(result.lines || [], [
                ["cash", "Cash"],
                ["non_cash", "Not cash"],
              ])}
              <div className="list-row">
                <span>{t("Closing")}</span>
                {value(result.closing_cash)}
              </div>
            </>
          )}
          {kind === "categories" &&
            rows(result.rows || [], [
              ["amount", "Amount"],
              ["change", "vs %1"],
            ])}
          {kind === "income-statement" && (
            <>
              {[
                ["income", "Income"],
                ["expenses", "Spending"],
              ].map(([key, label]) => (
                <section key={key}>
                  <div className="panel-heading">
                    <h2>{t(label)}</h2>
                    {value(result.totals?.[key]?.[0])}
                  </div>
                  {rows(result[key] || [], [["total", "Amount"]])}
                </section>
              ))}
              {result.equity_conversion && (
                <section>
                  <div className="panel-heading">
                    <h2>{t("Loan principal")}</h2>
                    {value(result.equity_conversion.totals?.[0])}
                  </div>
                  {rows(result.equity_conversion.rows || [], [
                    ["total", "Amount"],
                  ])}
                </section>
              )}
              <div className="list-row">
                <span>{t("Net:")}</span>
                {value(result.net?.[0])}
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}
