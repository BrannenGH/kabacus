// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useRef } from "react";
import { t } from "./i18n";
import { useDialogHistory } from "./phone-history";

export const destinations = ["Overview", "Register", "Funds", "Cards", "Loans", "People", "Reports", "Activity"];
export const pageLabels = { Overview: "Overview", Register: "Transactions", Funds: "Reserves", Cards: "Credit", Loans: "Loans", People: "People", Reports: "Reports", Activity: "Activity", Settings: "Settings" };
const paths = {
  Overview: "M3 10l9-7 9 7M5 9v12h14V9M9 21v-8h6v8",
  Register: "M4 4h16v16H4zM8 8h8M8 12h8M8 16h5",
  Funds: "M3 7h7l2 2h9v11H3zM3 7V4h7l2 3",
  Cards: "M3 5h18v14H3zM3 10h18M7 15h3",
  Loans: "M3 9l9-6 9 6M4 10h16M6 11v7M12 11v7M18 11v7M3 21h18",
  People: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M15 4a4 4 0 0 1 0 8M22 21v-2a4 4 0 0 0-3-4M13 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
  Reports: "M4 3v18h17M8 16v-5M13 16V7M18 16V4",
  Activity: "M3 12a9 9 0 1 0 3-7M3 3v5h5M12 7v5l3 2",
  Settings: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M8 3l1 2h6l1-2 4 4-2 1v8l2 1-4 4-1-2H9l-1 2-4-4 2-1V8L4 7z",
  More: "M4 12h1M11 12h1M18 12h1",
  Search: "M10.5 18a7.5 7.5 0 1 1 0-15 7.5 7.5 0 0 1 0 15M16 16l5 5",
  Chevron: "M6 9l6 6 6-6",
  Back: "M15 5l-7 7 7 7",
  Forward: "M9 5l7 7-7 7",
  Close: "M6 6l12 12M18 6L6 18",
  Info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18M12 11v5M12 7.5v.01",
  Overflow: "M12 5.5v.01M12 12v.01M12 18.5v.01",
  Add: "M12 4v16M4 12h16", Refresh: "M20 7v5h-5M4 17v-5h5M5 8a8 8 0 0 1 13-3l2 2M4 17l2 2a8 8 0 0 0 13-3",
};
// Back and Forward point the other way in a right-to-left language.
const directional = new Set(["Back", "Forward"]);
export function Icon({ name }) { return <svg className={directional.has(name) ? "line-icon directional" : "line-icon"} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name] || paths.Register} /></svg>; }
export function Key({ children }) { return <kbd>{children}</kbd>; }
// A modal sheet: its title, a close button, and Back or Esc to close it.
function Sheet({ id, className, title, close, onClose, onKeyDown, children }) {
  useDialogHistory({ onClose });
  const ref = useRef();
  useEffect(() => { ref.current.showModal(); ref.current.querySelector("[data-autofocus]")?.focus(); }, []);
  return <dialog ref={ref} className={className} aria-labelledby={id} onCancel={(e) => { e.preventDefault(); onClose(); }} onKeyDown={onKeyDown}>
    <div className="dialog-heading"><h2 id={id}>{title}</h2><button className="quiet" onClick={onClose}>{close}</button></div>
    {children}
  </dialog>;
}
export function AddChooser({ commands, onChoose, onClose }) {
  return <Sheet id="add-title" className="add-chooser" title={t("Add — what happened?")} close={<>{t("Cancel")} <Key>Esc</Key></>} onClose={onClose} onKeyDown={(e) => {
    if (/^[1-8]$/.test(e.key)) { e.preventDefault(); onChoose(commands[Number(e.key) - 1]); }
  }}>
    <p>{t("Choose what happened. The form fills in the lines; you see what changes before you save.")}</p>
    <div className="choice-grid">{commands.map((command, i) => <button data-choice data-autofocus={i === 0 || undefined} key={command.id} onClick={() => onChoose(command)}>
      <Icon name={i === 2 || i === 5 ? "Cards" : i === 4 ? "Funds" : "Register"} /><span><strong>{t(command.label)}</strong><small>{t(["Money left an account", "Income or a refund", "Adds the reserve lines for you", "Between two bank accounts", "Into a reserve, out, or between reserves", "From its reserve", "They owe you, or you owe them", "Any two accounts"][i])}</small></span><Key>{i + 1}</Key>
    </button>)}</div>
  </Sheet>;
}

export function AccountList({ data, selected, onSelect }) {
  const groups = [["money", "Bank & cash"], ["revolving-debt", "Credit"], ["term-liability", "Loans"], ["receivable", "People"], ["payable", "People"], ["expense", "Spending"], ["income", "Income"], ["non-liquid", "Investments"], ["unclassified", "Other"]];
  const neutral = account => account.kind === "income" || account.account.startsWith("equity:");
  const children = new Map();
  for (const c of data.accounts) if (c.home && c.home !== c.account) children.set(c.home, [...(children.get(c.home) || []), c]);
  const amount = (account) => (account.balance_inclusive || account.balances || []).map((b) => neutral(account) && b.exact.startsWith("-") ? b.formatted.replace(/^-/, "") : b.formatted).join(" / ");
  return <div className="account-list panel" aria-label={t("Accounts")}>
    <button className={!selected ? "selected quiet" : "quiet"} onClick={() => onSelect("")}>{t("All transactions")}</button>
    {groups.map(([kind, label]) => {
      const accounts = data.accounts.filter((a) => (a.account.startsWith("equity:") ? "unclassified" : a.kind) === kind);
      if (!accounts.length) return null;
      return <section key={kind}><h2>{t(label)}</h2>{accounts.map((a) => <React.Fragment key={a.account}>
        <button className={selected === a.account ? "selected" : ""} onClick={() => onSelect(a.account)}><span>{a.display_name || a.account}</span><span className={!neutral(a) && (a.balance_inclusive || []).some((b) => b.exact.startsWith("-")) ? "amount negative" : "amount"}>{amount(a)}</span></button>
        {(children.get(a.account) || []).map((c) => <button key={c.account} className={`account-child ${selected === c.account ? "selected" : ""}`} onClick={() => onSelect(c.account)}><span>{c.display_name || c.account}</span><span className="amount">{amount(c)}</span></button>)}
      </React.Fragment>)}</section>;
    })}
  </div>;
}


export function BooksSheet({ book, onClose, onAction }) {
  return <Sheet id="books-title" className="books-sheet" title={t("Books")} close={t("Close")} onClose={onClose}>
    <div className="location"><strong>{book?.name || "Kabacus"}</strong><small>{book?.local ? t("On this computer") : location.host} · {book?.location || ""}</small></div>
    <h3>{t("Open Recent")}</h3>
    {(book?.recent || []).map((source, i) => <button key={i} className="quiet" onClick={() => {onClose(); book.openRecent(source);}}>{source.name || source.filename || source.url || source.path}</button>)}
    {[["new", "New Books…"], ["open", "Open Books…"], ["save-copy", "Save a Copy As…"], ["connect", "Connect to a Server…"]].map(([id, label]) => <button key={id} className="quiet" onClick={() => onAction(id)}>{t(label)}</button>)}
  </Sheet>;

}
