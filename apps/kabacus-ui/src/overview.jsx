// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React from "react";
import { t } from "./i18n";
import { OverviewDetails } from "./overview-details";
const nonzero = (a) => a && !/^[-+]?0(?:\.0+)?$/.test(a.exact);
function Amount({ value }) {
  return <span className={`amount ${value?.exact.startsWith("-") ? "negative" : ""}`}>{value?.formatted}</span>;
}
export function Overview({ data, api, onAccount, onReserve, onCredit, onFix }) {
  const cash = data.cash;
  const funds = cash.funds.filter(fund => nonzero(fund.balance));
  const change = cash.change;
  const changeAmount = change?.cash_on_hand;
  const signedChange = changeAmount && `${nonzero(changeAmount) && !changeAmount.exact.startsWith("-") ? "+" : ""}${changeAmount.formatted}`;

  return <div className="overview-layout">
    <section className="panel cash-summary" aria-labelledby="spendable-heading">
      <div className="cash-label">
        <h2 id="spendable-heading">{t("Available to spend")}</h2>
        <details className="info-button">
          <summary aria-label={t("About Cash on Hand")}>ⓘ</summary>
          <p>{t("Free money you can spend right now, after reserves and card reserves.")}</p>
        </details>
      </div>
      <div className="cash-total"><Amount value={cash.cash_on_hand} /></div>
      {changeAmount && change.compare_to && <p className="cash-change">{t("%1 since %2", signedChange, change.compare_to)}</p>}
      <details className="cash-breakdown">
        <summary>{t("How it's worked out")}</summary>
        <dl>{[
          ["Money on Hand", cash.money_on_hand],
          ["Set aside in reserves", cash.future_allocations],
          ["Owes you", cash.receivables],
          ["Available to spend", cash.cash_on_hand],
        ].map(([label, value], i) => <div key={label} className={i === 3 ? "total-line" : ""}>
          <dt>{t(label)}{i === 0 && <details className="info-button">
            <summary aria-label={t("About Money on Hand")}>ⓘ</summary>
            <p>{t("Everything in your bank accounts, reserves and card reserves included, plus what people owe you. Cash on Hand is what's left once reserves, card reserves and what you're owed are set apart.")}</p>
          </details>}</dt>
          <dd>{i === 1 || i === 2 ? "− " : ""}<Amount value={value} /></dd>
        </div>)}</dl>
      </details>
    </section>
    {cash.warnings.length > 0 && <div className="overview-warnings">{cash.warnings.map((warning, i) => <div className="banner warning" key={i}>{warning.message}<button className="quiet" onClick={() => onFix(warning)}>{t("Fix")}</button></div>)}</div>}
    <section className="panel free-accounts">
      <h2>{t("Available by account")}</h2>
      {cash.homes.filter(home => nonzero(home.free)).map(home => <button className="overview-row quiet" key={home.account} onClick={() => onAccount(home.account)}>
        <span>{home.display_name || home.account}</span><Amount value={home.free} />
      </button>)}
      {!cash.homes.some(home => nonzero(home.free)) && <p className="empty">{t("No account has free money right now.")}</p>}
    </section>
    <section className="panel overview-reserves">
      <h2>{t("Reserves")}</h2>
      {funds.map(fund => <button className="overview-row quiet" key={fund.account} onClick={() => onReserve(fund)}>
        <span>{fund.display_name || fund.account}</span><Amount value={fund.balance} />
      </button>)}
      {!funds.length && <><p>{t("Nothing is set aside yet")}</p><p className="muted">{t("Reserves hold money for later, like property tax or car repairs. It stays in your bank account.")}</p><button onClick={() => onReserve(null)}>{t("Set aside")}</button></>}
    </section>
    <section className="panel overview-credit">
      <h2>{t("Credit")}</h2>
      {cash.cards.filter(card => nonzero(card.owed)).map(card => <button className="overview-row quiet" key={card.card} onClick={onCredit}>
        <span>{card.display_name || card.card}<small>{nonzero(card.difference) ? t("Still to reserve") : t("Reserved")}</small></span>
        <span className="card-amounts"><small>{t("Owes")}</small><Amount value={card.owed} /><small>{t("Set Aside for Credit Card Payments")}</small><Amount value={card.reserve_balance} /></span>
      </button>)}
      <p className="muted">{t("Each card's reserve should equal what it owes.")}</p>
    </section>
    <div className="overview-trend"><OverviewDetails api={api} data={data} /></div>
  </div>;
}
