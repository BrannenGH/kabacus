// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { t } from "./i18n";
export function Accounts({ data, loans = false, onOpen }) {
  const nonzero = (a) =>
    (a.balance_inclusive || a.balances || []).some(
      (b) => !/^[-+]?0(?:\.0+)?$/.test(b.exact),
    );
  const labels = {
    money: "Money",
    "revolving-debt": "Cards",
    "term-liability": "Loans",
    receivable: "People",
    payable: "People",
    expense: "Spending",
    income: "Income",
    "non-liquid": "Investments",
  };
  const groups = [...Object.entries(labels), ["other", "Other"]].filter(
    ([kind]) => !loans || kind === "term-liability",
  );
  return (
    <section className="panel">
      {groups.map(([kind, label]) => {
        const accounts = data.accounts
          .filter((a) => nonzero(a))
          .filter((a) =>
            kind === "other"
              ? !labels[a.kind] &&
                !["parent", "pre-allocation", "debt-reserve"].includes(a.kind)
              : a.kind === kind,
          );
        if (!accounts.length) return null;
        return (
          <section key={kind}>
            <div className="panel-heading">
              <h2>{t(label)}</h2>
            </div>
            {accounts.map((a) => (
              <div className="list-row" key={a.account}>
                <button className="quiet" onClick={() => onOpen(a.account)}>
                  {a.display_name || a.account}
                </button>
                <span className="amount">
                  {(a.balance_inclusive?.length
                    ? a.balance_inclusive
                    : a.balances || []
                  )
                    .map((b) => b.formatted)
                    .join(" / ")}
                </span>
                {data.accounts
                  .filter((child) => child.home === a.account && nonzero(child))
                  .map((child) => (
                    <button
                      key={child.account}
                      className="quiet"
                      onClick={() => onOpen(child.account)}
                    >
                      {child.display_name || child.account}
                    </button>
                  ))}
              </div>
            ))}
          </section>
        );
      })}
    </section>
  );
}
export function People({ api }) {
  const [people, setPeople] = useState(null);
  const [person, setPerson] = useState("");
  const [statement, setStatement] = useState(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState("");
  const now = new Date();
  const [from, setFrom] = useState(`${now.getFullYear() - 1}-01-01`);
  const [to, setTo] = useState(
    `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`,
  );
  useEffect(() => {
    const c = new AbortController();
    api
      .request("/counterparties", { signal: c.signal })
      .then(setPeople)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => c.abort();
  }, [api]);
  const query = new URLSearchParams({ from, to, open_items: String(open) });
  const path = person
    ? `/counterparties/${encodeURIComponent(person)}/statement?${query}`
    : "";
  useEffect(() => {
    setStatement(null);
    setError("");
    if (!path) return;
    const c = new AbortController();
    api
      .request(path, { signal: c.signal })
      .then(setStatement)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => c.abort();
  }, [api, path]);
  async function download(format) {
    try {
      await api.download(`${path}&format=${format}`, `statement.${format}`);
    } catch (e) {
      setError(e.message);
    }
  }
  return (
    <section className="panel">
      <div className="filters">
        <label>
          {t("People")}
          <select value={person} onChange={(e) => setPerson(e.target.value)}>
            <option value="">{t("All")}</option>
            {people?.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        {person && (
          <>
            <label>
              {t("From")}
              <input
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </label>
            <label>
              {t("To")}
              <input
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={open}
                onChange={(e) => setOpen(e.target.checked)}
              />
              {t("Open items only")}
            </label>
          </>
        )}
      </div>
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {!person ? (
        people ? (
          people
            .filter((p) => !/^[-+]?0(?:\.0+)?$/.test(p.net.exact))
            .map((p) => (
              <div className="list-row" key={p.name}>
                <button className="quiet" onClick={() => setPerson(p.name)}>
                  {p.name}
                </button>
                <span className="amount">{p.net.formatted}</span>
                <span>{p.label}</span>
              </div>
            ))
        ) : (
          <p className="empty">{t("Loading…")}</p>
        )
      ) : statement ? (
        <>
          <div className="panel-heading">
            <h2>{statement.name}</h2>
            <button className="secondary" onClick={() => download("csv")}>
              CSV
            </button>
            <button className="secondary" onClick={() => download("pdf")}>
              PDF
            </button>
          </div>
          <div className="list-row">
            <span>{t("Opening")}</span>
            <span className="amount">
              {statement.opening_balance.formatted}
            </span>
          </div>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{t("Date")}</th>
                  <th>{t("Payee")}</th>
                  <th>{t("Amount")}</th>
                  <th>{t("Money")}</th>
                </tr>
              </thead>
              <tbody>
                {statement.lines.map((line, i) => (
                  <tr key={i}>
                    <td>{line.date}</td>
                    <td>{line.description}</td>
                    <td>{line.net_amount.formatted}</td>
                    <td>{line.running_balance.formatted}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="list-row">
            <span>{t("Closing")}</span>
            <span className="amount">
              {statement.closing_balance.formatted}
            </span>
          </div>
        </>
      ) : (
        !error && <p className="empty">{t("Loading…")}</p>
      )}
    </section>
  );
}
