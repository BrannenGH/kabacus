// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useRef, useState } from "react";
import { t } from "./i18n";
import { ImportReview } from "./statement-import";
import { useDialogHistory } from "./phone-history";

export function BankConnections({ data, api, status, onChanged, onSync, local }) {
  const [token, setToken] = useState("");
  const [confirmed, setConfirmed] = useState(null);
  const [remote, setRemote] = useState(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [problem, setProblem] = useState("");
  const [editing, setEditing] = useState(false);
  const targets = data.accounts.filter(account => !account.synthetic && ["money", "revolving-debt"].includes(account.kind));
  async function perform(run) {
    setNotice(""); setProblem(""); setBusy(true);
    try { await run(); }
    catch (error) { setProblem(error.message); onChanged(); }
    finally { setBusy(false); }
  }
  async function connect(dry_run) {
    await perform(async () => {
      const reply = await api.request("/simplefin/connection", { method: "PUT", body: { token, dry_run, replace: !dry_run && !!confirmed?.connected } });
      if (dry_run) setConfirmed(reply);
      else {
        setRemote(reply); setToken(""); setConfirmed(null); setEditing(false);
        onChanged();
      }
    });
  }
  async function link(account, target) {
    await perform(async () => {
      const current = await api.request("/version");
      await api.request(`/accounts/${encodeURIComponent(target || account.linked)}/simplefin-link`, { method: "PUT", head: current.head, body: { remote: target ? { conn_id: account.conn_id, id: account.id } : null } });
      setRemote(previous => ({ ...previous, accounts: previous.accounts.map(candidate => candidate.id === account.id && candidate.conn_id === account.conn_id ? { ...candidate, linked: target || null } : candidate) }));
      onChanged();
    });
  }
  const groups = remote?.connections || [];
  function accountName(target) { return targets.find(item => item.account === target)?.display_name || target; }
  return <section className="bank-connections" aria-labelledby="bank-connections-title">
    <h2 id="bank-connections-title">{t("Bank connections")}</h2>
    {!local && <h3>{t("Your bank connection")}</h3>}
    {problem && <p role="alert" className="banner error">{problem}</p>}
    {notice && <p role="status">{notice}</p>}
    {busy && <p role="status">{t("Checking with the server…")}</p>}
    {status?.connected && !editing ? <>
      <p>{status.host}</p>
      <p className="muted">{t("Requests remaining today: %1", status.remaining)}</p>
      <div className="dialog-actions bank-actions">
        <button disabled={busy} onClick={onSync}>{t("Sync")}</button>
        <button className="secondary" disabled={busy} onClick={() => perform(async () => { setRemote(await api.request("/simplefin/accounts")); onChanged(); })}>{t("Load bank accounts")}</button>
        <button className="quiet" disabled={busy} onClick={() => setEditing(true)}>{t("Replace connection")}</button>
        <button className="quiet" disabled={busy} onClick={() => perform(async () => {
          const reply = await api.request("/simplefin/connection", { method: "DELETE" });
          setRemote(null); setNotice(reply.message); onChanged();
        })}>{t("Disconnect")}</button>
      </div>
    </> : <form className="form-fields" onSubmit={event => { event.preventDefault(); connect(!confirmed); }}>
      <a href="https://bridge.simplefin.org/simplefin/create" target="_blank" rel="noreferrer">{t("Get a SimpleFIN setup token")}</a>
      <label>{t("Setup token")}<input type="password" autoComplete="off" spellCheck={false} value={token} disabled={busy} onChange={event => { setToken(event.target.value); setConfirmed(null); }}/></label>
      {confirmed && <>
        <p>{t("Connect to %1?", confirmed.host)}</p>
        {local && <p className="banner">{t("Anyone with this file can read your connected bank transactions until you revoke access at your SimpleFIN server.")}</p>}
        {confirmed.connected && <p>{t("This replaces your current bank connection.")}</p>}
      </>}
      <div className="dialog-actions bank-actions">
        {editing && <button type="button" className="secondary" disabled={busy} onClick={() => { setEditing(false); setToken(""); setConfirmed(null); }}>{t("Cancel")}</button>}
        <button disabled={busy || !token.trim()}>{confirmed ? t("Connect") : t("Connect SimpleFIN")}</button>
      </div>
    </form>}
    {(remote?.warnings || status?.warnings || []).map((warning, index) => <p key={index} className="banner">{warning.message}</p>)}
    {groups.map(connection => <section key={connection.conn_id}>
      <h3>{connection.name}</h3>
      {remote.accounts.filter(account => account.conn_id === connection.conn_id).map(account => <div key={account.id} className="statement-link-row">
        <label>{account.name}<small className="muted">{account.balance} {account.currency}</small>
          <select disabled={busy || account.currency !== data.commodity.symbol} value={account.linked || ""} onChange={event => link(account, event.target.value)}>
            <option value="">{t("Don’t import")}</option>
            {targets.map(target => <option key={target.account} value={target.account}>{target.display_name || target.account}</option>)}
          </select>
        </label>
      </div>)}
    </section>)}
    {!!remote?.other_links?.length && <section><h3>{t("Linked through another member’s connection")}</h3><ul>{remote.other_links.map(account => <li key={account}>{accountName(account)}</li>)}</ul></section>}
  </section>;
}

export function BankSync({ api, data, disabled, onClose, onUpdated, onCategorize }) {
  const dialog = useRef(null);
  const [result, setResult] = useState(null);
  const [confirmed, setConfirmed] = useState(null);
  const [starts, setStarts] = useState({});
  const [decisions, setDecisions] = useState({});
  const [reviewAgain, setReviewAgain] = useState(false);
  const [rangeChanged, setRangeChanged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { dialog.current.show(); }, []);
  useDialogHistory({ onClose, preview: result, setPreview: setResult, dirty: false, busy });
  function accountName(target) { return data.accounts.find(account => account.account === target)?.display_name || target; }
  function setStart(account, date) {
    setStarts(previous => {
      const next = { ...previous };
      if (date) next[account] = date;
      else delete next[account];
      return next;
    });
    setDecisions(previous => {
      const next = { ...previous };
      delete next[account];
      return next;
    });
    setRangeChanged(true);
  }
  async function review(dryRun, cursors = {}) {
    setError(""); setBusy(true);
    try {
      const head = !dryRun || (result && !rangeChanged) ? result.head : (await api.request("/version")).head;
      const choices = Object.fromEntries(Object.entries(decisions).map(([account, rows]) => [account, Object.entries(rows).map(([import_id, value]) => {
        const [action, transaction] = value.split(":");
        return { import_id, action, ...(transaction && { transaction: Number(transaction) }) };
      })]));
      const reply = await api.request("/simplefin/sync", { method: "POST", head, body: { dry_run: dryRun, snapshot: rangeChanged ? null : result?.snapshot, starts, decisions: choices, cursors, limit: 100 } });
      if (dryRun) { setResult(reply); setReviewAgain(false); setRangeChanged(false); }
      else { setConfirmed(reply); onUpdated(); }
    } catch (failure) {
      setError(failure.message);
      if (failure.status === 409 || failure.code === "snapshot_expired") {
        setDecisions({});
        setReviewAgain(true);
        setRangeChanged(true);
      }
    } finally { setBusy(false); }
  }
  const canImport = result && !reviewAgain && !rangeChanged && result.counts.new + result.counts.matched > 0;
  return <dialog ref={dialog} className="bookkeeping-dialog" aria-labelledby="sync-title" onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="dialog-heading"><h2 id="sync-title">{confirmed ? t("Import complete") : result ? t("Review") : t("Sync bank transactions")}</h2><button className="quiet" disabled={busy} onClick={onClose}>{confirmed ? t("Close") : t("Cancel")}</button></div>
    {error && <p role="alert" className="banner error">{error}</p>}
    {busy && <p role="status">{t("Checking with the server…")}</p>}
    {confirmed ? <><p role="status">{t("Imported %1 new and matched %2 transactions.", confirmed.counts.new, confirmed.counts.matched)}</p><button onClick={onCategorize}>{t("Categorize")}</button></> : <>
      {result?.warnings?.map((warning, index) => <p key={index} className="banner">{warning.message}</p>)}
      {result?.accounts.map(account => <section className="bank-sync-account" key={account.account}>
        <h3>{accountName(account.account)}</h3>
        {account.other_connection ? <p>{t("Linked through another member’s connection")}</p> : account.unavailable ? <p>{t("This account is unavailable")}</p> : <>
          <label>{t("Start date")}<input type="date" value={starts[account.account] ?? account.since ?? ""} onChange={event => setStart(account.account, event.target.value)}/></label>
          <ImportReview result={account} decisions={decisions[account.account] || {}} onDecision={(id, choice) => { setDecisions(previous => ({ ...previous, [account.account]: { ...previous[account.account], [id]: choice } })); setReviewAgain(true); }}/>
          {account.next_cursor && !reviewAgain && !rangeChanged && <button className="secondary" onClick={() => review(true, { [account.account]: account.next_cursor })} disabled={busy}>{t("Load more")}</button>}
        </>}
      </section>)}
      <div className="dialog-actions"><button disabled={disabled || busy || (!!result && !canImport && !reviewAgain && !rangeChanged)} onClick={() => review(!canImport)}>{canImport ? t("Import") : t("Review")}</button></div>
    </>}
  </dialog>;
}
