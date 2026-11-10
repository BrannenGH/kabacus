// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { t, initializeLocale } from "./i18n";
import { Workspace } from "./workspace";
import { useDialogHistory, keepsUnsavedChange } from "./phone-history";
import { PhonePicker } from "./phone-picker";
import { useMedia, phoneQuery, desktopQuery } from "./form-factor";
import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Api, exactAmount } from "./api";
import { initializeAuth } from "./auth";
import "./style.css";
import { Reports } from "./reports";
import { AmountField } from "./amount-field";
import { accountCurrency, currencies } from "./operations";
import { OperationDialog } from "./operation-dialog";
import { Accounts, People } from "./accounts";
import { Settings } from "./settings";
import { Overview } from "./overview";
import { Activity } from "./activity";
import { CardDetails } from "./card-details";
import icon from "./icon.svg";
import { destinations, pageLabels, Icon, Key, AddChooser, AccountList, BooksSheet } from "./navigation";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
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
export function App({ auth, book, onFileAction }) {
  const phone = useMedia(phoneQuery);
  const [api] = useState(() => book?.api || new Api(auth.token));
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(book?.state?.page || "Overview");
  useEffect(() => { if (!phone && page === "More") setPage("Overview"); }, [phone, page]);
  const [form, setForm] = useState(null);
  const [operationForm, setOperationForm] = useState(null);
  const [openedAccount, setOpenedAccount] = useState(book?.state?.account || "");
  const [selectedRow,setSelectedRow]=useState(book?.state?.selected||"");
  const [chooser, setChooser] = useState(false);
  const [bookMenu, setBookMenu] = useState(false);
  const [notice, setNotice] = useState("");
  const controller = useRef();
  const [phoneSearch, setPhoneSearch] = useState(false);
  const [registerQuery, setRegisterQuery] = useState("");
  const [registerOpen, setRegisterOpen] = useState(!!book?.state?.account);
  useEffect(() => {
    if (!phone) return;
    const state = { kabacus: true, page, account: openedAccount, registerOpen };
    history.replaceState(state, "");
    const back = (event) => {
      if (!event.state?.kabacus) return;
      if (keepsUnsavedChange()) {
        history.pushState(state, ""); return;
      }
      setForm(null); setOperationForm(null); setChooser(false); setBookMenu(false);
      setPage(event.state.page); setOpenedAccount(event.state.account); setRegisterOpen(event.state.registerOpen);
    };
    window.addEventListener("popstate", back);
    return () => window.removeEventListener("popstate", back);
  }, [phone, book?.id]);
  const navigate = (destination, account = "", register = false) => {
    if (phone) history.pushState({ kabacus: true, page: destination, account, registerOpen: register }, "");
    setPage(destination); setOpenedAccount(account); setRegisterOpen(register); setPhoneSearch(false); setRegisterQuery("");
  };
  const openRegister = (account) => navigate("Register", account, true);
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
  useEffect(()=>{if(book?.activity?.expires_at)latestRefresh.current();},[book?.activity?.expires_at]);
  const go = (destination) => {
    if (keepsUnsavedChange()) return;
    setForm(null); setOperationForm(null); navigate(destination, phone ? "" : openedAccount);
  };
  const beginAdd=()=>{
    if(keepsUnsavedChange())return;
    setForm(null);setOperationForm(null);setChooser(true);
  };
  const choose = (command) => {
    setChooser(false);
    const kind = { "transaction.spent": "transaction", "transaction.received": "received", "reserve.setAside": "allocation" }[command.id];
    if (kind) setForm(kind === "received" ? {kind, to: openedAccount} : { kind, from: openedAccount });
    else setOperationForm({ kind: { "card.purchase": "purchase", "card.pay": "payment", "split.record": "split", "transaction.split": "split", "transaction.transfer": "transfer", "transaction.postings": "postings" }[command.id], account: openedAccount });
  };
  useEffect(() => { book?.remember?.({ page, account: openedAccount, selected:selectedRow }); }, [page, openedAccount, selectedRow, book?.id]);
  useEffect(() => {
    let goUntil = 0;
    const keys = (event) => {
      if (event.key.length === 1 || event.key === "Tab") document.documentElement.classList.add("hardware-keyboard");
      const editing = event.target.closest("input, textarea, select, [contenteditable=true]");
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        const form = document.querySelector(".bookkeeping-dialog form");
        if (form?.checkValidity()) { event.preventDefault(); form.requestSubmit(); }
        return;
      }
      const native=!!window.__TAURI_INTERNALS__;
      if(native&&(event.ctrlKey||event.metaKey)&&/^[1-8]$/.test(event.key)){event.preventDefault();go(destinations[Number(event.key)-1]);return;}
      if((event.ctrlKey||event.metaKey)&&event.key===","){event.preventDefault();go("Settings");return;}
      if(native&&((event.ctrlKey&&event.key.toLowerCase()==="n")||event.key==="F5")){event.preventDefault();if(event.key==="F5")latestRefresh.current();else if(data&&!error)beginAdd();return;}
      if (editing || event.altKey || event.ctrlKey || event.metaKey || document.querySelector("dialog[open]")) return;
      if (event.key.toLowerCase() === "g") { goUntil = Date.now() + 1500; return; }
      if (/^[1-8]$/.test(event.key) && Date.now() < goUntil) { event.preventDefault(); go(destinations[Number(event.key)-1]); goUntil = 0; }
      else if (event.key.toLowerCase() === "n" && data && !error) { event.preventDefault(); beginAdd(); }
      else if (event.key.toLowerCase() === "r") { event.preventDefault(); latestRefresh.current(); }
      else if (event.key === "/") { event.preventDefault(); go("Register"); setTimeout(() => document.querySelector('input[type="search"], input[placeholder="' + t("Search payee, memo, account") + '"]')?.focus(), 0); }
    };
    window.addEventListener("keydown", keys); return () => window.removeEventListener("keydown", keys);
  }, [data, error]);
  const saved = (result) => {
    const revision = result?.head || result?.written?.head;
    setNotice(revision ? t("Saved as revision %1", revision) : "");
    refresh();
  };
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
  }, [auth.signedIn]);
  if (!auth.signedIn)
    return (
      <main className="signin">
        <img className="brand-icon" src={icon} alt="" />
        <h1>{t("Sign in to %1", book?.filename || location.host)}</h1>
        <p>{t("Not signed in")}</p>
        <button className="quiet" onClick={()=>onFileAction?.("welcome")}>{t("Back to my books")}</button>
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
      {phone && <header className="phone-appbar">
        {registerOpen || ["Loans", "People", "Reports", "Activity", "Settings"].includes(page)
          ? <><button className="quiet" aria-label={t("Back")} onClick={() => history.back()}>‹</button><strong>{registerOpen ? name(data?.accounts.find(a => a.account === openedAccount) || { account: openedAccount }) : t(pageLabels[page])}</strong></>
          : <button className="quiet phone-books-name" onClick={() => setBookMenu(true)}>{book?.name || "Kabacus"} ⌄</button>}
        <button className="quiet" aria-label={t("Search")} onClick={() => { if (page !== "Register") navigate("Register"); setPhoneSearch(!phoneSearch); }}>⌕</button>
        <details className="phone-overflow"><summary aria-label={t("More")}>⋮</summary><div>
          <button className="quiet" data-action="refresh" onClick={refresh}>{t("Refresh")}</button>
          {registerOpen && <button className="quiet" onClick={() => setOperationForm({kind: "reconcile", account: openedAccount})}>{t("Reconcile…")}</button>}
          <button className="quiet" onClick={() => setBookMenu(true)}>{t("Books")}</button>
        </div></details>
      </header>}
      {phone && phoneSearch && <div className="phone-search"><input type="search" aria-label={t("Search")} placeholder={t("Search payee, memo, account")} value={registerQuery} onChange={e => setRegisterQuery(e.target.value)} autoFocus /></div>}
      {phone && bookMenu && <BooksSheet book={book} onClose={() => setBookMenu(false)} onAction={id => {setBookMenu(false); onFileAction?.(id);}} />}
      {!phone && <>
      <aside>
        <button className="brand quiet" aria-haspopup="menu" aria-expanded={bookMenu} onClick={() => setBookMenu(!bookMenu)}>
          <img className="brand-icon" src={icon} alt="" /><span>{book?.name || "Kabacus"}<small>{book?.filename || location.host}</small></span><span aria-hidden="true">⌄</span>
        </button>
        {bookMenu && <div className="book-menu" role="menu">{[
          ["new-window", "New Window", "Ctrl+Shift+N"], ["new", "New Books…", ""], ["open", "Open Books…", "Ctrl+O"], ["recent", "Open Recent", ""], ["save-copy", "Save a Copy As…", "Ctrl+Shift+S"], ["close", "Close Window", "Ctrl+Shift+W"], ["connect", "Connect to a Server…", ""], ["quit", "Quit", "Ctrl+Q"]
        ].map(([id, label, key]) => <button role="menuitem" key={id} onClick={() => { setBookMenu(false); onFileAction?.(id); }}>{t(label)}{key && <Key>{key}</Key>}</button>)}</div>}
        <div className="nav-spacer" />
        <nav aria-label={t("Go")}>
          {destinations.map((item, i) => (
            <button
              key={item}
              data-destination={item}
              aria-label={t(pageLabels[item])}
              className={page === item ? "active" : ""}
              aria-current={page === item ? "page" : undefined}
              onClick={() => {
                go(item);
                setNotice("");
              }}
            >
              <Icon name={item} />
              {t(pageLabels[item])}<Key>{window.__TAURI_INTERNALS__?`Ctrl+${i+1}`:`G ${i+1}`}</Key>
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <button className="quiet settings-action" onClick={() => go("Settings")}><Icon name="Settings" />{t("Settings")}<Key>Ctrl+,</Key></button>
          <div className="location"><span className="status-dot" /> <strong>{book?.local ? t("On this computer") : location.host}</strong><small>{book?.location || auth.username || ""} · {t("All changes saved")}</small></div>
          {book?.activity?.agent && <p role="status">{t("%1 is working in these books", book.activity.agent)}</p>}
          {auth.signOut && <button className="quiet" onClick={auth.signOut}>{t("Sign out")}</button>}
        </div>
      </aside></>}
      <div className="workspace">
        <main>
          <div className="page-heading">
            <div>

              <h1>{page === "More" ? t("More") : t(pageLabels[page])}</h1>

            </div>
            <div className="header-actions">
              <button disabled={!data || loading || !!error} data-action="add" aria-label={t("Add")} onClick={beginAdd}><Icon name="Add" />{t("Add")} <Key>{window.__TAURI_INTERNALS__?"Ctrl+N":"N"}</Key></button>
              <button className="secondary" data-action="refresh" aria-label={t("Refresh")} onClick={refresh} disabled={loading}><Icon name="Refresh" />{loading ? t("Loading…") : t("Refresh")} <Key>{window.__TAURI_INTERNALS__?"F5":"R"}</Key></button>
            </div>
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
            <div className="save-toast" role="status">{notice}<button className="quiet" onClick={() => go("Activity")}>{t("View")}</button></div>
          )}
          {phone && page === "More" && <section className="phone-more">
            {["Loans", "People", "Reports", "Activity", "Settings"].map(item => <button key={item} data-destination={item} className={item === "Settings" ? "quiet settings-action" : "quiet"} onClick={() => go(item)}><Icon name={item} />{t(pageLabels[item])}<span aria-hidden="true">›</span></button>)}
            <h2>{t("Books")}</h2><div className="location"><strong>{book?.name || "Kabacus"}</strong><small>{book?.local ? t("On this computer") : location.host} · {book?.location || auth.username || ""}</small></div>
            <button className="quiet" onClick={() => setBookMenu(true)}>{t("Switch Books…")}</button>
            <button className="quiet" onClick={() => onFileAction?.("connect")}>{t("Connect to a Server…")}</button>
          </section>}
          {page === "Settings" && (
            <Settings
              data={data || { version: { auth: {}, server_version: "—" } }}
              api={api}
              auth={auth} book={book}
            />
          )}
          {!data ? (
            <Empty>{loading ? t("Loading…") : t("Refresh")}</Empty>
          ) : (
            <>
              {(!phone || page !== "Overview") && data.cash.warnings.map((warning, i) => (
                <div className="banner warning" key={i}>
                  {warning.message}
                  <button
                    className="quiet"
                    onClick={() =>
                      go(
                        warning.code?.includes("reserve")
                          ? "Cards"
                          : warning.code?.includes("fund")
                            ? "Funds"
                            : "Register",
                      )
                    }
                  >
                    {t("Fix")}
                  </button>
                </div>
              ))}
              {page === "Overview" && <Overview data={data} api={api} onFix={warning => go(warning.code?.includes("reserve") ? "Cards" : warning.code?.includes("fund") ? "Funds" : "Register")} onAccount={openRegister} onReserve={(fund)=>setForm({kind:"allocation",from:fund?.home,to:fund?.account})} onCredit={()=>go("Cards")} />}
              {page === "Loans" && (
                <Accounts
                  data={data}
                  loans={page === "Loans"}
                  onOpen={(account) => {
                    openRegister(account);
                  }}
                />
              )}
              {page === "People" && <People api={api} />}
              {page === "Reports" && <Reports api={api} data={data} />}
              {page === "Register" && (
                <div className="transactions-workspace">{(!phone || !registerOpen) && <AccountList data={registerQuery ? {...data, accounts: data.accounts.filter(a => `${a.display_name} ${a.account}`.toLowerCase().includes(registerQuery.toLowerCase()))} : data} selected={openedAccount} onSelect={phone ? openRegister : setOpenedAccount} />}{(!phone || registerOpen) && <Register
                  phone={phone} query={registerQuery}
                  data={data}
                  api={api}
                  initialAccount={openedAccount}
                  onAccountChange={setOpenedAccount}
                  selected={selectedRow} onSelect={setSelectedRow} activity={book?.activity}
                  onOperation={
                    !error && !loading ? setOperationForm : undefined
                  }
                />}</div>
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
                      <h2>{t("Set Aside for Credit Card Payments")}</h2>
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

            </>
          )}
        </main>
      </div>
      {phone && <>
        <nav className="phone-tabs" aria-label={t("Go")}>{["Overview", "Register", "Funds", "Cards", "More"].map(item => <button key={item} data-destination={item} aria-label={t(item === "More" ? "More" : pageLabels[item])} aria-current={(item === page || (item === "More" && ["Loans", "People", "Reports", "Activity", "Settings"].includes(page))) ? "page" : undefined} onClick={() => go(item)}><Icon name={item} /><span>{t(item === "More" ? "More" : pageLabels[item])}</span></button>)}</nav>
        {!["More", "Reports", "Activity", "Settings"].includes(page) && <button className="phone-fab" aria-label={t("Add")} disabled={!data || loading || !!error} onClick={beginAdd}><Icon name="Add" /></button>}
      </>}
      {chooser && data?.commands?.add && <AddChooser commands={data.commands.add} onChoose={choose} onClose={() => setChooser(false)} />}
      {operationForm && (
        <OperationDialog
          key={`${operationForm.kind}:${operationForm.tx?.id || ""}`}
          onInspect={setOperationForm}
          {...operationForm}
          disabled={!!error || loading}
          data={data}
          api={api}
          onClose={() => setOperationForm(null)}
          onSaved={(result) => { setOperationForm(null); saved(result); }}
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
          onSaved={(result) => { setForm(null); saved(result); }}
        />
      )}
    </div>
  );
}
function Transactions({ transactions, onOperation, selected, onSelect, activity }) {
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
            <tr key={tx.id} aria-selected={selected===String(tx.id)} onClick={()=>onSelect?.(String(tx.id))} className={activity?.changed?.includes(tx.id)?"agent-changed":""} title={activity?.changed?.includes(tx.id)?activity.changed_by:undefined}>
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
function Register({ phone, query: phoneQuery = "", data, api, onOperation, initialAccount = "", onAccountChange, selected, onSelect, activity }) {
  const [query, setQuery] = useState("");
  const [unreserved, setUnreserved] = useState(false);
  const [account, setAccount] = useState(initialAccount);
  useEffect(()=>setAccount(initialAccount),[initialAccount]);
  useEffect(()=>onAccountChange?.(account),[account]);
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
  const searchQuery = phone ? phoneQuery : query;
  const transactions = data.transactions.filter(
    (tx) =>
      (!unreserved || tx.unreserved) &&
      `${tx.payee} ${tx.memo} ${tx.postings.map((p) => p.account).join(" ")}`
        .toLowerCase()
        .includes(searchQuery.toLowerCase()),
  );
  return (
    <section className="panel">
      <div className="panel-heading">
        <div>
          <h2>{t("Transactions")}</h2>
          <p>{t("Search payee, memo, account")}</p>
        </div>
      </div>
      <div className="filters" hidden={phone}>
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
      {phone && !account ? <div className="phone-register">{[...new Set(transactions.map(tx => tx.date))].sort().reverse().map(day => <section key={day}><h3>{day}</h3>{transactions.filter(tx => tx.date === day).reverse().map(tx => <button key={tx.id} className="register-row quiet" onClick={() => onOperation?.({kind:"edit",tx})}><span>{["*", "cleared"].includes(tx.status) ? "●" : "○"}</span><span className="grow"><strong>{tx.payee}</strong><small>{tx.memo}</small></span><Amount value={tx.postings[0]?.amount} /></button>)}</section>)}</div> : !account ? (
        <Transactions
          transactions={[...transactions].reverse()}
          onOperation={onOperation}
          selected={selected} onSelect={onSelect} activity={activity}
        />
      ) : !lines ? (
        <Empty>{t("Loading…")}</Empty>
      ) : (
        phone ? <div className="phone-register">{[...new Set(lines.map(line => line.date))].sort().reverse().map(day => <section key={day}>
          <h3>{day}</h3>{lines.filter(line => line.date === day && `${line.payee} ${line.memo} ${line.other_account}`.toLowerCase().includes(searchQuery.toLowerCase())).reverse().map((line, i) => <button className="register-row quiet" key={`${line.id}-${i}`} onClick={() => {onSelect?.(String(line.id)); const tx = data.transactions.find(tx => tx.id === line.id); if (tx && onOperation) onOperation({kind: "edit", tx});}}>
            <span className="cleared-mark">{["*", "cleared"].includes(data.transactions.find(tx => tx.id === line.id)?.status) ? "●" : "○"}</span>
            <span className="grow"><strong>{line.payee}</strong><small>{line.memo || line.other_account}</small></span><span><Amount value={line.amount} /><small><Amount value={line.balance} /></small></span>
          </button>)}</section>)}</div> :
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
                      .includes(searchQuery.toLowerCase()),
                )
                .map((line, i) => (
                  <tr key={`${line.id}-${i}`} aria-selected={selected===String(line.id)} onClick={()=>onSelect?.(String(line.id))} className={activity?.changed?.includes(line.id)?"agent-changed":""} title={activity?.changed?.includes(line.id)?activity.changed_by:undefined}>
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
  const wide = useMedia(desktopQuery);
  const phone = useMedia(phoneQuery);
  const generation = useRef(0);
  const timer = useRef();
  const previewRef = useRef();
  const busyRef = useRef();
  const [preview, setPreview] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [blocked, setBlocked] = useState(false);
  const [currency, setCurrency] = useState(accountCurrency(data, defaults.from || defaults.to));
  const headRef = useRef(data.version.head);
  const [conflictDiff, setConflictDiff] = useState("");
  async function previewAgain() {
    const version=await api.request("/version");headRef.current=version.head;setPreview(null);setBlocked(false);setError("");
  }
  async function theirChange() {const history=await api.request("/history");if(history[0])setConflictDiff(await api.text(`/history/${history[0].hash}`));}
  const dialog = useRef();
  const formRef = useRef();
  const draft = useRef({});
  const [payees, setPayees] = useState([]);
  const [dirty, setDirty] = useState(false);
  useDialogHistory({ onClose, preview, setPreview, dirty, busy });
  const close = () => {
    if (!busy && (!dirty || window.confirm(t("Discard this change?"))))
      onClose();
  };
  useEffect(() => {
    dialog.current.show();
  }, []);
  useEffect(() => {
    if (preview) return;
    for (const [name, value] of Object.entries(draft.current)) {
      if (name === "currency") continue;
      const field = formRef.current?.elements.namedItem(name);
      if (field) {
        if (field.type === "checkbox") field.checked = value === "on";
        else field.value = value;
      }
    }
  }, [preview, phone]);
  const allocation = kind === "allocation";
  const received = kind === "received";
  const transfer = kind === "transfer";
  const incomeAccounts = data.accounts.filter((a) => a.kind === "income");
  const moneyAccounts = data.accounts.filter((a) => a.kind === "money" || a.kind === "non-liquid" && a.account.startsWith("assets:"));
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
  function recall(value) {
    const known = payees.find((p) => p.payee === value);
    const tx = known?.last;
    if (!tx || tx.postings.length !== 2 || tx.postings.some(p => p.cost) || tx.postings[0].amount.commodity !== tx.postings[1].amount.commodity) return;
    const from = tx.postings.find((p) => p.amount.exact.startsWith("-"));
    const to = tx.postings.find((p) => !p.amount.exact.startsWith("-"));
    if (!from || !to) return;
    setCurrency(to.amount.commodity);
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
    event.currentTarget.dataset.dirty = "true";
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
        const result = await api.write(preview.path, preview.body, preview.head, false);
        onSaved(result);
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
            currency,
            currencies(data).find(c => c.symbol === currency)?.places,
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
        const result = await api.write(path, body, headRef.current, true);
        if (currentGeneration !== generation.current) return;
        setPreview({
          path,
          body,
          head: headRef.current,
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
  const amountField = <AmountField data={data}
    input={{name:"amount", autoFocus:phone, pattern:"[0-9]+([.][0-9]+)?", placeholder:"0.00",
      "aria-label":t("%1 (%2)", t("Amount"), allocation ? data.commodity.symbol : currency)}}
    currency={allocation ? null : {name:"currency",value:currency,onChange:e=>setCurrency(e.target.value)}} />;
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
              {phone && preview ? t("Review") : allocation
                ? t("Set money aside")
                : received
                  ? t("Received")
                  : transfer ? t("Transfer") : t("Spent")}
            </h2>
          </div>
          <button
            type="button"
            className="quiet"
            aria-label={t(phone && preview ? "Back" : "Cancel")}
            onClick={() => phone && preview ? setPreview(null) : close()}
            disabled={busy}
          >
            {phone ? t(preview ? "Edit" : "Cancel") : "✕"}
          </button>
        </div>
        {error && !blocked && (
          <div className="banner error" role="alert">
            {error}
          </div>
        )}
        {blocked && (
          <section className="conflict-panel" role="alert"><h3>{t("Someone saved first")}</h3><p>{error}</p><button type="button" onClick={()=>theirChange().catch(e=>setError(e.message))}>{t("See their change")}</button><button type="button" onClick={()=>setBlocked(false)}>{t("Keep editing")}</button><button type="button" onClick={()=>previewAgain().catch(e=>setError(e.message))}>{t("Preview again")}</button>{conflictDiff&&<pre>{conflictDiff}</pre>}</section>
        )}
          <div hidden={blocked} className={wide ? "editor-grid" : undefined}>
            <div hidden={!wide && !!preview}>
              <div className="form-fields">
                {phone && amountField}
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
                      autoFocus={!phone}
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
                    onChange={e => { if (!received && !allocation) setCurrency(accountCurrency(data, e.target.value)); }}
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
                      onChange={e => setCurrency(accountCurrency(data, e.target.value))}
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
                {!phone && amountField}
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
                <h3>{t("What changes")}</h3><p className="balanced-badge">{t("Balanced · checked against revision %1", preview.head)}</p>
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
                : t(phone ? "Review" : "Preview")} {preview && <Key>Ctrl+Enter</Key>}
          </button>
        </div>
      </form>
      <PhonePicker form={formRef} />
    </dialog>
  );
}
const nativeLauncher =
  window.__KABACUS_LAUNCHER__ ||
  (window.__TAURI_INTERNALS__ &&
    (location.protocol === "tauri:" ||
      location.hostname === "tauri.localhost"));
initializeLocale().then(async () => {
  const query=new URLSearchParams(location.search);
  if(query.has("connect"))sessionStorage.setItem("kabacus.connect","1");
  if(query.has("returnTo")){try{const back=new URL(query.get("returnTo"));if(back.protocol==="https:"||back.origin===location.origin)sessionStorage.setItem("kabacus.return",back.href);}catch{}}
  let auth=null;
  if(!nativeLauncher && location.pathname.startsWith("/app/")) {
    try { auth=await initializeAuth(); } catch { /* Local books remain available offline. */ }
  }
  createRoot(document.getElementById("root")).render(<Workspace Application={App} hosted={auth}/>);
  if("serviceWorker" in navigator && !nativeLauncher)navigator.serviceWorker.register(new URL("./sw.js",document.baseURI)).catch(()=>{});
});
