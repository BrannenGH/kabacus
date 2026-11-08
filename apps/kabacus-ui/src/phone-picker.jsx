// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { useDialogHistory } from "./phone-history";
import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { t } from "./i18n";
import { phoneQuery, useMedia } from "./form-factor";

// Keep the native fields as the form's values and validation boundary.
export function PhonePicker({ form }) {
  const phone = useMedia(phoneQuery);
  const [field, setField] = useState(null);
  const [query, setQuery] = useState("");
  const dialog = useRef();
  useDialogHistory({ active: !!field, onClose: () => setField(null) });
  useEffect(() => {
    if (!phone || !form.current) return;
    const inputFor = event => event.pointerType === "touch"
      ? event.target.closest('select,input[list]:not([name="description"]),input[type="date"]')
      : null;
    const preventNative = event => {
      if (inputFor(event)) event.preventDefault();
    };
    const pick = event => {
      const input = inputFor(event);
      if (!input) return;
      event.preventDefault();
      setQuery(""); setField(input);
    };
    const node = form.current;
    // Open after the tap finishes so its release cannot choose an option.
    node.addEventListener("pointerdown", preventNative);
    node.addEventListener("click", pick);
    return () => {
      node.removeEventListener("pointerdown", preventNative);
      node.removeEventListener("click", pick);
    };
  }, [phone, form]);
  useEffect(() => { if (field) dialog.current.showModal(); }, [field]);
  if (!field) return null;
  const label = field.labels?.[0]?.childNodes[0]?.textContent.trim() || t("Account");
  const options = [...(field.options || field.list?.options || [])].filter(option => option.value && `${option.textContent} ${option.value}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
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
      <div className="picker-options">{options.map((option, i) => <button className="quiet" key={`${option.value}-${i}`} onClick={() => select(option.value)}>{option.label || option.textContent || option.value}</button>)}</div>
      {field.tagName !== "SELECT" && query && <button onClick={() => select(query)}>{query}</button>}
    </>}
  </dialog>, document.body);
}
