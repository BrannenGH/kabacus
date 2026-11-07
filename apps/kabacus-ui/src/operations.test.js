// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test } from "vitest";
import { operation } from "./operations";
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
