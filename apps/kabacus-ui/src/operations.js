// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { exactAmount } from "./api";
import { t } from "./i18n";
export function currencies(data) {
  return data.commodity.commodities?.length ? data.commodity.commodities : [{ symbol: data.commodity.symbol, places: data.commodity.places }];
}
export function accountCurrency(data, account) {
  const balances = data.accounts?.find(a => a.account === account)?.balances || [];
  return balances.length === 1 ? balances[0].commodity : data.commodity.symbol;
}
// A card purchase: expense, card, source and reserve lines, all one amount.
// Returns the index of its expense posting.
export function cardPurchase(data, tx) {
  const ps = tx?.postings || [];
  if (ps.length !== 4 || ps.some(p => p.cost || p.amount.commodity !== data.commodity.symbol)) return -1;
  const card = data.cash.cards.find(c => ps.some(p => p.account === c.card) && ps.some(p => p.account === c.reserve));
  const reserve = ps.find(p => p.account === card?.reserve);
  if (!reserve) return -1;
  const sign = p => p.amount.exact.startsWith("-");
  const rest = ps.map((p, i) => i).filter(i => ps[i].account !== card.card && i !== ps.indexOf(reserve) && sign(ps[i]) === sign(reserve));
  return rest.length === 1 ? rest[0] : -1;
}
function posting(fields, key, data, old, transfer) {
  const index = key.slice(7);
  const symbol = fields["currency" + index] || old?.amount.commodity || data.commodity.symbol;
  const places = currencies(data).find(c => c.symbol === symbol)?.places;
  const amount = exactAmount(fields["amount" + index], symbol, places);
  if (transfer && index === "0") amount.mantissa = (-BigInt(amount.mantissa)).toString();
  const result = { account: fields[key], amount, status: "" };
  if (old) result.status = old.account === fields[key] ? old.status || "" : "";
  if (fields["cost" + index]) {
    const currency = fields["cost_currency" + index] || old?.cost?.amount.commodity || data.commodity.symbol;
    result.cost = {kind: fields["cost_kind" + index] || old?.cost?.kind || "total",
      amount: exactAmount(fields["cost" + index], currency, currencies(data).find(c => c.symbol === currency)?.places)};
  } else if (old?.cost && ["", "none"].includes(fields["cost_kind" + index])) result.clear_cost = true;
  return result;
}
export function operation(kind, fields, data, tx) {
  const amount = (value) =>
    exactAmount(value, data.commodity.symbol, data.commodity.places);
  const base = { method: "POST" };
  if (kind === "account-type")
    return {
      method: "PUT",
      path: `/accounts/${encodeURIComponent(fields.account)}/type`,
      body: { synthetic: fields.synthetic === "true" },
    };
  if (kind === "postings" || kind === "transfer")
    return {
      ...base,
      path: "/transactions",
      body: {
        date: fields.date,
        description: fields.description,
        postings: Object.keys(fields)
          .filter((key) => /^account[0-9]+$/.test(key))
          .map((key) => posting(fields, key, data, null, kind === "transfer")),
        tags: [],
      },
    };
  if (kind === "status")
    return {
      method: "PATCH",
      path: `/transactions/${tx.id}/status`,
      body: { account: fields.account, status: fields.status },
    };
  if (kind === "void")
    return {
      ...base,
      path: `/transactions/${tx.id}/corrections`,
      body: { void: true },
    };
  if (kind === "edit") {
    if (Object.keys(fields).some(key => /^account[0-9]+$/.test(key))) {
      return {
        method: "PATCH",
        path: `/transactions/${tx.id}`,
        body: {
          date: fields.date,
          description: fields.description,
          tags: tx.tags || [],
          postings: Object.keys(fields).filter(key => /^account[0-9]+$/.test(key)).map(key => posting(fields, key, data, tx.postings[Number(key.slice(7))])) ,
        },
      };
    }
    const positive = amount(fields.amount);
    const expense = cardPurchase(data, tx);
    return {
      method: "PATCH",
      path: `/transactions/${tx.id}`,
      body: {
        date: fields.date,
        description: fields.description,
        tags: tx.tags || [],
        postings: tx.postings.map((p, i) => {
          const account = expense >= 0
            ? (i === expense ? fields.category || p.account : p.account)
            : p.amount.exact.startsWith("-")
              ? fields.from || p.account
              : fields.to || p.account;
          return {
            account,
            status:
              tx.postings.find((old) => old.account === account)?.status || "",
            amount: {
              ...positive,
              mantissa: (p.amount.exact.startsWith("-")
                ? -BigInt(positive.mantissa)
                : BigInt(positive.mantissa)
              ).toString(),
            },
          };
        }),
      },
    };
  }
  if (kind === "fix") {
    const card = data.cash.cards.find((card) => card.card === fields.card);
    const reserve = data.accounts.find(
      (account) => account.account === card?.reserve,
    );
    if (!card || !reserve) throw new Error(t("No credit cards"));
    const source = fields.source === "free" ? reserve.home : fields.source;
    if (fields.ids && !fields.ids.length)
      throw new Error(t("Choose at least one transaction"));
    const ops = data.transactions
      .filter(
        (tx) =>
          (!fields.ids || fields.ids.includes(String(tx.id))) &&
          tx.missing_reserve?.some((m) => m.card === fields.card),
      )
      .map((tx) => {
        const missing = amount(
          tx.missing_reserve.find((m) => m.card === fields.card).amount.exact,
        );
        return {
          tool: "edit_transaction",
          args: {
            id: tx.id,
            date: tx.date,
            description: [tx.payee, tx.memo].filter(Boolean).join(" | "),
            tags: tx.tags || [],
            postings: [
              ...tx.postings.map((p) => ({
                account: p.account,
                status: p.status,
                amount: p.amount.exact,
              })),
              {
                account: source,
                amount: {
                  ...missing,
                  mantissa: (-BigInt(missing.mantissa)).toString(),
                },
              },
              { account: card.reserve, amount: missing },
            ],
          },
        };
      });
    if (!ops.length) throw new Error(t("No unreserved transactions"));
    return { ...base, path: "/batch", body: { head: data.version.head, ops } };
  }
  if (kind !== "reconcile") amount(fields.amount);
  if (kind === "split") {
    amount(fields.their_share);
    return {
      ...base,
      path: "/splits",
      body: {
        date: fields.date,
        description: fields.description,
        paid_with: fields.paid_with,
        category: fields.category,
        person: fields.person,
        total: fields.amount,
        their_share: fields.their_share,
      },
    };
  }
  if (kind === "settle") {
    const person = fields.person.trim();
    return {
      ...base,
      path: "/settlements",
      body: {
        date: fields.date,
        description: t("Settled up with %1", person.charAt(0).toUpperCase() + person.slice(1)),
        account: fields.account,
        person,
        amount: fields.amount,
        direction: fields.direction,
      },
    };
  }
  if (kind === "purchase")
    return {
      ...base,
      path: "/cards/purchases",
      body: {
        card: fields.card,
        expense_account: fields.category,
        source: fields.source,
        amount: fields.amount,
        date: fields.date,
        description: fields.description,
      },
    };
  if (kind === "payment")
    return {
      ...base,
      path: "/cards/payments",
      body: { card: fields.card, amount: fields.amount, date: fields.date },
    };
  if (kind === "reconcile")
    return {
      ...base,
      path: "/reconcile",
      body: {
        head: data.version.head,
        account: fields.account,
        date: fields.date,
        balance: exactAmount(fields.amount, fields.currency || accountCurrency(data, fields.account)),
        mode: "assert",
      },
    };
  throw new Error(t("The server had a problem."));
}
