// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useEffect, useRef, useState } from "react";
import { t } from "./i18n";
import { useDialogHistory } from "./phone-history";

function encoded(bytes) {
  let text = "";
  for (let index = 0; index < bytes.length; index += 8192) {
    text += String.fromCharCode(...bytes.subarray(index, index + 8192));
  }
  return btoa(text);
}

function columnOptions(columns) {
  return <><option value="">{t("None")}</option>{columns.map((name, index) => <option key={index} value={index}>{name}</option>)}</>;
}

function Mapping({ inspection, value, onChange }) {
  const columns = inspection.columns;
  const [split, setSplit] = useState(value.amount == null && value.debit != null);
  const field = (key, label) => <label key={key}>{label}<select value={value[key] ?? ""} onChange={event => onChange({ ...value, [key]: event.target.value === "" ? null : Number(event.target.value) })}>{columnOptions(columns)}</select></label>;
  return <div className="form-fields">
    <h3>{t("Match CSV columns")}</h3>
    <p className="muted">{inspection.sample[0]?.join(" · ")}</p>
    {field("date", t("Date"))}
    <label>{t("Date format")}<select value={value.date_format ?? ""} onChange={event => onChange({ ...value, date_format: event.target.value || null })}>
      <option value="">{t("Automatic")}</option><option value="%Y-%m-%d">2026-10-25</option><option value="%m/%d/%Y">10/25/2026</option><option value="%d/%m/%Y">25/10/2026</option><option value="%d.%m.%Y">25.10.2026</option>
    </select></label>
    <fieldset><legend>{t("Description")}</legend>{columns.map((name, index) => <label className="checkbox" key={index}><input type="checkbox" checked={value.description?.includes(index) ?? false} onChange={event => onChange({ ...value, description: event.target.checked ? [...(value.description || []), index] : value.description.filter(column => column !== index) })}/>{name}</label>)}</fieldset>
    <label className="checkbox"><input type="checkbox" checked={split} onChange={event => { setSplit(event.target.checked); onChange({ ...value, amount: null, debit: null, credit: null }); }}/>{t("Separate debit and credit columns")}</label>
    {split ? <>{field("debit", t("Debit"))}{field("credit", t("Credit"))}</> : <>{field("amount", t("Amount"))}<label className="checkbox"><input type="checkbox" checked={value.invert ?? false} onChange={event => onChange({ ...value, invert: event.target.checked })}/>{t("Reverse amount signs")}</label></>}
    {field("id", t("Transaction ID"))}
    {field("balance", t("Balance"))}
  </div>;
}

export function ImportReview({ result, decisions, onDecision }) {
  const { counts, balance_check: balance } = result;
  const currency = result.source?.currency;
  const amount = value => currency ? `${value} ${currency}` : value;
  return <div className="preview">
    <h3>{t("Review before saving")}</h3>
    <dl className="statement-counts">{[[t("New transactions"), counts.new], [t("Matched"), counts.matched], [t("Already imported"), counts.duplicate], [t("Skipped"), counts.skipped]].map(([label, count]) => <div key={label}><dt>{label}</dt><dd>{count}</dd></div>)}</dl>
    {result.warnings?.map((warning, index) => <p className="banner" key={`${warning.code}:${index}`}>{warning.message}</p>)}
    {balance && <section><h4>{t("Balance check")}</h4><dl className="statement-counts">
      {[[t("Date"), balance.date], [t("Statement balance"), amount(balance.reported)], [t("Books balance"), amount(balance.books)], [t("Difference"), amount(balance.difference)]].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}
    </dl></section>}
    <ul className="statement-rows">{result.rows.map((row, index) => <li key={`${row.import_id}:${index}`}>
      <div className="list-row"><span><strong>{row.description}</strong><small>{row.date}</small></span><span className="amount">{amount(row.amount)}</span></div>
      {row.outcome !== "duplicate" && <label>{t("Action")}<select aria-label={`${t("Action")} ${row.description}`} value={decisions[row.import_id] ?? (row.transaction ? `match:${row.transaction}` : row.outcome === "skipped" ? "skip" : "add")} onChange={event => onDecision(row.import_id, event.target.value)}>
        {row.candidates?.map(id => <option key={id} value={`match:${id}`}>{t("Match transaction %1", id)}</option>)}
        <option value="add">{t("Import as new")}</option><option value="skip">{t("Skip")}</option>
      </select></label>}
    </li>)}</ul>
  </div>;
}

export function StatementImport({ account, api, data, disabled, onClose, onUpdated, onCategorize }) {
  const dialog = useRef(null);
  const heading = useRef(null);
  const head = useRef(data.version.head);
  const key = `kabacus:csv-mapping:${data.version.ledger ?? ""}:${account}`;
  const [file, setFile] = useState(null);
  const [selection, setSelection] = useState(null);
  const [inspection, setInspection] = useState(null);
  const [mapping, setMapping] = useState({});
  const [statement, setStatement] = useState(0);
  const [result, setResult] = useState(null);
  const [saved, setSaved] = useState(null);
  const [decisions, setDecisions] = useState({});
  const [reviewed, setReviewed] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [stale, setStale] = useState(false);
  const accountName = data.accounts.find(item => item.account === account)?.display_name || account;
  const path = `/accounts/${encodeURIComponent(account)}/import`;
  useEffect(() => { dialog.current.show(); }, []);
  useEffect(() => { if (result || saved) heading.current?.focus(); }, [result, saved]);
  useDialogHistory({ onClose, preview: result, setPreview: setResult, dirty: false, busy });

  function savedMapping(inspected) {
    try {
      const remembered = JSON.parse(localStorage.getItem(key));
      if (remembered?.columns.join("\0") === inspected.columns.join("\0")) return remembered.mapping;
    } catch { /* Storage may be unavailable. */ }
    return inspected.mapping || { description: [], header: inspected.header, delimiter: inspected.delimiter };
  }
  async function request(contents, extras, dryRun, expected = head.current, selections = decisions) {
    setBusy(true);
    setError("");
    try {
      const reply = await api.request(path, { method: "POST", head: expected, body: { ...contents, ...extras, dry_run: dryRun } });
      if (reply.csv) {
        setInspection(reply.csv);
        setMapping(savedMapping(reply.csv));
      } else if (reply.statements) {
        setSelection(reply.statements);
        setStatement(reply.statements[0].statement);
      } else if (dryRun) {
        setResult(reply);
        setReviewed(selections);
        if (inspection) {
          try { localStorage.setItem(key, JSON.stringify({ columns: inspection.columns, mapping })); } catch { /* Storage may be unavailable. */ }
        }
      } else {
        setSaved(reply);
        onUpdated();
      }
    } catch (failure) {
      setError(failure.message);
      if (failure.status === 409) setStale(true);
    } finally { setBusy(false); }
  }
  async function choose(event) {
    const chosen = event.target.files[0];
    if (!chosen) return;
    const selections = {};
    setFile(null); setResult(null); setInspection(null); setSelection(null); setDecisions(selections); setStale(false);
    setBusy(true);
    try {
      const contents = { filename: chosen.name, format: chosen.name.split(".").at(-1).toLowerCase(), content_base64: encoded(new Uint8Array(await chosen.arrayBuffer())) };
      setFile(contents);
      await request(contents, {}, true, head.current, selections);
    } catch (failure) { setError(failure.message); setBusy(false); }
  }
  function options() {
    const selected = Object.entries(decisions).map(([import_id, value]) => {
      const [action, transaction] = value.split(":");
      return { import_id, action, ...(transaction && { transaction: Number(transaction) }) };
    });
    return { ...(inspection && { mapping }), ...(selection && { statement }), decisions: selected, limit: 100 };
  }
  async function review(cursor) {
    if (stale) {
      try { head.current = (await api.request("/version")).head; setStale(false); }
      catch (failure) { setError(failure.message); return; }
    }
    await request(file, { ...options(), cursor }, true);
  }
  const mappingComplete = !inspection || (mapping.date != null && mapping.description?.length && (mapping.amount != null || (mapping.debit != null && mapping.credit != null)));
  const canSave = result && reviewed === decisions && !stale && (result.counts.new + result.counts.matched > 0);
  return <dialog ref={dialog} className="bookkeeping-dialog" aria-labelledby="statement-import-title" onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="dialog-heading"><div><h2 id="statement-import-title" ref={heading} tabIndex={-1}>{saved ? t("Import complete") : result ? t("Review") : t("Import statement…")}</h2><p className="muted">{accountName}</p></div><button type="button" className="quiet" disabled={busy} onClick={onClose}>{saved ? t("Close") : t("Cancel")}</button></div>
    {error && <p className="banner error" role="alert">{error}</p>}
    {busy && <p role="status">{t("Checking with the server…")}</p>}
    {saved ? <><p role="status">{t("Imported %1 new and matched %2 transactions.", saved.counts.new, saved.counts.matched)}</p><button onClick={onCategorize}>{t("Categorize")}</button></> : <>
      {!result && <label>{t("Statement file")}<input type="file" accept=".ofx,.qfx,.csv" disabled={disabled || busy} onChange={choose}/></label>}
      {selection && !result && <label>{t("Statement")}<select value={statement} onChange={event => setStatement(Number(event.target.value))}>{selection.map(item => <option key={item.statement} value={item.statement}>{item.account_hint || item.statement + 1} · {item.kind === "money" ? t("Bank & cash") : t("Credit")} · {item.rows}</option>)}</select></label>}
      {inspection && !result && <Mapping inspection={inspection} value={mapping} onChange={setMapping}/>}
      {result && <ImportReview result={result} decisions={decisions} onDecision={(id, value) => setDecisions(old => ({ ...old, [id]: value }))}/>}
      {result?.next_cursor && reviewed === decisions && <button className="secondary" disabled={busy} onClick={() => review(result.next_cursor)}>{t("Load more")}</button>}
      <div className="dialog-actions">
        {result && <button type="button" className="secondary" disabled={busy} onClick={() => setResult(null)}>{t("Back to edit")}</button>}
        {file && (canSave ? <button disabled={busy || disabled} onClick={() => request(file, options(), false, result.head)}>{t("Import")}</button> : <button disabled={busy || disabled || !mappingComplete || (result && reviewed === decisions && !stale)} onClick={() => review()}>{t(stale ? "Review again" : "Review")}</button>)}
      </div>
    </>}
  </dialog>;
}
