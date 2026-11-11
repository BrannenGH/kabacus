// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test } from "vitest";
import { operation, cardPurchase } from "./operations";
const data = {
  commodity: { symbol: "USD", places: 2 },
  version: { head: "42" },
};
test("account type changes use the conversion API in either direction", () => {
  for (const synthetic of ["true", "false"]) {
    expect(operation("account-type", {
      account: "assets:bank:checking:travel", synthetic,
    }, data)).toEqual({
      method: "PUT",
      path: "/accounts/assets%3Abank%3Achecking%3Atravel/type",
      body: { synthetic: synthetic === "true" },
    });
  }
});
test("card workflows delegate reserves to the server with decimal strings", () => {
  expect(
    operation(
      "purchase",
      {
        card: "liabilities:credit-card:visa",
        category: "expenses:groceries",
        source: "free",
        amount: "84.17",
        date: "2026-10-25",
        description: "Cub Foods",
      },
      data,
    ),
  ).toMatchObject({
    path: "/cards/purchases",
    body: {
      amount: "84.17",
      expense_account: "expenses:groceries",
      source: "free",
    },
  });
});
test("editing preserves tags and each posting status while keeping the transaction number", () => {
  const tx = {
    id: 12,
    tags: [["trip", "Oslo"]],
    postings: [
      {
        account: "assets:bank:checking",
        status: "*",
        amount: { exact: "-84.17", commodity: "USD" },
      },
      {
        account: "expenses:groceries",
        status: "!",
        amount: { exact: "84.17", commodity: "USD" },
      },
    ],
  };
  const result = operation(
    "edit",
    { date: "2026-10-25", description: "Cub Foods", amount: "85.19" },
    data,
    tx,
  );
  expect(result).toMatchObject({
    method: "PATCH",
    path: "/transactions/12",
    body: {
      tags: tx.tags,
      postings: [
        {
          account: "assets:bank:checking",
          status: "*",
          amount: { mantissa: "-8519" },
        },
        {
          account: "expenses:groceries",
          status: "!",
          amount: { mantissa: "8519" },
        },
      ],
    },
  });
});
test("void adds a correction instead of deleting or rewriting", () => {
  expect(operation("void", {}, data, { id: 12 })).toEqual({
    method: "POST",
    path: "/transactions/12/corrections",
    body: { void: true },
  });
});

test("reserve repairs use server-computed shortfalls in one batch and preserve the transaction", () => {
  const data = {
    version: { head: "42" },
    commodity: { symbol: "USD", places: 2 },
    accounts: [
      {
        account: "assets:bank:checking:card-visa",
        home: "assets:bank:checking",
      },
    ],
    cash: {
      cards: [
        {
          card: "liabilities:credit-card:visa",
          reserve: "assets:bank:checking:card-visa",
        },
      ],
    },
    transactions: [
      {
        id: 12,
        date: "2026-10-25",
        payee: "Cub Foods",
        memo: "Groceries",
        tags: [],
        postings: [],
        missing_reserve: [
          { card: "liabilities:credit-card:visa", amount: { exact: "84.17" } },
        ],
      },
    ],
  };
  const result = operation(
    "fix",
    { card: "liabilities:credit-card:visa", source: "free" },
    data,
  );
  expect(result.path).toBe("/batch");
  expect(result.body.head).toBe("42");
  expect(result.body.ops).toHaveLength(1);
  expect(result.body.ops[0].args.postings).toEqual([
    {
      account: "assets:bank:checking",
      amount: { commodity: "USD", mantissa: "-8417", places: 2 },
    },
    {
      account: "assets:bank:checking:card-visa",
      amount: { commodity: "USD", mantissa: "8417", places: 2 },
    },
  ]);
});

test("split edits preserve repeated accounts' individual marks and exact amounts", () => {
  const tx = { id: 18, tags: [["trip", "Bergen"]], postings: [
    { account: "expenses:groceries", status: "!", amount: { exact: "30.00", commodity: "USD" } },
    { account: "expenses:groceries", status: "*", amount: { exact: "10.00", commodity: "USD" } },
    { account: "assets:bank:checking", status: "*", amount: { exact: "-40.00", commodity: "USD" } },
  ] };
  const fields = { date: "2026-10-25", description: "Maple Market",
    account0: "expenses:groceries", amount0: "25.00",
    account1: "expenses:groceries", amount1: "15.00",
    account2: "assets:bank:checking", amount2: "-40.00" };
  expect(operation("edit", fields, data, tx)).toMatchObject({ method: "PATCH", path: "/transactions/18", body: {
    tags: tx.tags, postings: [
      { account: fields.account0, status: "!", amount: { mantissa: "2500" } },
      { account: fields.account1, status: "*", amount: { mantissa: "1500" } },
      { account: fields.account2, status: "*", amount: { mantissa: "-4000" } },
    ],
  } });
});


test("removing the first split posting and adding a new one retains row identities", () => {
  const tx = { id: 18, tags: [], postings: [
    { account: "expenses:groceries", status: "!", amount: { commodity: "USD" } },
    { account: "expenses:dining", status: "*", amount: { commodity: "USD" } },
    { account: "assets:bank:checking", status: "*", amount: { commodity: "USD" } },
  ] };
  const result = operation("edit", {
    date: "2026-10-25", description: "Harbor Cafe",
    account1: "expenses:dining", amount1: "15.00",
    account2: "assets:bank:checking", amount2: "-20.00",
    account3: "expenses:groceries", amount3: "5.00",
  }, data, tx);
  expect(result.body.postings.map(p => [p.account, p.status, p.amount.mantissa])).toEqual([
    ["expenses:dining", "*", "1500"], ["assets:bank:checking", "*", "-2000"],
    ["expenses:groceries", "", "500"],
  ]);
});

test("wallet postings retain each currency, conversion cost and added fee rows", () => {
  const tx = { id: 28, tags: [], postings: [
    { account: "assets:investments:ethereum", status: "*", amount: { exact: "0.4", commodity: "ETH" }, cost: { kind: "total", amount: { exact: "1000.00", commodity: "USD" } } },
    { account: "assets:bank:checking", status: "*", amount: { exact: "-1025.00", commodity: "USD" } },
    { account: "expenses:fees:platform", amount: { exact: "15.00", commodity: "USD" } },
    { account: "expenses:fees:broker", amount: { exact: "10.00", commodity: "USD" } },
  ] };
  const result = operation("edit", { date: "2026-11-02", description: "Cedar Exchange",
    account0: tx.postings[0].account, amount0: "0.400000000000000001", currency0: "ETH", cost0: "1000.00", cost_currency0: "USD", cost_kind0: "total",
    account1: tx.postings[1].account, amount1: "-1025.00", currency1: "USD",
    account2: tx.postings[2].account, amount2: "15.00", currency2: "USD",
    account3: tx.postings[3].account, amount3: "10.00", currency3: "USD",
    account4: "expenses:fees:broker", amount4: "0.000000000001", currency4: "XMR",
  }, data, tx);
  expect(result.body.postings[0]).toMatchObject({ amount: { commodity: "ETH", mantissa: "400000000000000001", places: 18 }, cost: {kind: "total", amount: {commodity: "USD", mantissa: "100000", places: 2}} });
  expect(result.body.postings[4].amount.commodity).toBe("XMR");
});
test("editing a card purchase rewrites its four lines from the card, category and source", () => {
  const money = exact => ({ commodity: "USD", exact });
  const line = (account, exact) => ({ account, amount: money(exact), status: "" });
  const wells = "liabilities:credit-card:wells-fargo-platinum";
  const reserve = "assets:bank:checking:card-wells-fargo-platinum";
  const books = {
    ...data,
    accounts: [{ account: reserve, home: "assets:bank:checking" }],
    cash: { cards: [{ card: wells, reserve }] },
  };
  const tx = { id: 7, tags: [], postings: [
    line("expenses:household:groceries", "19.49"), line(wells, "-19.49"),
    line("assets:bank:checking", "-19.49"), line(reserve, "19.49"),
  ] };
  expect(cardPurchase(books, tx)).toEqual({ card: wells, category: "expenses:household:groceries", source: "free" });
  const { body } = operation("edit", { date: "2026-11-10", description: "Aldi", amount: "21.00",
    card: wells, category: "expenses:household:cleaning", source: "assets:bank:checking:travel" }, books, tx);
  expect(body.postings.map(p => [p.account, p.amount.mantissa])).toEqual([
    ["expenses:household:cleaning", "2100"], [wells, "-2100"],
    ["assets:bank:checking:travel", "-2100"], [reserve, "2100"],
  ]);
});
