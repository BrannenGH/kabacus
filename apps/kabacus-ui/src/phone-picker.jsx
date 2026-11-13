// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { useDialogHistory } from "./phone-history";
import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { t } from "./i18n";
import { phoneQuery, useMedia } from "./form-factor";
import { browse, search, words } from "./account-tree";

// Account fields hold account paths (a datalist may show the path as its text).
const pathOf = option => option.value.includes(":") ? option.value : option.textContent;
const isAccountField = el => el?.matches("select,input[list]") &&
  [...(el.options || el.list?.options || [])].some(option => pathOf(option).includes(":"));

// Account and date fields open a picker over the form: groups to browse, or a
// flat list of matches once something is typed. The native fields stay the
// form's values and validation boundary.
export function PhonePicker({ form }) {
  const phone = useMedia(phoneQuery);
  const [field, setField] = useState(null);
  const [query, setQuery] = useState("");
  const [path, setPath] = useState([]);
  const dialog = useRef();
  useDialogHistory({ active: !!field, onClose: () => setField(null) });
  useEffect(() => {
    if (!form.current) return;
    let pointerType = "";
    const inputFor = (event, type = event.pointerType) => {
      const el = event.target.closest('select,input[list]:not([name="description"]),input[type="date"]');
      if (type === "touch") return phone ? el : null;
      return type === "mouse" && isAccountField(el) ? el : null;
    };
    const preventNative = event => {
      if (event.type === "pointerdown") pointerType = event.pointerType;
      if (inputFor(event, event.pointerType || pointerType)) event.preventDefault();
    };
    const pick = event => {
      // WebKit can deliver a MouseEvent click after a touch pointerdown.
      const input = inputFor(event, pointerType || event.pointerType);
      pointerType = "";
      if (!input) return;
      event.preventDefault();
      const current = [...(input.options || input.list?.options || [])].find(option => option.value === input.value);
      setQuery(""); setPath(isAccountField(input) && current ? pathOf(current).split(":").slice(0, -1) : []); setField(input);
    };
    const node = form.current;
    // Open after the tap finishes so its release cannot choose an option.
    node.addEventListener("pointerdown", preventNative);
    node.addEventListener("mousedown", preventNative);
    node.addEventListener("click", pick);
    return () => {
      node.removeEventListener("pointerdown", preventNative);
      node.removeEventListener("mousedown", preventNative);
      node.removeEventListener("click", pick);
    };
  }, [phone, form]);
  useEffect(() => {
    if (!field) return;
    dialog.current.showModal();
    if (!phone) dialog.current.querySelector('input[type="search"]')?.focus();
  }, [field]);
  if (!field) return null;
  const label = field.getAttribute("aria-label") || field.labels?.[0]?.childNodes[0]?.textContent.trim() || t("Account");
  const all = [...(field.options || field.list?.options || [])]
    .filter(option => option.value)
    .map(option => pathOf(option) === option.textContent && option.textContent.includes(":") && !option.value.includes(":")
      ? { value: option.value, path: option.textContent, label: option.value }
      : { value: option.value, path: option.value, label: option.label || option.textContent || option.value });
  const tree = all.some(option => option.path.includes(":"));
  const rows = query ? search(all, query) : browse(all, tree ? path : []);
  function select(value) {
    const prototype = field.tagName === "SELECT" ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value").set.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
    field.dispatchEvent(new Event("change", { bubbles: true }));
    setField(null); field.focus();
  }
  return createPortal(<dialog className="phone-picker" ref={dialog} aria-label={label} onCancel={event => {event.preventDefault(); setField(null);}}>
    <div className="dialog-heading"><button className="quiet" onClick={() => setField(null)}>{t("Back")}</button><h2>{label}</h2></div>
    {field.type === "date" ? <input type="date" aria-label={t("Date")} defaultValue={field.value} onChange={event => select(event.target.value)} /> : <>
      <input type="search" aria-label={t("Search")} placeholder={t("Search")} value={query} onChange={event => setQuery(event.target.value)} />
      {tree && !query && <nav className="picker-path">{[label, ...path].map((name, i) =>
        <button className="quiet" key={i} disabled={i === path.length} onClick={() => setPath(path.slice(0, i))}>{i ? words(name) : name}</button>)}</nav>}
      <div className="picker-options">{rows.map((row, i) => row.group
        ? <button className="quiet" key={`g-${i}`} onClick={() => setPath(row.group)}>{row.label} ›</button>
        : <button className="quiet" key={`${row.option.value}-${i}`} aria-label={row.option.label} onClick={() => select(row.option.value)}>
          {row.option.label}{row.path && <small>{row.path}</small>}{row.option.value === field.value && " ✓"}</button>)}</div>
      {field.tagName !== "SELECT" && query && <button onClick={() => select(query)}>{query}</button>}
    </>}
  </dialog>, document.body);
}
