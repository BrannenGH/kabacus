// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { exactAmount } from "./api";
import { t } from "./i18n";
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
  if (kind === "postings")
    return {
      ...base,
      path: "/transactions",
      body: {
        date: fields.date,
        description: fields.description,
        postings: Object.keys(fields)
          .filter((key) => /^account[0-9]+$/.test(key))
          .map((key) => ({
            account: fields[key],
            amount: amount(fields["amount" + key.slice(7)]),
          })),
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
    if (fields.account0 !== undefined) {
      return {
        method: "PATCH",
        path: `/transactions/${tx.id}`,
        body: {
          date: fields.date,
          description: fields.description,
          tags: tx.tags || [],
          postings: Object.keys(fields).filter(key => /^account[0-9]+$/.test(key)).map(key => {
            const index = Number(key.slice(7));
            const old = tx.postings[index];
            const account = fields[key];
            return {
              account,
              status: old?.account === account ? old.status || "" : "",
              amount: exactAmount(fields["amount" + index], old?.amount.commodity || data.commodity.symbol),
            };
          }),
        },
      };
    }
    const positive = amount(fields.amount);
    return {
      method: "PATCH",
      path: `/transactions/${tx.id}`,
      body: {
        date: fields.date,
        description: fields.description,
        tags: tx.tags || [],
        postings: tx.postings.map((p) => {
          const account = p.amount.exact.startsWith("-")
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
  amount(fields.amount);
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
        balance: fields.amount,
        mode: "assert",
      },
    };
  throw new Error(t("The server had a problem."));
}
