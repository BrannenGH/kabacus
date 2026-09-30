// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React, { useState } from "react";
import { t } from "./i18n";
export function CardDetails({ card, api, onOperation }) {
  const [detail, setDetail] = useState(null);
  const [error, setError] = useState("");
  async function open(event) {
    if (!event.currentTarget.open || detail) return;
    try {
      const response = await api.request(
        `/cards/reserves?card=${encodeURIComponent(card.card)}`,
      );
      setDetail(response.cards.find((item) => item.card === card.card));
    } catch (e) {
      setError(e.message);
    }
  }
  return (
    <details onToggle={open}>
      <summary>{t("What the reserve holds")}</summary>
      {error && <p className="banner error">{error}</p>}
      {detail?.sources.map((source, i) => (
        <div className="list-row" key={i}>
          <span>{source.name}</span>
          <span className="amount">{source.amount.formatted}</span>
        </div>
      ))}
      <button
        className="secondary"
        disabled={!onOperation}
        onClick={() => onOperation({ kind: "fix", card: card.card })}
      >
        {t("Add reserve lines")}
      </button>
    </details>
  );
}
