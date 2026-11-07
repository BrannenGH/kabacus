// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React from "react";
import { t } from "./i18n";
import { OverviewDetails } from "./overview-details";
const nonzero = (a) => a && !/^[-+]?0(?:\.0+)?$/.test(a.exact);
function Amount({ value }) { return <span className={`amount ${value?.exact.startsWith("-") ? "negative" : ""}`}>{value?.formatted}</span>; }
export function Overview({ data, api, onAccount, onReserve, onCredit }) {
  const cash = data.cash;
  const amounts=[cash.cash_on_hand,...cash.funds.map(f=>f.balance),...cash.cards.map(c=>c.reserve_balance)].filter(Boolean);
  const places=Math.max(0,...amounts.map(a=>(a.exact.split(".")[1]||"").length));
  const units=a=>{const [whole,fraction=""]=(a?.exact||"0").split(".");const value=BigInt(whole+fraction.padEnd(places,"0"));return value>0n?value:0n;};
  const parts=[units(cash.cash_on_hand),cash.funds.reduce((sum,f)=>sum+units(f.balance),0n),cash.cards.reduce((sum,c)=>sum+units(c.reserve_balance),0n)];
  const total=parts.reduce((sum,n)=>sum+n,0n)||1n;
  const share=n=>Number(n*10000n/total)/100;

  return <div className="overview-layout">
    <section className="panel cash-summary"><div className="cash-label">{t("Cash on Hand")}<details className="info-button"><summary aria-label={t("About Cash on Hand")}>ⓘ</summary><p>{t("Free money you can spend right now, after reserves and card reserves.")}</p></details></div>
      <div className="cash-total"><Amount value={cash.cash_on_hand} /></div>
      <p className="muted">{cash.change && <><Amount value={cash.change.cash_on_hand} /> · </>}{t("Free money you can spend right now, after reserves and card reserves.")}</p>
      <div className="allocation-bar" aria-hidden="true">{parts.map((part,i)=><span key={i} style={{flex:"none",width:`${share(part)}%`}}/>)}</div>
      <p className="money-legend"><span><i className="dot free" />{t("Free money")}</span><span><i className="dot funds" />{t("Reserves")}</span><span><i className="dot reserves" />{t("Card reserves")}</span></p>
      <dl>{[["Money on Hand", cash.money_on_hand], ["Set aside in reserves", cash.future_allocations], ["Owes you", cash.receivables], ["Cash on Hand", cash.cash_on_hand]].map(([label,amount],i)=><div key={label} className={i===3 ? "total-line" : ""}><dt>{t(label)}{i===0&&<details className="info-button"><summary aria-label={t("About Money on Hand")}>ⓘ</summary><p>{t("Everything in your bank accounts, reserves and card reserves included, plus what people owe you. Cash on Hand is what's left once reserves, card reserves and what you're owed are set apart.")}</p></details>}</dt><dd>{i===1||i===2?"− ":""}<Amount value={amount}/></dd></div>)}</dl>
    </section>
    <section className="panel free-accounts"><h2>{t("Free money by account")}</h2>{cash.homes.filter(h=>nonzero(h.free)).map(home=><button className="overview-row quiet" key={home.account} onClick={()=>onAccount(home.account)}><span>{home.display_name||home.account}</span><Amount value={home.free}/></button>)}{!cash.homes.length&&<p className="empty">{t("No accounts")}</p>}</section>
    <section className="panel overview-credit"><h2>{t("Credit")}</h2>{cash.cards.filter(c=>nonzero(c.owed)).map(card=><button className="overview-row quiet" key={card.card} onClick={onCredit}><span>{card.display_name||card.card}<small>{nonzero(card.difference)?t("Still to reserve"):t("Reserved")}</small></span><Amount value={card.owed}/></button>)}</section>
    <section className="panel overview-reserves"><h2>{t("Reserves")}</h2>{cash.funds.filter(f=>nonzero(f.balance)).map(fund=><button className="overview-row quiet" key={fund.account} onClick={()=>onReserve(fund)}><span>{fund.display_name||fund.account}</span><Amount value={fund.balance}/></button>)}{!cash.funds.length&&<><p>{t("Nothing is set aside yet")}</p><p className="muted">{t("Reserves hold money for later, like property tax or car repairs. It stays in your bank account.")}</p><button onClick={()=>onReserve(null)}>{t("Set aside")}</button></>}</section>
    <div className="overview-trend"><OverviewDetails api={api} data={data}/></div>
  </div>;
}
