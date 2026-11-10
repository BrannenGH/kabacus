// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test } from "vitest";
import { browse, search } from "./account-tree";

const options = [
  { path: "expenses:food:groceries", label: "Groceries" },
  { path: "expenses:food:fast-food", label: "Fast food and takeout" },
  { path: "expenses:food:restaurants:date-night", label: "Date night" },
  { path: "expenses:housing:rent", label: "Rent" },
  { path: "income:salary", label: "Salary" },
];

test("browse lists groups and the accounts directly in a group", () => {
  expect(browse(options, []).map(r => r.label)).toEqual(["Expenses", "Income"]);
  expect(browse(options, ["expenses", "food"]).map(r => r.option?.label ?? `${r.label} ›`))
    .toEqual(["Groceries", "Fast food and takeout", "Restaurants ›"]);
});

test("browse keeps an account that is also a group", () => {
  const rows = browse([{ path: "assets:bank:checking", label: "Checking" },
    { path: "assets:bank:checking:travel", label: "Travel" }], ["assets", "bank"]);
  expect(rows.map(r => r.option?.label ?? r.label)).toEqual(["Checking", "Checking"]);
});

test("search matches names and paths across groups and shows where each lives", () => {
  expect(search(options, "food").map(r => r.option.label)).toEqual(["Groceries", "Fast food and takeout", "Date night"]);
  expect(search(options, "SALARY")).toEqual([{ option: options[4], path: "Income" }]);
  expect(search(options, "fast")[0].path).toBe("Expenses › Food");
});
