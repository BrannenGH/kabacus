// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { useDialogHistory } from "./phone-history";
import { PhonePicker } from "./phone-picker";
import { useMedia, phoneQuery, desktopQuery } from "./form-factor";
import React, { useEffect, useRef, useState } from "react";
import { AmountField } from "./amount-field";
import { operation, accountCurrency } from "./operations";
import { t } from "./i18n";
const titles = {
  postings: "Postings",
  transfer: "Transfer",
  status: "Review",
  edit: "Edit transaction",
  void: "Void",
  purchase: "Card purchase",
  payment: "Pay a card",
  reconcile: "Reconcile",
  fix: "Fix reserve lines",
  split: "Split with someone",
  settle: "Settle up with someone",
  "account-type": "Account type",
};
export function OperationDialog({
  disabled,
  kind: initialKind,
  data,
  api,
  tx,
  card,
  account,
  prefill,
  onClose,
  onSaved,
  onInspect,
  onSwitch,
}) {
  const [kind, setKind] = useState(initialKind);
  const wide = useMedia(desktopQuery);
  const phone = useMedia(phoneQuery);
  const generation = useRef(0);
  const timer = useRef();
  const previewRef = useRef();
  const busyRef = useRef();
  const dialog = useRef();
  const formRef = useRef();
  const draft = useRef({});
  const [preview, setPreview] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [stale, setStale] = useState(false);
  const [conflictDiff,setConflictDiff]=useState("");
  async function previewAgain(){head.current=(await api.request("/version")).head;setStale(false);setError("");setPreview(null);}
  async function theirChange(){const history=await api.request("/history");if(history[0])setConflictDiff(await api.text(`/history/${history[0].hash}`));}
  const [dirty, setDirty] = useState(false);
  useDialogHistory({ onClose, preview, setPreview, dirty, busy });
  const [checked, setChecked] = useState(null);
  const presetCard = card || data.cash.cards.find(c => c.card === account)?.card || "";
  const [repairCard, setRepairCard] = useState(presetCard);
  const [raw, setRaw] = useState(false);
  const postingEdit = kind === "edit" && (raw || tx.postings.length !== 2 ||
    tx.postings.some(p => p.cost || p.amount.commodity !== data.commodity.symbol));
  const [rows, setRows] = useState(postingEdit ? tx.postings.map((_, i) => i) : [0, 1]);
  const [priceKinds, setPriceKinds] = useState(Object.fromEntries((tx?.postings || []).map((p, i) => [i, p.cost?.kind || ""])));
  const [currency, setCurrency] = useState(accountCurrency(data, account));
  const nextRow = useRef(postingEdit ? tx.postings.length : 2);
  const snapshot = useRef(data);
  const head = useRef(data.version.head);
  useEffect(() => {
    dialog.current.show();
  }, []);
  useEffect(() => {
    if (preview) return;
    for (const [name, value] of Object.entries(draft.current)) {
      const field = formRef.current?.elements.namedItem(name);
      if (field) {
        if (name === "ids")
          for (const item of field.length === undefined
            ? [field]
            : Array.from(field))
            item.checked = value.includes(item.value);
        else field.value = value;
      }
    }
  }, [preview, phone]);
  function viewRaw() {
    if (dirty && !window.confirm(t("Discard this change?"))) return;
    draft.current = {};
    setDirty(false);
    setPreview(null);
    setRows(tx.postings.map((_, i) => i));
    nextRow.current = tx.postings.length;
    setRaw(true);
  }
  const close = () => {
    if (!busy && (!dirty || window.confirm(t("Discard this change?"))))
      onClose();
  };
  previewRef.current = preview;
  busyRef.current = busy;
  useEffect(() => () => clearTimeout(timer.current), []);
  function schedule() {
    clearTimeout(timer.current);
    if (!wide || disabled || stale) return;
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
    if (["postings", "transfer"].includes(kind) && /^account[0-9]+$/.test(event.target.name)) {
      const field = event.currentTarget.elements.namedItem("currency" + event.target.name.slice(7));
      if (field) field.value = accountCurrency(data, event.target.value);
    }
    setDirty(true);
    event.currentTarget.dataset.dirty = "true";
    setChecked(null);
    const form = new FormData(event.currentTarget);
    draft.current = Object.fromEntries(form);
    if (kind === "fix") draft.current.ids = form.getAll("ids");
    generation.current++;
    setPreview(null);
    setError("");
    schedule();
  }
  async function submit(event) {
    event.preventDefault();
    clearTimeout(timer.current);
    const currentGeneration = generation.current;
    if (busy || stale || disabled) return;
    setBusy(true);
    setError("");
    try {
      const formData = new FormData(event.currentTarget);
      const fields = Object.fromEntries(formData);
      if (kind === "fix") fields.ids = formData.getAll("ids");
      const call =
        preview?.call || operation(kind, fields, snapshot.current, tx);
      if (kind === "reconcile" && !preview) {
        const check = await api.request(call.path, {
          method: call.method,
          head: head.current,
          body: {
            ...call.body,
            mode: "check",
            dry_run: true,
          },
        });
        if (currentGeneration !== generation.current) return;
        setChecked(check);
        if (check.status === "mismatch") return;
        if (check.status !== "matched")
          throw new Error(t("The server had a problem."));
      }
      const result = await api.request(call.path, {
        method: call.method,
        head: head.current,
        body: {
          ...call.body,
          dry_run: !preview,
        },
      });
      if (preview) {
        onSaved(result);
        return;
      }
      if (currentGeneration !== generation.current) return;
      setPreview({
        call,
        result: result.written || {
          ...result,
          balances: result.balances || result.balance_changes || [],
          warnings:
            result.warnings ||
            result.ops?.flatMap((op) => op.warnings || []) ||
            [],
        },
      });
    } catch (e) {
      if (currentGeneration !== generation.current) return;
      setError(e.message);
      if (e.status === 409) {
        setStale(true);
        setPreview(null);
      }
    } finally {
      setBusy(false);
    }
  }
  const localDate = new Date();
  const typeAccount = snapshot.current.accounts.find((a) => a.account === account);
  const date = `${localDate.getFullYear()}-${String(localDate.getMonth() + 1).padStart(2, "0")}-${String(localDate.getDate()).padStart(2, "0")}`;
  const amountField = !postingEdit && !["fix", "status", "postings", "transfer", "account-type"].includes(kind) &&
    <AmountField data={data} input={{name:"amount",autoFocus:phone,
      pattern:kind === "reconcile" ? "-?[0-9]+([.][0-9]+)?" : "[0-9]+([.][0-9]+)?",
      defaultValue:tx?.postings[0]?.amount.exact.replace(/^-/, "") || prefill?.amount}}
      currency={kind === "reconcile" ? {name:"currency",value:currency,onChange:e=>setCurrency(e.target.value)} : null} />;
  return (
    <dialog
      aria-labelledby="operation-title"
      ref={dialog}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      className="bookkeeping-dialog"
    >
      <form ref={formRef} onSubmit={submit} onChange={changed}>
        <div className="dialog-heading">
          <h2 id="operation-title">{phone && preview ? t("Review") : t(titles[kind])}</h2>
          {phone && kind === "edit" && <details className="operation-overflow"><summary aria-label={t("More")}>⋮</summary><div>{!postingEdit && <button type="button" className="quiet" onClick={viewRaw}>{t("View as raw postings")}</button>}<button type="button" className="quiet" onClick={() => {setKind("void"); setPreview(null); setDirty(true);}}>{t("Void")}</button></div></details>}
          {!phone && kind === "edit" && !postingEdit && <button type="button" className="quiet" onClick={viewRaw} disabled={busy}>{t("View as raw postings")}</button>}
          <button
            type="button"
            className="quiet"
            onClick={close}
            disabled={busy}
          >
            {t("Cancel")}
          </button>
        </div>
        {stale && <section className="conflict-panel"><h3>{t("Someone saved first")}</h3><button type="button" onClick={()=>theirChange().catch(e=>setError(e.message))}>{t("See their change")}</button><button type="button" onClick={()=>setStale(false)}>{t("Keep editing")}</button><button type="button" onClick={()=>previewAgain().catch(e=>setError(e.message))}>{t("Preview again")}</button>{conflictDiff&&<pre>{conflictDiff}</pre>}</section>}
        {error && (
          <p role="alert" className="banner error">
            {error}
          </p>
        )}
        {
          <div className={wide ? "editor-grid" : undefined}>
            <div hidden={!wide && !!preview}>
              {kind === "void" ? (
                <p className="empty">
                  {t("Only the reversal: it didn't happen.")}
                </p>
              ) : (
                <div className="form-fields">
{phone && amountField}
                  {kind === "account-type" && (
                    <>
                      <label>
                        {t("Account")}
                        <select name="account" required defaultValue={account}>
                          <option value={account}>
                            {typeAccount?.display_name || account}
                          </option>
                        </select>
                      </label>
                      <label>
                        {t("Account type")}
                        <select
                          name="synthetic"
                          required
                          defaultValue={String(!!typeAccount?.synthetic)}
                        >
                          <option value="false">{t("Real account")}</option>
                          <option value="true">{t("Synthetic account")}</option>
                        </select>
                      </label>
                    </>
                  )}
                  {!["status", "account-type"].includes(kind) && (
                    <label>
                      {t("Date")}
                      <input
                        type="date"
                        name="date"
                        required
                        defaultValue={tx?.date || date}
                      />
                    </label>
                  )}
                  {["edit", "purchase", "split", "postings", "transfer"].includes(kind) && (
                    <label>
                      {t("Payee")}
                      <input
                        name="description"
                        required
                        defaultValue={
                          tx
                            ? [tx.payee, tx.memo].filter(Boolean).join(" | ")
                            : [prefill?.description, prefill?.memo].filter(Boolean).join(" | ")
                        }
                      />
                    </label>
                  )}
                  {(["postings", "transfer"].includes(kind) || postingEdit) && (
                    <>
                      <datalist id="posting-accounts">
                        {data.accounts.map((a) => (
                          <option key={a.account} value={a.account}>
                            {a.display_name || a.account}
                          </option>
                        ))}
                      </datalist>
                      {rows.map((row, i) => (
                        <fieldset key={row}>
                          <legend>{kind === "transfer" && i < 2 ? t(i === 0 ? "From" : "To") : t("%1 · %2", t("Postings"), i + 1)}</legend>
                          <label>
                            {t("Account")}
                            <input
                              defaultValue={postingEdit ? tx.postings[row]?.account : kind === "transfer" && row === 0 ? account || "" : ""}
                              name={"account" + row}
                              list="posting-accounts"
                              required
                            />
                          </label>
                          <AmountField data={data} input={{name:"amount" + row,
                            defaultValue:postingEdit ? tx.postings[row]?.amount.exact : "",
                            pattern:kind === "transfer" ? "[0-9]+([.][0-9]+)?" : "-?[0-9]+([.][0-9]+)?"}}
                            currency={{name:"currency" + row,defaultValue:postingEdit ? tx.postings[row]?.amount.commodity : accountCurrency(data, kind === "transfer" && row === 0 ? account : "")}} />
                          <label>
                            {t("Details")}
                            <select name={"cost_kind" + row} value={priceKinds[row] || "none"} onChange={e => setPriceKinds({...priceKinds, [row]: e.target.value})}>
                              <option value="none">—</option>
                              <option value="unit">{t("Unit price")}</option>
                              <option value="total">{t("Total price")}</option>
                            </select>
                          </label>
                          {priceKinds[row] && priceKinds[row] !== "none" &&
                            <AmountField data={data} label={t(priceKinds[row] === "unit" ? "Unit price" : "Total price")}
                              input={{name:"cost" + row,pattern:"[0-9]+([.][0-9]+)?",defaultValue:tx?.postings[row]?.cost?.amount.exact}}
                              currency={{name:"cost_currency" + row,defaultValue:tx?.postings[row]?.cost?.amount.commodity || data.commodity.symbol}} />}
                          {rows.length > 2 && (kind !== "transfer" || row > 1) && (
                            <button
                              type="button"
                              className="quiet"
                              aria-label={t("%1 · %2", t("Postings"), i + 1)}
                              onClick={() => {
                                for (const key of ["account", "amount", "currency", "cost_kind", "cost", "cost_currency"]) delete draft.current[key + row];
                                setRows(rows.filter((r) => r !== row));
                                setDirty(true);
                                generation.current++;
                                setPreview(null);
                                schedule();
                              }}
                            >
                              −
                            </button>
                          )}
                        </fieldset>
                      ))}
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => {
                          setRows([...rows, nextRow.current++]);
                          setDirty(true);
                          generation.current++;
                          setPreview(null);
                        }}
                      >
                        {t("Add")}
                      </button>
                    </>
                  )}
                  {kind === "status" && (
                    <>
                      <label>
                        {t("Account")}
                        <select name="account" required>
                          {tx.postings.map((p, i) => (
                            <option key={i} value={p.account}>
                              {p.account}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label>
                        {t("Review")}
                        <select name="status" required>
                          <option value="unmarked">{t("Unknown")}</option>
                          <option value="pending">{t("Review")}</option>
                          <option value="cleared">{t("Check")}</option>
                        </select>
                      </label>
                    </>
                  )}
                  {kind === "edit" && !postingEdit &&
                    [
                      ["from", "From", true],
                      ["to", "To", false],
                    ].map(([field, label, negative]) => (
                      <label key={field}>
                        {t(label)}
                        <select
                          name={field}
                          defaultValue={
                            tx.postings.find(
                              (p) =>
                                p.amount.exact.startsWith("-") === negative,
                            )?.account
                          }
                          required
                        >
                          {data.accounts.map((a) => (
                            <option key={a.account} value={a.account}>
                              {a.display_name || a.account}
                            </option>
                          ))}
                        </select>
                      </label>
                    ))}
                  {kind === "purchase" && (
                    <p className="banner">
                      {t("This is a card purchase: its reserve lines are added for you.")}{" "}
                      <button
                        type="button"
                        className="quiet"
                        onClick={() => {
                          const fields = Object.fromEntries(new FormData(formRef.current));
                          onSwitch?.({ card: fields.card, description: fields.description, amount: fields.amount, category: fields.category });
                        }}
                      >
                        {t("Switch to Spent")}
                      </button>
                    </p>
                  )}
                  {["purchase", "payment", "fix"].includes(kind) && (
                    <label>
                      {t("Card")}
                      <select
                        name="card"
                        required
                        onChange={(e) => setRepairCard(e.target.value)}
                        defaultValue={presetCard}
                      >
                        <option value="">{t("Card")}</option>
                        {data.cash.cards.map((c) => (
                          <option value={c.card} key={c.card}>
                            {c.display_name || c.card}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {kind === "settle" && (
                    <>
                      <label>
                        {t("Direction")}
                        <select name="direction" required>
                          <option value="paid">{t("I paid them")}</option>
                          <option value="received">{t("They paid me")}</option>
                        </select>
                      </label>
                      <label>
                        {t("Account")}
                        <select name="account" defaultValue={account || ""} required>
                          {data.accounts
                            .filter((a) => ["money", "revolving-debt"].includes(a.kind))
                            .map((a) => (
                              <option key={a.account} value={a.account}>
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </select>
                      </label>
                      <label>
                        {t("Who")}
                        <input
                          name="person"
                          list="settle-people"
                          required
                          defaultValue={
                            data.accounts
                              .find((a) => a.kind === "receivable")
                              ?.account.replace(/^assets:receivable:/, "") || ""
                          }
                        />
                        <datalist id="settle-people">
                          {data.accounts
                            .filter((a) => a.kind === "receivable")
                            .map((a) => (
                              <option key={a.account} value={a.account.replace(/^assets:receivable:/, "")}>
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </datalist>
                      </label>
                    </>
                  )}
                  {kind === "split" && (
                    <>
                      <label>
                        {t("Paid with")}
                        <select name="paid_with" defaultValue={account || ""} required>
                          {data.accounts
                            .filter((a) =>
                              ["money", "revolving-debt"].includes(a.kind),
                            )
                            .map((a) => (
                              <option key={a.account} value={a.account}>
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </select>
                      </label>
                      <label>
                        {t("Category")}
                        <select name="category" required>
                          {data.accounts
                            .filter((a) => a.kind === "expense")
                            .map((a) => (
                              <option key={a.account} value={a.account}>
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </select>
                      </label>
                      <label>
                        {t("Who")}
                        <input
                          name="person"
                          list="shared-people"
                          required
                          defaultValue={
                            data.accounts
                              .find((a) => a.kind === "receivable")
                              ?.account.replace(/^assets:receivable:/, "") || ""
                          }
                        />
                        <datalist id="shared-people">
                          {data.accounts
                            .filter((a) =>
                              ["receivable", "payable"].includes(a.kind),
                            )
                            .map((a) => (
                              <option
                                key={a.account}
                                value={a.account.replace(
                                  /^(assets:receivable|liabilities:payable):/,
                                  "",
                                )}
                              >
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </datalist>
                      </label>
                      <label>
                        {t("Their share")}
                        <input
                          name="their_share"
                          required
                          inputMode="decimal"
                          pattern="[0-9]+([.][0-9]+)?"
                        />
                      </label>
                    </>
                  )}
                  {kind === "purchase" && (
                    <>
                      <label>
                        {t("Category")}
                        <select name="category" required defaultValue={prefill?.category || undefined}>
                          {data.accounts
                            .filter((a) => a.kind === "expense")
                            .map((a) => (
                              <option key={a.account} value={a.account}>
                                {a.display_name || a.account}
                              </option>
                            ))}
                        </select>
                      </label>
                      <label>
                        {t("Paid from:")}
                        <select name="source">
                          <option value="free">{t("Free money")}</option>
                          {data.cash.funds.map((f) => (
                            <option key={f.account} value={f.account}>
                              {f.display_name || f.account}
                            </option>
                          ))}
                        </select>
                      </label>
                    </>
                  )}
                  {kind === "fix" &&
                    snapshot.current.transactions
                      .filter((tx) =>
                        tx.missing_reserve?.some((m) => m.card === repairCard),
                      )
                      .map((tx) => (
                        <label className="checkbox" key={tx.id}>
                          <input
                            type="checkbox"
                            name="ids"
                            value={String(tx.id)}
                            defaultChecked
                          />
                          {t("%1 · %2", tx.date, tx.payee)}
                        </label>
                      ))}
                  {kind === "fix" && (
                    <label>
                      {t("Paid from:")}
                      <select name="source">
                        <option value="free">{t("Free money")}</option>
                        {data.cash.funds.map((f) => (
                          <option key={f.account} value={f.account}>
                            {f.display_name || f.account}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {kind === "reconcile" && (
                    <label>
                      {t("Account")}
                      <select name="account" defaultValue={account} required onChange={e => setCurrency(accountCurrency(data, e.target.value))}>
                        {data.accounts
                          .filter((a) =>
                            [
                              "money",
                              "revolving-debt",
                              "term-liability",
                              "non-liquid",
                            ].includes(a.kind),
                          )
                          .map((a) => (
                            <option key={a.account} value={a.account}>
                              {a.display_name || a.account}
                            </option>
                          ))}
                      </select>
                    </label>
                  )}
{!phone && amountField}
                </div>
              )}
            </div>
            {preview && (
              <div className="preview">
                <h3>{t("Review before saving")}</h3>
                {kind === "account-type" && (
                  <>
                    <p>{typeAccount?.display_name || account}</p>
                    <p>
                      {t(
                        "%1 → %2",
                        t(typeAccount?.synthetic ? "Synthetic account" : "Real account"),
                        t(preview.call.body.synthetic ? "Synthetic account" : "Real account"),
                      )}
                    </p>
                  </>
                )}
                <pre>
                  {preview.result.preview ||
                    preview.result.after ||
                    t("The books still check out with this change")}
                </pre>
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
                {(preview.result.balances || []).map((b, i) => (
                  <div className="list-row" key={i}>
                    <span>{b.account}</span>
                    <span>
                      {t("%1 → %2", b.before.formatted, b.after.formatted)}
                    </span>
                  </div>
                ))}
                {(preview.result.warnings || []).map((w, i) => (
                  <p className="banner warning" key={i}>
                    {w.message || w.code}
                  </p>
                ))}
              </div>
            )}
          </div>
        }
        {checked && (
          <section className="home">
            <p role="status">
              {checked.status === "matched"
                ? t(
                    "Matched: %1 on %2, the same as the statement.",
                    checked.ledger_balance,
                    checked.date,
                  )
                : t(
                    "Doesn't match yet: the books say %1, the statement %2 (difference %3). Nothing has been written.",
                    checked.ledger_balance,
                    checked.statement_balance,
                    checked.difference,
                  )}
            </p>
            {checked.status === "mismatch" && (
              <>
                <h3>{t("Likely explanations")}</h3>
                {[
                  ...(checked.suggestions || []),
                  ...(checked.later_transactions || []),
                ].map((set, i) => (
                  <div key={i}>
                    {set.map((tx) => (
                      <div className="list-row" key={tx.id}>
                        <span>
                          {tx.date} {tx.description}
                        </span>
                        <span className="amount">{tx.amount}</span>
                        {onInspect &&
                          snapshot.current.transactions.find(
                            (t) => t.id === tx.id,
                          ) && (
                            <button
                              type="button"
                              className="quiet"
                              onClick={() =>
                                onInspect({
                                  kind: "edit",
                                  tx: snapshot.current.transactions.find(
                                    (t) => t.id === tx.id,
                                  ),
                                })
                              }
                            >
                              {t("Open transaction")}
                            </button>
                          )}
                      </div>
                    ))}
                  </div>
                ))}
              </>
            )}
          </section>
        )}
        <div className="dialog-actions">
          {preview && (
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => setPreview(null)}
            >
              {t("Back to edit")}
            </button>
          )}
          <button
            disabled={
              busy ||
              stale ||
              disabled ||
              (["edit", "account-type"].includes(kind) && !dirty) ||
              preview?.result.warnings?.some((w) => w.code === "unchanged")
            }
          >
            {t(
              busy ? "Checking with the server…" : preview ? "Save" : phone ? "Review" : "Preview",
            )}
          </button>
        </div>
      </form>
      <PhonePicker form={formRef} />
    </dialog>
  );
}
