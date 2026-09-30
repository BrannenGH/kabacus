// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { t, initializeLocale } from "./i18n";
import { NativeLauncher } from "./native-launcher";
import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Api, exactAmount } from "./api";
import { initializeAuth } from "./auth";
import "./style.css";
import { Reports } from "./reports";
import { OperationDialog } from "./operation-dialog";
import { Accounts, People } from "./accounts";
import { Settings } from "./settings";
import { OverviewDetails } from "./overview-details";
import { Activity } from "./activity";
import { CardDetails } from "./card-details";
const pageLabels = {
  Settings: "Settings",
  Accounts: "Money",
  Loans: "Loans",
  People: "People",
  Reports: "Reports",
  Overview: "Overview",
  Register: "Transactions",
  Funds: "Reserves",
  Cards: "Cards",
  Activity: "Activity",
};
const today = () => {
  const date = new Date();
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
};
const name = (account) => account.display_name || account.account;
function Amount({ value }) {
  return (
    <span
      className={`amount ${value?.exact?.startsWith("-") ? "negative" : ""}`}
    >
      {value?.formatted ?? "—"}
    </span>
  );
}
function Empty({ children }) {
  return <p className="empty">{children}</p>;
}
function App({ auth }) {
  const [api] = useState(() => new Api(auth.token));
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState("Overview");
  const [form, setForm] = useState(null);
  const [operationForm, setOperationForm] = useState(null);
  const [openedAccount, setOpenedAccount] = useState("");
  const [notice, setNotice] = useState("");
  const controller = useRef();
  const retry = useRef();
  const delay = useRef(3000);
  const latestRefresh = useRef();
  async function refresh() {
    controller.current?.abort();
    const current = new AbortController();
    controller.current = current;
    setLoading(true);
    setError("");
    try {
      const snapshot = await api.snapshot(current.signal);
      setData(snapshot);
      clearTimeout(retry.current);
      delay.current = 3000;
    } catch (e) {
      if (e.name !== "AbortError") {
        setError(e.message);
        clearTimeout(retry.current);
        retry.current = setTimeout(
          () => latestRefresh.current(),
          delay.current,
        );
        delay.current = Math.min(delay.current * 2, 60000);
      }
    } finally {
      if (!current.signal.aborted) setLoading(false);
    }
  }
  latestRefresh.current = refresh;
  useEffect(() => {
    if (!auth.signedIn) return;
    latestRefresh.current();
    const wake = () => {
      if (document.visibilityState === "visible") latestRefresh.current();
    };
    window.addEventListener("focus", wake);
    document.addEventListener("visibilitychange", wake);
    const interval = setInterval(wake, 60000);
    return () => {
      controller.current?.abort();
      clearTimeout(retry.current);
      clearInterval(interval);
      window.removeEventListener("focus", wake);
      document.removeEventListener("visibilitychange", wake);
    };
  }, []);
  if (!auth.signedIn)
    return (
      <main className="signin">
        <div className="brand-icon">{"k"}</div>
        <h1>{t("Overview")}</h1>
        <p>{t("Not signed in")}</p>
        <button onClick={() => auth.signIn().catch((e) => setError(e.message))}>
          {t("Sign in")}
        </button>
        {error && <p role="alert">{error}</p>}
      </main>
    );
  const titles = {
    Overview: t(
      "Free money you can spend right now, after reserves and card reserves.",
    ),
    Register: t("Transactions"),
    Funds: t("Set money aside"),
    Cards: t("Each card's reserve should equal what it owes."),
    Activity: t("Activity"),
  };
  return (
    <div className="shell">
      <aside>
        <a className="brand" href="/app/">
          <span className="brand-icon">{"k"}</span>
          {"Kabacus"}
        </a>
        <div className="nav-spacer" />
        <nav aria-label={t("Go")}>
          {[
            "Overview",
            "Accounts",
            "Register",
            "Funds",
            "Cards",
            "Loans",
            "People",
            "Activity",
            "Reports",
            "Settings",
          ].map((item, i) => (
            <button
              key={t(pageLabels[item])}
              className={page === item ? "active" : ""}
              aria-current={page === item ? "page" : undefined}
              onClick={() => {
                setPage(item);
                setNotice("");
              }}
            >
              <span aria-hidden="true">{["◫", "≡", "▤", "▱", "↺"][i]}</span>
              {t(pageLabels[item])}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <span className="status-dot" />
          {"Kabacus-server"}
          {auth.signOut && (
            <button className="quiet" onClick={auth.signOut}>
              {t("Sign out")}
            </button>
          )}
        </div>
      </aside>
      <div className="workspace">
        <header>
          <span>
            {t("This app")}
            <span className="slash">/</span>{" "}
            <strong>{t(pageLabels[page])}</strong>
          </span>
          <div className="header-actions">
            <span className="revision">
              {t("Commit:")}
              {data?.version.head ?? "—"}
            </span>
            <button className="secondary" onClick={refresh} disabled={loading}>
              {loading ? t("Loading…") : t("Refresh")}
            </button>
          </div>
        </header>
        <main>
          <div className="page-heading">
            <div>
              <p className="eyebrow">{t("Overview")}</p>
              <h1>{t(pageLabels[page])}</h1>
              <p className="subtitle">{titles[page]}</p>
            </div>
            <select
              aria-label={t("Add")}
              value=""
              disabled={!data || loading || !!error}
              onChange={(e) => {
                const kind = e.target.value;
                if (["transaction", "received", "allocation"].includes(kind))
                  setForm(kind);
                else if (kind)
                  setOperationForm({
                    kind,
                  });
              }}
            >
              <option value="">{t("Add")}</option>
              {[
                ["transaction", "Spent"],
                ["received", "Received"],
                ["purchase", "Card purchase"],
                ["transaction", "Transfer"],
                ["allocation", "Set aside"],
                ["payment", "Pay a card"],
                ["split", "Split with someone"],
                ["postings", "Postings"],
              ].map(([kind, label]) => (
                <option key={label} value={kind}>
                  {t(label)}
                </option>
              ))}
            </select>
            <button
              disabled={!data || loading || !!error}
              onClick={() => setForm("transaction")}
            >
              {t("Add transaction")}
            </button>
          </div>
          {error && (
            <div className="banner error" role="alert">
              {error}{" "}
              <button className="quiet" onClick={refresh}>
                {t("Retry")}
              </button>
              {auth.signIn && (
                <button
                  className="quiet"
                  onClick={() =>
                    auth.signIn().catch((e) => setError(e.message))
                  }
                >
                  {t("Sign in")}
                </button>
              )}
            </div>
          )}
          {notice && (
            <div className="banner" role="status">
              {notice}
            </div>
          )}
          {page === "Settings" && (
            <Settings
              data={data || { version: { auth: {}, server_version: "—" } }}
              api={api}
              auth={auth}
            />
          )}
          {!data ? (
            <Empty>{loading ? t("Loading…") : t("Refresh")}</Empty>
          ) : (
            <>
              {data.cash.warnings.map((warning, i) => (
                <div className="banner warning" key={i}>
                  {warning.message}
                  <button
                    className="quiet"
                    onClick={() =>
                      setPage(
                        warning.code?.includes("reserve")
                          ? "Cards"
                          : warning.code?.includes("fund")
                            ? "Funds"
                            : "Accounts",
                      )
                    }
                  >
                    {t("Fix")}
                  </button>
                </div>
              ))}
              {page === "Overview" && (
                <>
                  <section className="metrics" aria-label={t("Money")}>
                    <article className="metric primary">
                      <p>{t("Cash on Hand")}</p>
                      <Amount value={data.cash.cash_on_hand} />
                      {data.cash.change && (
                        <small>
                          {t("vs %1", data.cash.change.compare_to)}{" "}
                          <Amount value={data.cash.change.cash_on_hand} />
                        </small>
                      )}
                      <small>
                        {t(
                          "Free money you can spend right now, after reserves and card reserves.",
                        )}
                      </small>
                    </article>
                    {[
                      [
                        t("Money on Hand"),
                        data.cash.money_on_hand,
                        t("Money on Hand"),
                      ],
                      [
                        t("Set aside in reserves"),
                        data.cash.future_allocations,
                        t("Set aside in reserves"),
                      ],
                      [t("Owes you"), data.cash.receivables, t("Owes you")],
                    ].map(([label, amount, detail]) => (
                      <article className="metric" key={label}>
                        <p>{label}</p>
                        <Amount value={amount} />
                        <small>{detail}</small>
                      </article>
                    ))}
                  </section>
                  <div className="overview-grid">
                    <section className="panel">
                      <div className="panel-heading">
                        <div>
                          <h2>{t("Money")}</h2>
                          <p>{t("in the bank")}</p>
                        </div>
                        <span className="badge">{data.cash.homes.length}</span>
                      </div>
                      {!data.cash.homes.length && (
                        <Empty>{t("No accounts")}</Empty>
                      )}
                      {data.cash.homes
                        .filter(
                          (home) =>
                            !/^[-+]?0(?:\.0+)?$/.test(home.inclusive.exact),
                        )
                        .map((home) => (
                          <article className="home" key={home.account}>
                            <div className="home-title">
                              <div>
                                <h3>
                                  <button
                                    className="quiet"
                                    onClick={() => {
                                      setOpenedAccount(home.account);
                                      setPage("Register");
                                    }}
                                  >
                                    {home.display_name || home.account}
                                  </button>
                                </h3>
                                <small>{home.account}</small>
                              </div>
                              <Amount value={home.inclusive} />
                            </div>
                            <dl>
                              <div>
                                <dt>
                                  <i className="dot free" />
                                  {t("Free money")}
                                </dt>
                                <dd>
                                  <Amount value={home.free} />
                                </dd>
                              </div>
                              <div>
                                <dt>
                                  <i className="dot funds" />
                                  {t("Reserves")}
                                </dt>
                                <dd>
                                  <Amount value={home.funds} />
                                </dd>
                              </div>
                              <div>
                                <dt>
                                  <i className="dot reserves" />
                                  {t("Card reserves")}
                                </dt>
                                <dd>
                                  <Amount value={home.reserves} />
                                </dd>
                              </div>
                            </dl>
                          </article>
                        ))}
                    </section>
                    <section className="panel">
                      <div className="panel-heading">
                        <div>
                          <h2>{t("Reserves")}</h2>
                          <p>{t("Set money aside")}</p>
                        </div>
                      </div>
                      {data.cash.funds
                        .filter(
                          (f) => !/^[-+]?0(?:\.0+)?$/.test(f.balance.exact),
                        )
                        .slice(0, 5)
                        .map((fund) => (
                          <div className="list-row" key={fund.account}>
                            <div>
                              <h3>
                                <button
                                  className="quiet"
                                  onClick={() =>
                                    setForm({
                                      kind: "allocation",
                                      from: fund.home,
                                      to: fund.account,
                                    })
                                  }
                                >
                                  {fund.display_name || fund.account}
                                </button>
                              </h3>
                              <small>{fund.home}</small>
                            </div>
                            <Amount value={fund.balance} />
                          </div>
                        ))}
                      {!data.cash.funds.length && (
                        <Empty>{t("Nothing is set aside in a reserve.")}</Empty>
                      )}
                      <div className="panel-footer">
                        <button
                          className="secondary"
                          onClick={() => setPage("Funds")}
                        >
                          {t("Reserves")}
                        </button>
                      </div>
                    </section>
                  </div>
                  <section className="panel">
                    <div className="panel-heading">
                      <h2>{t("Cards")}</h2>
                    </div>
                    {data.cash.cards
                      .filter(
                        (card) => !/^[-+]?0(?:\.0+)?$/.test(card.owed.exact),
                      )
                      .map((card) => (
                        <div className="list-row" key={card.card}>
                          <button
                            className="quiet"
                            onClick={() => setPage("Cards")}
                          >
                            {card.display_name || card.card}
                          </button>
                          <span className="amount">{card.owed.formatted}</span>
                        </div>
                      ))}
                  </section>
                  <OverviewDetails api={api} data={data} />
                  <section className="panel">
                    <div className="panel-heading">
                      <div>
                        <h2>{t("Transactions")}</h2>
                        <p>{t("All transactions")}</p>
                      </div>
                      <button
                        className="quiet"
                        onClick={() => setPage("Register")}
                      >
                        {t("Transactions")}
                      </button>
                    </div>
                    <Transactions
                      transactions={[...data.transactions]
                        .reverse()
                        .slice(0, 6)}
                    />
                  </section>
                </>
              )}
              {["Accounts", "Loans"].includes(page) && (
                <Accounts
                  data={data}
                  loans={page === "Loans"}
                  onOpen={(account) => {
                    setOpenedAccount(account);
                    setPage("Register");
                  }}
                />
              )}
              {page === "People" && <People api={api} />}
              {page === "Reports" && <Reports api={api} />}
              {page === "Register" && (
                <Register
                  data={data}
                  api={api}
                  initialAccount={openedAccount}
                  onOperation={
                    !error && !loading ? setOperationForm : undefined
                  }
                />
              )}
              {page === "Funds" && (
                <section className="panel">
                  <div className="panel-heading">
                    <div>
                      <h2>{t("Reserves")}</h2>
                      <p>{t("Into a reserve, out, or between reserves")}</p>
                    </div>
                    <button
                      onClick={() => setForm("allocation")}
                      disabled={loading || !!error}
                    >
                      {t("Set money aside")}
                    </button>
                  </div>
                  {data.cash.funds.map((fund) => (
                    <div className="list-row" key={fund.account}>
                      <div>
                        <h3>
                          <button
                            className="quiet"
                            onClick={() =>
                              setForm({
                                kind: "allocation",
                                from: fund.home,
                                to: fund.account,
                              })
                            }
                          >
                            {fund.display_name || fund.account}
                          </button>
                        </h3>
                        <small>{fund.home}</small>
                      </div>
                      <Amount value={fund.balance} />
                    </div>
                  ))}
                  {!data.cash.funds.length && (
                    <Empty>{t("Nothing is set aside in a reserve.")}</Empty>
                  )}
                </section>
              )}
              {page === "Cards" && (
                <section className="panel">
                  <div className="panel-heading">
                    <button
                      onClick={() =>
                        setOperationForm({
                          kind: "purchase",
                        })
                      }
                      disabled={loading || !!error}
                    >
                      {t("Record purchase")}
                    </button>
                    <button
                      onClick={() =>
                        setOperationForm({
                          kind: "fix",
                        })
                      }
                      disabled={loading || !!error}
                    >
                      {t("Add all reserve lines")}
                    </button>
                    <button
                      onClick={() =>
                        setOperationForm({
                          kind: "payment",
                        })
                      }
                      disabled={loading || !!error}
                    >
                      {t("Pay card")}
                    </button>
                  </div>
                  <div className="panel-heading">
                    <div>
                      <h2>{t("Card reserves")}</h2>
                      <p>
                        {t("Each card's reserve should equal what it owes.")}
                      </p>
                    </div>
                    <span className="badge">
                      {t("%1 unreserved", data.cash.unreserved_count)}
                    </span>
                  </div>
                  {data.cash.cards.map((card) => (
                    <article className="home" key={card.card}>
                      <h3>{card.display_name || card.card}</h3>
                      <dl>
                        {[
                          [t("Owes"), card.owed],
                          [t("Reserve"), card.reserve_balance],
                          [t("Still to reserve"), card.difference],
                        ].map(([label, value]) => (
                          <div key={label}>
                            <dt>{label}</dt>
                            <dd>
                              <Amount value={value} />
                            </dd>
                          </div>
                        ))}
                      </dl>
                      <small>{t("%1 unreserved", card.unreserved_count)}</small>
                      <CardDetails
                        card={card}
                        api={api}
                        onOperation={
                          !error && !loading ? setOperationForm : undefined
                        }
                      />
                    </article>
                  ))}
                  {!data.cash.cards.length && (
                    <Empty>{t("No credit cards")}</Empty>
                  )}
                </section>
              )}
              {page === "Activity" && (
                <Activity api={api} data={data} onOpen={setOperationForm} />
              )}
              <footer>{t("%1 · %2", "kabacus-server", location.host)} </footer>
            </>
          )}
        </main>
      </div>
      {operationForm && (
        <OperationDialog
          key={`${operationForm.kind}:${operationForm.tx?.id || ""}`}
          onInspect={setOperationForm}
          {...operationForm}
          disabled={!!error || loading}
          data={data}
          api={api}
          onClose={() => setOperationForm(null)}
          onSaved={() => {
            setOperationForm(null);
            refresh();
          }}
        />
      )}
      {form && (
        <WriteDialog
          kind={typeof form === "string" ? form : form.kind}
          defaults={typeof form === "object" ? form : {}}
          disabled={!!error || loading}
          data={data}
          api={api}
          onClose={() => setForm(null)}
          onSaved={() => {
            setForm(null);
            setNotice(t("The books still check out with this change"));
            refresh();
          }}
        />
      )}
    </div>
  );
}
function Transactions({ transactions, onOperation }) {
  if (!transactions.length) return <Empty>{t("No transactions")}</Empty>;
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>{t("Date")}</th>
            <th>{t("Payee")}</th>
            <th>{t("Postings")}</th>
            <th>{t("Details")}</th>
          </tr>
        </thead>
        <tbody>
          {transactions.map((tx) => (
            <tr key={tx.id}>
              <td className="date">{tx.date}</td>
              <td>
                {onOperation && (
                  <button
                    className="quiet"
                    onClick={() =>
                      onOperation({
                        kind: "edit",
                        tx,
                      })
                    }
                  >
                    {t("Edit")}
                  </button>
                )}
                {onOperation && (
                  <button
                    className="quiet"
                    onClick={() =>
                      onOperation({
                        kind: "void",
                        tx,
                      })
                    }
                  >
                    {t("Void")}
                  </button>
                )}
                {onOperation && (
                  <button
                    className="quiet"
                    onClick={() => onOperation({ kind: "status", tx })}
                  >
                    {t("Review")}
                  </button>
                )}
                <strong>{tx.payee || "—"}</strong>
                <small className="block">{tx.memo}</small>
              </td>
              <td>
                {tx.postings.map((posting, i) => (
                  <div className="posting" key={i}>
                    <span>{posting.account}</span>
                    <Amount value={posting.amount} />
                  </div>
                ))}
              </td>
              <td>
                <span className={`badge ${tx.unreserved ? "attention" : ""}`}>
                  {tx.unreserved
                    ? t("unreserved")
                    : ["*", "cleared"].includes(tx.status)
                      ? t("Check")
                      : ["!", "pending"].includes(tx.status)
                        ? t("Review")
                        : t("Unknown")}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
function Register({ data, api, onOperation, initialAccount = "" }) {
  const [query, setQuery] = useState("");
  const [unreserved, setUnreserved] = useState(false);
  const [account, setAccount] = useState(initialAccount);
  const [lines, setLines] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => {
    setLines(null);
    setError("");
    if (!account) return;
    const controller = new AbortController();
    api
      .request(`/register/${encodeURIComponent(account)}`, {
        signal: controller.signal,
      })
      .then(setLines)
      .catch((e) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => controller.abort();
  }, [account, data, api]);
  const transactions = data.transactions.filter(
    (tx) =>
      (!unreserved || tx.unreserved) &&
      `${tx.payee} ${tx.memo} ${tx.postings.map((p) => p.account).join(" ")}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  );
  return (
    <section className="panel">
      <div className="panel-heading">
        <div>
          <h2>{t("Transactions")}</h2>
          <p>{t("Search payee, memo, account")}</p>
        </div>
      </div>
      <div className="filters">
        <label className="checkbox">
          <input
            type="checkbox"
            checked={unreserved}
            onChange={(e) => setUnreserved(e.target.checked)}
          />
          {t("Unreserved only")}
        </label>
        {account && onOperation && (
          <>
            <span>
              {t(
                data.accounts.find((a) => a.account === account)?.synthetic
                  ? "Synthetic account"
                  : "Real account",
              )}
            </span>
            <button
              className="secondary"
              onClick={() => onOperation({ kind: "account-type", account })}
            >
              {t("Account type")}
            </button>
            <button
              onClick={() =>
                onOperation({
                  kind: "reconcile",
                  account,
                })
              }
            >
              {t("Reconcile…")}
            </button>
          </>
        )}
        <label>
          {t("Search")}
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("Search payee, memo, account")}
          />
        </label>
        <label>
          {t("Account")}
          <select value={account} onChange={(e) => setAccount(e.target.value)}>
            <option value="">{t("All")}</option>
            {data.accounts.map((a) => (
              <option key={a.account} value={a.account}>
                {name(a)}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error && (
        <p role="alert" className="banner error">
          {error}
        </p>
      )}
      {!account ? (
        <Transactions
          transactions={[...transactions].reverse()}
          onOperation={onOperation}
        />
      ) : !lines ? (
        <Empty>{t("Loading…")}</Empty>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t("Date")}</th>
                <th>{t("Payee")}</th>
                <th>{t("Account")}</th>
                <th>{t("Amount")}</th>
                <th>{t("Money")}</th>
              </tr>
            </thead>
            <tbody>
              {lines
                .filter(
                  (line) =>
                    (!unreserved ||
                      data.transactions.find((tx) => tx.id === line.id)
                        ?.unreserved) &&
                    `${line.payee} ${line.memo} ${line.other_account}`
                      .toLowerCase()
                      .includes(query.toLowerCase()),
                )
                .map((line, i) => (
                  <tr key={`${line.id}-${i}`}>
                    <td>{line.date}</td>
                    <td>
                      {onOperation &&
                      data.transactions.find((tx) => tx.id === line.id) ? (
                        <button
                          className="quiet"
                          onClick={() =>
                            onOperation({
                              kind: "edit",
                              tx: data.transactions.find(
                                (tx) => tx.id === line.id,
                              ),
                            })
                          }
                        >
                          {line.payee}
                        </button>
                      ) : (
                        line.payee
                      )}
                      <small className="block">{line.memo}</small>
                    </td>
                    <td>{line.other_account}</td>
                    <td>
                      <Amount value={line.amount} />
                    </td>
                    <td>
                      <Amount value={line.balance} />
                    </td>
                  </tr>
                ))}
            </tbody>
          </table>
          {!lines.length && <Empty>{t("No transactions")}</Empty>}
        </div>
      )}
    </section>
  );
}
function WriteDialog({
  disabled,
  defaults = {},
  kind,
  data,
  api,
  onClose,
  onSaved,
}) {
  const wide = window.innerWidth >= 1200;
  const generation = useRef(0);
  const timer = useRef();
  const previewRef = useRef();
  const busyRef = useRef();
  const [preview, setPreview] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [blocked, setBlocked] = useState(false);
  const dialog = useRef();
  const formRef = useRef();
  const draft = useRef({});
  const [payees, setPayees] = useState([]);
  const [dirty, setDirty] = useState(false);
  const close = () => {
    if (!busy && (!dirty || window.confirm(t("Discard this change?"))))
      onClose();
  };
  useEffect(() => {
    dialog.current.showModal();
  }, []);
  useEffect(() => {
    if (preview) return;
    for (const [name, value] of Object.entries(draft.current)) {
      const field = formRef.current?.elements.namedItem(name);
      if (field) {
        if (field.type === "checkbox") field.checked = value === "on";
        else field.value = value;
      }
    }
  }, [preview]);
  const allocation = kind === "allocation";
  const received = kind === "received";
  const incomeAccounts = data.accounts.filter((a) => a.kind === "income");
  const moneyAccounts = data.accounts.filter((a) => a.kind === "money");
  function accountPath(value) {
    const typed = value.trim();
    if (data.accounts.some((a) => a.account === typed)) return typed;
    const matches = data.accounts.filter(
      (a) => name(a).toLocaleLowerCase() === typed.toLocaleLowerCase(),
    );
    return matches.length === 1 ? matches[0].account : typed;
  }
  useEffect(() => {
    if (allocation) return;
    const c = new AbortController();
    api
      .request("/payees", {
        signal: c.signal,
      })
      .then(setPayees)
      .catch(() => {});
    return () => c.abort();
  }, [api, allocation]);
  useEffect(() => {
    if (!dirty) return;
    const leave = (event) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", leave);
    return () => window.removeEventListener("beforeunload", leave);
  }, [dirty]);
  function recall(value) {
    const known = payees.find((p) => p.payee === value);
    const tx = known?.last;
    if (!tx || tx.postings.length !== 2) return;
    const from = tx.postings.find((p) => p.amount.exact.startsWith("-"));
    const to = tx.postings.find((p) => !p.amount.exact.startsWith("-"));
    if (!from || !to) return;
    for (const [field, value] of Object.entries({
      from: from.account,
      to: to.account,
      amount: to.amount.exact,
      memo: tx.memo || "",
    }))
      formRef.current.elements.namedItem(field).value = value;
  }
  previewRef.current = preview;
  busyRef.current = busy;
  useEffect(() => () => clearTimeout(timer.current), []);
  function schedule() {
    clearTimeout(timer.current);
    if (!wide || disabled || blocked) return;
    const run = () => {
      if (previewRef.current) return;
      if (busyRef.current) {
        timer.current = setTimeout(run, 150);
        return;
      }
      if (formRef.current?.checkValidity()) formRef.current.requestSubmit();
    };
    timer.current = setTimeout(run, 700);
  }
  function changed(event) {
    setDirty(true);
    draft.current = Object.fromEntries(new FormData(event.currentTarget));
    generation.current++;
    setPreview(null);
    setError("");
    schedule();
  }
  async function submit(event) {
    event.preventDefault();
    clearTimeout(timer.current);
    const currentGeneration = generation.current;
    if (busy || disabled) return;
    setBusy(true);
    setError("");
    try {
      if (preview) {
        await api.write(preview.path, preview.body, preview.head, false);
        onSaved();
      } else {
        const fields = Object.fromEntries(new FormData(event.currentTarget));
        let path, body;
        if (allocation) {
          exactAmount(
            fields.amount,
            data.commodity.symbol,
            data.commodity.places,
          );
          path = "/allocations";
          body = {
            from: fields.from,
            to: fields.new_fund === "on" ? fields.to : accountPath(fields.to),
            amount: fields.amount,
            date: fields.date,
            new_fund: fields.new_fund === "on",
          };
        } else {
          const amount = exactAmount(
            fields.amount,
            data.commodity.symbol,
            data.commodity.places,
          );
          path = "/transactions";
          body = {
            date: fields.date,
            description:
              fields.description +
              (fields.memo?.trim() ? " | " + fields.memo.trim() : ""),
            postings: [
              {
                account: accountPath(fields.to),
                amount,
              },
              {
                account: fields.from,
                amount: {
                  ...amount,
                  mantissa: (-BigInt(amount.mantissa)).toString(),
                },
              },
            ],
            tags: [],
          };
        }
        const result = await api.write(path, body, data.version.head, true);
        if (currentGeneration !== generation.current) return;
        setPreview({
          path,
          body,
          head: data.version.head,
          result,
        });
      }
    } catch (e) {
      if (currentGeneration !== generation.current) return;
      setError(e.message);
      if (e.status === 409) {
        setPreview(null);
        setBlocked(true);
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <dialog
      aria-labelledby="dialog-title"
      ref={dialog}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) close();
      }}
      className="bookkeeping-dialog"
    >
      <form ref={formRef} onSubmit={submit} onChange={changed}>
        <div className="dialog-heading">
          <div>
            <p className="eyebrow">
              {preview ? t("Review before saving") : t("Add")}
            </p>
            <h2 id="dialog-title">
              {allocation
                ? t("Set money aside")
                : received
                  ? t("Received")
                  : t("Add transaction")}
            </h2>
          </div>
          <button
            type="button"
            className="quiet"
            aria-label={t("Cancel")}
            onClick={close}
            disabled={busy}
          >
            {"✕"}
          </button>
        </div>
        {error && (
          <div className="banner error" role="alert">
            {error}
          </div>
        )}
        {blocked ? (
          <Empty>
            {t("The books changed while you were editing. Review again.")}
          </Empty>
        ) : (
          <div className={wide ? "editor-grid" : undefined}>
            <div hidden={!wide && !!preview}>
              <div className="form-fields">
                <label>
                  {t("Date")}
                  <input
                    name="date"
                    type="date"
                    required
                    defaultValue={today()}
                  />
                </label>
                {!allocation && (
                  <label>
                    {t("Payee")}
                    <input
                      name="description"
                      list="known-payees"
                      onChange={(e) => recall(e.target.value)}
                      required
                      autoFocus
                      placeholder={t("Payee")}
                    />
                    <datalist id="known-payees">
                      {payees.map((p) => (
                        <option key={p.payee} value={p.payee} />
                      ))}
                    </datalist>
                  </label>
                )}
                {!allocation && (
                  <label>
                    {t("Memo")}
                    <input name="memo" />
                  </label>
                )}
                <label>
                  {t("From")}
                  <select
                    name="from"
                    required
                    defaultValue={
                      defaults.from ||
                      (received && incomeAccounts.length === 1
                        ? incomeAccounts[0].account
                        : "")
                    }
                  >
                    <option value="">{t("Account")}</option>
                    {(received ? incomeAccounts : data.accounts).map((a) => (
                      <option key={a.account} value={a.account}>
                        {name(a)}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  {t("To")}
                  {received ? (
                    <select
                      name="to"
                      required
                      defaultValue={
                        defaults.to ||
                        (moneyAccounts.length === 1 ? moneyAccounts[0].account : "")
                      }
                    >
                      <option value="">{t("Account")}</option>
                      {moneyAccounts.map((a) => (
                        <option key={a.account} value={a.account}>
                          {name(a)}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      name="to"
                      list="accounts"
                      defaultValue={defaults.to || ""}
                      required
                      placeholder={allocation ? t("Reserve") : t("Account")}
                    />
                  )}
                  <datalist id="accounts">
                    {data.accounts.map((a) => (
                      <option key={a.account} value={name(a)}>
                        {a.account}
                      </option>
                    ))}
                  </datalist>
                </label>
                <label>
                  {t("%1 (%2)", t("Amount"), data.commodity.symbol)}
                  <input
                    name="amount"
                    inputMode="decimal"
                    required
                    pattern="[0-9]+([.][0-9]+)?"
                    placeholder="0.00"
                  />
                </label>
                {allocation && (
                  <label className="checkbox">
                    <input name="new_fund" type="checkbox" />
                    {t("Name the new reserve")}
                  </label>
                )}
              </div>
            </div>
            {preview && (
              <section className="preview">
                <h3>{t("Preview")}</h3>
                <pre>{preview.result.preview || preview.result.after}</pre>
                {preview.result.cash_on_hand && (
                  <div className="list-row">
                    <strong>{t("Cash on Hand")}</strong>
                    <span>
                      {t(
                        "%1 → %2",
                        preview.result.cash_on_hand.before.formatted,
                        preview.result.cash_on_hand.after.formatted,
                      )}
                    </span>
                  </div>
                )}
                {preview.result.balances?.map((balance, i) => (
                  <div className="list-row" key={i}>
                    <span>{balance.account}</span>
                    <span>
                      <Amount value={balance.before} /> →{" "}
                      <Amount value={balance.after} />
                    </span>
                  </div>
                ))}
                {preview.result.warnings?.map((warning, i) => (
                  <p className="banner warning" key={i}>
                    {warning.message || warning.code}
                  </p>
                ))}
                {preview.result.unreserved && (
                  <p className="banner warning">
                    {t(
                      "A card is missing its reserve lines; it will be flagged",
                    )}
                  </p>
                )}
              </section>
            )}
          </div>
        )}
        <div className="dialog-actions">
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => (preview ? setPreview(null) : close())}
          >
            {preview ? t("Back to edit") : t("Cancel")}
          </button>
          <button disabled={busy || blocked || disabled}>
            {busy
              ? t("Checking with the server…")
              : preview
                ? t("Save")
                : t("Preview")}
          </button>
        </div>
      </form>
    </dialog>
  );
}
const nativeLauncher =
  window.__KABACUS_LAUNCHER__ ||
  (window.__TAURI_INTERNALS__ &&
    (location.protocol === "tauri:" ||
      location.hostname === "tauri.localhost"));
initializeLocale()
  .then(() => (nativeLauncher ? null : initializeAuth()))
  .then((auth) =>
    createRoot(document.getElementById("root")).render(
      nativeLauncher ? <NativeLauncher /> : <App auth={auth} />,
    ),
  )
  .catch((error) =>
    createRoot(document.getElementById("root")).render(
      <main className="signin">
        <h1>{t("Can't reach kabacus-server")}</h1>
        <p role="alert">{String(error.message || error)}</p>
        <button onClick={() => location.reload()}>{t("Retry")}</button>
      </main>,
    ),
  );
