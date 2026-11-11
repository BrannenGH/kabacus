// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef } from "react";
import { phoneQuery, useMedia } from "./form-factor";
import { t } from "./i18n";
const dialogs = [];

// An edited form is open and the person chose to keep it.
export const keepsUnsavedChange = () =>
  !!document.querySelector('.bookkeeping-dialog form[data-dirty="true"]') && !window.confirm(t("Discard this change?"));

export function useDialogHistory({ active = true, onClose, preview, setPreview, dirty, busy }) {
  const phone = useMedia(phoneQuery);
  const id = useRef(crypto.randomUUID());
  const state = useRef();
  state.current = { onClose, preview, setPreview, dirty, busy };
  useEffect(() => {
    if (!phone || !active) return;
    const token = id.current;
    const base = history.state;
    const entry = review => ({ ...base, phoneDialog: token, review });
    const handoff = base?.phoneDialog && !dialogs.includes(base.phoneDialog);
    if (handoff) history.replaceState(entry(false), "");
    else history.pushState(entry(false), "");
    dialogs.push(token);
    const back = event => {
      if (dialogs.at(-1) !== token) return;
      event.stopImmediatePropagation();
      const current = state.current;
      if (current.busy) { history.pushState(entry(!!current.preview), ""); return; }
      if (event.state?.phoneDialog === token) { current.setPreview?.(null); return; }
      if (current.dirty && !window.confirm(t("Discard this change?"))) {
        history.pushState(entry(!!current.preview), ""); return;
      }
      current.onClose();
    };
    window.addEventListener("popstate", back, true);
    return () => {
      window.removeEventListener("popstate", back, true);
      dialogs.splice(dialogs.indexOf(token), 1);
      // A replacement dialog can take over this entry in the same render.
      queueMicrotask(() => {
        if (history.state?.phoneDialog === token) history.go(history.state.review ? -2 : -1);
      });
    };
  }, [phone, active]);
  useEffect(() => {
    if (!phone || !active || history.state?.phoneDialog !== id.current) return;
    if (preview && !history.state.review) history.pushState({ ...history.state, review: true }, "");
    else if (!preview && history.state.review) history.back();
  }, [phone, active, !!preview]);
  useEffect(() => {
    if (!dirty) return;
    const leave = (event) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", leave);
    return () => window.removeEventListener("beforeunload", leave);
  }, [dirty]);
}
