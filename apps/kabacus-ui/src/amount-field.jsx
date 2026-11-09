// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React from "react";
import { t } from "./i18n";
import { currencies } from "./operations";

export function AmountField({ data, label = t("Amount"), input, currency }) {
  return <label className="amount-field">
    {label}
    <span className="amount-entry">
      <input inputMode="decimal" required {...input} aria-label={input["aria-label"] || label} />
      {currency ? <select {...currency} aria-label={t("Currency")}>
        {currencies(data).map(c => <option key={c.symbol} value={c.symbol}>{c.symbol}</option>)}
      </select> : <span className="amount-currency">{data.commodity.symbol}</span>}
    </span>
  </label>;
}
