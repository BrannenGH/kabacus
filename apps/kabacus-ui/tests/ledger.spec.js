// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test } from "@playwright/test";
const amount = (exact) => ({
  commodity: "USD",
  exact,
  formatted: `${exact.startsWith("-") ? "-" : ""}$${exact.replace("-", "")}`,
});
const version = {
  server_version: "0.2.12",
  commit: "7a9fce322640ecba4268aa2edf2c8013e8f33329",
  source_url: "https://github.com/BrannenGH/kabacus",
  head: "42",
  books: "42",
  auth: { mode: "none" },
};
const accounts = [
  { account: "assets:bank:checking", display_name: "Checking", kind: "money", synthetic: false },
  {
    account: "assets:bank:checking:travel",
    display_name: "Travel",
    kind: "pre-allocation",
    synthetic: true,
    home: "assets:bank:checking",
  },
  { account: "expenses:groceries", display_name: "Groceries", kind: "expense" },
];
accounts.push(
  { account: "income:salary", display_name: "Salary", kind: "income" },
  {
    account: "assets:receivable:jordan",
    display_name: "Jordan Rivera",
    kind: "receivable",
  },
  {
    account: "assets:bank:checking:card-everyday",
    display_name: "Everyday card reserve",
    kind: "debt-reserve",
    home: "assets:bank:checking",
  },
);
const transaction = {
  id: 12,
  date: "2026-10-23",
  payee: "Maple Market",
  memo: "Weekly groceries",
  status: "*",
  postings: [
    { account: "expenses:groceries", amount: amount("84.25") },
    { account: "assets:bank:checking", amount: amount("-84.25") },
  ],
};
const cash = {
  cash_on_hand: amount("4520.75"),
  money_on_hand: amount("6200.75"),
  future_allocations: amount("1800.00"),
  receivables: amount("120.00"),
  homes: [
    {
      account: accounts[0].account,
      display_name: "Checking",
      inclusive: amount("6200.75"),
      free: amount("3900.75"),
      funds: amount("1800.00"),
      reserves: amount("500.00"),
    },
  ],
  funds: [
    {
      account: accounts[1].account,
      display_name: "Travel",
      home: accounts[0].account,
      balance: amount("1800.00"),
    },
  ],
  cards: [
    {
      card: "liabilities:card:everyday",
      display_name: "Everyday card",
      owed: amount("500.00"),
      reserve_balance: amount("500.00"),
      difference: amount("0.00"),
      unreserved_count: 0,
    },
  ],
  unreserved_count: 0,
  warnings: [],
};
async function setup(page, conflict = false, shownTransaction = transaction, suppliedAccounts = accounts, suppliedCommodity = { symbol: "USD", places: 2 }) {
  const writes = [];
  const currentAccounts = structuredClone(suppliedAccounts);
  const currentVersion = { ...version };
  await page.route("**/*", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (
      path.startsWith("/app/") ||
      path.startsWith("/node_modules/") ||
      path.startsWith("/@") ||
      path.startsWith("/src/")
    )
      return route.continue();
    if (["POST", "PATCH", "PUT"].includes(route.request().method())) {
      const body = route.request().postDataJSON();
      writes.push({ body, headers: route.request().headers(), path });
      if (path === "/reconcile" && body.mode === "check")
        return route.fulfill({
          json: {
            status: "matched",
            ledger_balance: "6200.75",
            date: "2026-10-25",
          },
        });
      if (conflict && !body.dry_run)
        return route.fulfill({ status: 409, json: { message: "Stale head" } });
      if (path.endsWith("/type")) {
        const account = decodeURIComponent(path.slice("/accounts/".length, -"/type".length));
        if (!body.dry_run) {
          currentAccounts.find((a) => a.account === account).synthetic = body.synthetic;
          currentVersion.head = currentVersion.books = String(BigInt(currentVersion.head) + 1n);
        }
        return route.fulfill({ json: {
          head: currentVersion.head,
          details: { account, synthetic: body.synthetic },
          warnings: [], balances: [],
        } });
      }
      return route.fulfill({
        json: {
          head: body.dry_run ? "42" : "43",
          preview:
            "2026-10-25 Maple Market\n    expenses:groceries  $84.25\n    assets:bank:checking  -$84.25",
          warnings: [],
          balances: [],
        },
      });
    }
    const fixtures = {
      "/commands": {add: [
        ["transaction.spent","Spent"],["transaction.received","Received"],["card.purchase","Card purchase"],["transaction.transfer","Transfer"],["reserve.setAside","Set aside"],["card.pay","Pay a card"],["split.record","Split with someone"],["transaction.postings","Postings"]
      ].map(([id,label],i)=>({id,label,key:String(i+1)}))},
      "/version": currentVersion,
      "/commodity": suppliedCommodity,
      "/cash-on-hand": cash,
      "/reports/cash-on-hand-history": {
        commodity: "USD",
        points: [
          { date: "2026-09-30", cash_on_hand: "4423.17" },
          { date: "2026-10-25", cash_on_hand: "4520.75" },
        ],
      },
      "/accounts": currentAccounts,
      "/transactions": [shownTransaction],
      "/counterparties": [
        { name: "jordan", net: amount("42.08"), label: "Owes you" },
      ],
      "/counterparties/jordan/statement": {
        name: "Jordan Rivera",
        opening_balance: amount("0.00"),
        closing_balance: amount("42.08"),
        lines: [
          {
            date: "2026-10-25",
            description: "Maple Market shared groceries",
            net_amount: amount("42.08"),
            running_balance: amount("42.08"),
            open: true,
          },
        ],
      },
      "/payees": [{ payee: transaction.payee, count: 1, last: transaction }],
      "/history": [
        {
          hash: "42",
          revision: 42,
          summary: "Recorded Maple Market",
          user: "Morgan Reed",
          date: "2026-10-23T12:00:00Z",
          kind: "add",
        },
      ],
      "/register/assets%3Abank%3Achecking": [
        {
          id: 12,
          date: transaction.date,
          payee: transaction.payee,
          memo: transaction.memo,
          other_account: "expenses:groceries",
          amount: amount("-84.25"),
          balance: amount("6200.75"),
        },
      ],
      "/register/assets%3Abank%3Achecking%3Atravel": [],
      "/register/assets%3Ainvestments%3Amonero": [],
    };
    if (fixtures[path]) return route.fulfill({ json: fixtures[path] });
    return route.continue();
  });
  await page.goto("/app/?connect=1");
  await expect(page.locator(".overview-layout")).toBeVisible();
  return writes;
}
async function fillTransaction(page) {
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await page.getByRole("button", { name: /^Spent/ }).click();
  await page.getByLabel("Payee").fill("Maple Market");
  await page.getByLabel("From").selectOption(accounts[0].account);
  await page.getByRole("dialog").getByLabel("To", { exact: true }).fill("expenses:groceries");
  await page.getByLabel("Amount (USD)").fill("84.25");
  await page.getByRole("button", { name: "Preview" }).click();
  await expect(
    page.getByRole("heading", { name: "What changes", exact: true }),
  ).toBeVisible();
}
for (const target of [accounts[0], accounts[1]]) {
  test(`account register converts ${target.display_name} after preview`, async ({ page }) => {
    const writes = await setup(page);
    await page.getByRole("navigation").getByRole("button", { name: "Transactions", exact: true }).click();
    await page.getByRole("combobox", { name: "Account", exact: true }).selectOption(target.account);
    for (const synthetic of [!target.synthetic, target.synthetic]) {
      const value = String(synthetic);
      const label = synthetic ? "Synthetic account" : "Real account";
      await page.getByRole("button", { name: "Account type", exact: true }).click();
      const dialog = page.getByRole("dialog");
      await dialog.getByRole("combobox", { name: "Account type", exact: true }).selectOption(value);
      await expect(dialog.getByText("Review before saving", { exact: true })).toBeVisible();
      expect(writes.at(-1).body).toEqual({ synthetic: value === "true", dry_run: true });
      const previewHead = writes.at(-1).headers["if-match"];
      await dialog.getByRole("button", { name: "Save", exact: true }).click();
      await expect(dialog).not.toBeVisible();
      await expect(page.getByText(label, { exact: true })).toBeVisible();
      expect(writes.at(-1).path).toBe(`/accounts/${encodeURIComponent(target.account)}/type`);
      expect(writes.at(-1).body.dry_run).toBe(false);
      expect(writes.at(-1).headers["if-match"]).toBe(previewHead);
    }
  });
}

test("account type conflicts keep the edit open without changing the displayed type", async ({ page }) => {
  const writes = await setup(page, true);
  await page.getByRole("navigation").getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("combobox", { name: "Account", exact: true }).selectOption(accounts[0].account);
  await page.getByRole("button", { name: "Account type", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("combobox", { name: "Account type", exact: true }).selectOption("true");
  await expect(dialog.getByText("Review before saving", { exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog.getByRole("alert")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Preview", exact: true })).toBeDisabled();
  await expect(page.locator(".filters").getByText("Real account", { exact: true })).toBeVisible();
  expect(writes.at(-1).body.dry_run).toBe(false);
});

test("desktop navigation and account register", async ({ page }) => {
  await setup(page);
  await page.screenshot({ path: "test-results/overview.png", fullPage: true });
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await page.getByLabel("Search", { exact: true }).fill("Maple");
  await expect(page.getByText("Maple Market", { exact: true })).toBeVisible();
  await page
    .getByRole("combobox", { name: "Account", exact: true })
    .selectOption(accounts[0].account);
  await expect(
    page.getByRole("cell", { name: "$6200.75", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Reserves", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Travel" })).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Credit", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Everyday card" }),
  ).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Activity", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Recorded Maple Market" }),
  ).toBeVisible();
});
test("requires a preview before saving exact balanced postings", async ({
  page,
}) => {
  const writes = await setup(page);
  await fillTransaction(page);
  expect(writes).toHaveLength(1);
  expect(writes[0].body.dry_run).toBe(true);
  expect(writes[0].body.postings.map((p) => p.amount.mantissa)).toEqual([
    "8425",
    "-8425",
  ]);
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  expect(writes).toHaveLength(2);
  expect(writes[1].headers["if-match"]).toBe('"42"');
  expect(writes[1].body.dry_run).toBe(false);
});
test("stale save requires refreshing and a new preview", async ({ page }) => {
  const writes = await setup(page, true);
  await fillTransaction(page);
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText("books changed");
  await expect(page.getByRole("button", { name: "Preview", exact:true })).toBeDisabled();
  expect(writes).toHaveLength(2);
});
test("allocation creates a fund only after confirmation", async ({ page }) => {
  const writes = await setup(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Reserves", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Set money aside", exact: true })
    .click();
  await page.getByLabel("From").selectOption(accounts[0].account);
  await page.getByLabel("To").fill("assets:bank:checking:repairs");
  await page.getByLabel("Amount (USD)").fill("100.00");
  await page.getByLabel("Name the new reserve").check();
  await page.getByRole("button", { name: "Preview" }).click();
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  expect(writes[1]).toMatchObject({
    path: "/allocations",
    body: { amount: "100.00", new_fund: true, dry_run: false },
  });
});

for (const [locale, direction] of [
  ["de-DE", "ltr"],
  ["ar", "rtl"],
]) {
  test(`loads the existing ${locale} catalog and ${direction} layout`, async ({
    browser,
  }) => {
    const context = await browser.newContext({
      locale,
      viewport: { width: 1440, height: 1000 },
    });
    const page = await context.newPage();
    await setup(page);
    const language = locale.split("-")[0];
    const response = await page.request.get(
      `/app/assets/i18n/${language}.json`,
    );
    const catalog = await response.json();
    expect(catalog.Overview).not.toBe("Overview");
    await expect(
      page.getByRole("heading", { name: catalog.Overview, exact: true }),
    ).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("dir", direction);
    await page.screenshot({
      path: `test-results/overview-${language}.png`,
      fullPage: true,
    });
    await context.close();
  });
}

test("reports preserve exact strings and switch periods and loan-principal options", async ({
  page,
}) => {
  await setup(page);
  const requests = [];
  await page.route("**/reports/**", (route) => {
    const url = new URL(route.request().url());
    requests.push(url);
    const fixture = url.pathname.endsWith("cash-flow")
      ? {
          commodity: "USD",
          opening_cash: "9007199254740993.27",
          closing_cash: "9007199254741003.27",
          lines: [
            {
              group: "income",
              display_name: "Salary",
              cash: "10.00",
              non_cash: "0.00",
            },
          ],
        }
      : url.pathname.endsWith("categories")
        ? {
            commodity: "USD",
            rows: [
              {
                account: "expenses:groceries",
                display_name: "Groceries",
                amount: "84.25",
                change: "-5.00",
              },
            ],
            previous_period: { label: "2026-09" },
          }
        : {
            commodity: "USD",
            income: [
              {
                account: "income:salary",
                display_name: "Salary",
                total: "5000.00",
              },
            ],
            expenses: [],
            totals: { income: ["5000.00"], expenses: ["0.00"] },
            net: ["5000.00"],
          };
    return route.fulfill({ json: fixture });
  });
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Reports", exact: true })
    .click();
  await expect(
    page.getByText("USD 9007199254740993.27", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("combobox", { name: "Reports", exact: true })
    .selectOption("categories");
  await expect(
    page.getByRole("cell", { name: "USD 84.25", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("combobox", { name: "Reports", exact: true })
    .selectOption("income-statement");
  await page.getByLabel("Count loan principal as money spent").uncheck();
  await page.getByLabel("Date", { exact: true }).fill("2026-02");
  await expect
    .poll(() => requests.at(-1)?.searchParams.get("to"))
    .toBe("2026-02-28");
  expect(requests.at(-1).searchParams.get("equity_conversion")).toBe("false");
  await expect(page.getByRole("cell", { name: "Salary" })).toBeVisible();
});

for (const action of ["edit", "void", "purchase", "payment"]) {
  test(`desktop ${action} previews then writes against the same head`, async ({
    page,
  }) => {
    const writes = await setup(page);
    if (["edit", "void"].includes(action)) {
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Transactions", exact: true })
        .click();
      await page
        .getByRole("button", {
          name: action === "edit" ? "Edit" : "Void",
          exact: true,
        })
        .click();
      if (action === "edit")
        await page
          .getByRole("dialog")
          .getByLabel("Amount", { exact: true })
          .fill("85.19");
    } else {
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Credit", exact: true })
        .click();
      await page
        .getByRole("button", {
          name: action === "purchase" ? "Record purchase" : "Pay card",
          exact: true,
        })
        .click();
      const dialog = page.getByRole("dialog");
      await dialog
        .getByRole("combobox", { name: "Card", exact: true })
        .selectOption("liabilities:card:everyday");
      await dialog.getByLabel("Amount", { exact: true }).fill("84.17");
      if (action === "purchase")
        await dialog.getByLabel("Payee", { exact: true }).fill("Maple Market");
    }
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Preview", exact: true })
      .click();
    await expect(
      page
        .getByRole("dialog")
        .getByRole("heading", { name: "Review before saving" }),
    ).toBeVisible();
    expect(writes).toHaveLength(1);
    expect(writes[0].body.dry_run).toBe(true);
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Save", exact: true })
      .click();
    await expect(page.getByRole("dialog")).not.toBeVisible();
    expect(writes).toHaveLength(2);
    expect(writes[1].headers["if-match"]).toBe('"42"');
    expect(writes[1].body.dry_run).toBe(false);
    if (action === "void") expect(writes[1].body.void).toBe(true);
    if (action === "edit")
      expect(writes[1].body.postings[0].amount.mantissa).toBe("8519");
  });
}

test("reconciliation is opened from a register and freezes the reviewed head", async ({
  page,
}) => {
  const writes = await setup(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await page
    .getByRole("combobox", { name: "Account", exact: true })
    .selectOption(accounts[0].account);
  await page.getByRole("button", { name: "Reconcile…", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByLabel("Amount", { exact: true })
    .fill("6200.75");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Preview", exact: true })
    .click();
  await expect(
    page
      .getByRole("dialog")
      .getByRole("heading", { name: "Review before saving" }),
  ).toBeVisible();
  expect(writes[1]).toMatchObject({
    path: "/reconcile",
    body: { head: "42", balance: {commodity:"USD",mantissa:"620075",places:2}, mode: "assert", dry_run: true },
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Save", exact: true })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  expect(writes.at(-1).body.head).toBe("42");
  expect(writes.at(-1).body.dry_run).toBe(false);
});
test("a lost connection keeps balances readable and prevents writes", async ({
  page,
}) => {
  await setup(page);
  await page.route("**/version", (route) =>
    route.fulfill({ status: 503, json: { message: "Server unavailable" } }),
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.locator(".overview-layout")).toContainText("$4520.75");
  await expect(
    page.getByRole("button", { name: "Add", exact: true }),
  ).toBeDisabled();
});

test("people statements use dates and open-item filters and export authenticated files", async ({
  page,
}) => {
  await setup(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "People", exact: true })
    .click();
  await page.getByRole("button", { name: "jordan", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Jordan Rivera" }),
  ).toBeVisible();
  await expect(
    page.getByRole("cell", { name: "Maple Market shared groceries" }),
  ).toBeVisible();
  await page.getByLabel("Open items only").check();
  await page.route("**/counterparties/jordan/statement?**", (route) => {
    const url = new URL(route.request().url());
    if (url.searchParams.get("format") === "csv")
      return route.fulfill({
        contentType: "text/csv",
        body: "Date,Description,Amount\n2026-10-25,Maple Market shared groceries,42.08\n",
      });
    return route.fallback();
  });
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "CSV", exact: true }).click();
  expect((await download).suggestedFilename()).toBe("statement.csv");
});
test("shared-expense form delegates exact total and share to the server", async ({
  page,
}) => {
  const writes = await setup(page);
  await page.getByRole("button",{name:"Add",exact:true}).click();
  await page.getByRole("button",{name:/^Split with someone/}).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Payee", { exact: true }).fill("Maple Market");
  await dialog.getByLabel("Amount", { exact: true }).fill("84.17");
  await dialog.getByLabel("Their share", { exact: true }).fill("42.08");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    dialog.getByRole("heading", { name: "Review before saving" }),
  ).toBeVisible();
  expect(writes[0]).toMatchObject({
    path: "/splits",
    body: {
      total: "84.17",
      their_share: "42.08",
      person: "jordan",
      dry_run: true,
    },
  });
});

test("returning from preview keeps a changed transaction draft", async ({
  page,
}) => {
  await setup(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Amount", { exact: true }).fill("85.19");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    dialog.getByRole("heading", { name: "Review before saving" }),
  ).toBeVisible();
  await dialog
    .getByRole("button", { name: "Back to edit", exact: true })
    .click();
  await expect(dialog.getByLabel("Amount", { exact: true })).toHaveValue(
    "85.19",
  );
});

test("known payees recall account choices, exact amount, and memo", async ({
  page,
}) => {
  await setup(page);
  const memory = page.waitForResponse("**/payees");
  await page
    .getByRole("button", { name: "Add", exact: true })
    .click();
  await page.getByRole("button", { name: /^Spent/ }).click();
  await memory;
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Payee", { exact: true }).fill("Maple Market");
  await expect(
    dialog.getByRole("combobox", { name: "From", exact: true }),
  ).toHaveValue("assets:bank:checking");
  await expect(
    dialog.getByRole("combobox", { name: "To", exact: true }),
  ).toHaveValue("expenses:groceries");
  await expect(dialog.getByLabel("Amount (USD)", { exact: true })).toHaveValue(
    "84.25",
  );
  await expect(dialog.getByLabel("Memo", { exact: true })).toHaveValue(
    "Weekly groceries",
  );
});

test("a reconciliation mismatch only checks and never offers an assertion save", async ({
  page,
}) => {
  await setup(page);
  const checks = [];
  await page.route("**/reconcile", (route) => {
    checks.push(route.request().postDataJSON());
    return route.fulfill({
      json: {
        status: "mismatch",
        ledger_balance: "6200.75",
        statement_balance: "6190.62",
        difference: "10.13",
        date: "2026-10-25",
        suggestions: [],
        later_transactions: [],
      },
    });
  });
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await page
    .getByRole("combobox", { name: "Account", exact: true })
    .selectOption(accounts[0].account);
  await page.getByRole("button", { name: "Reconcile…", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Amount", { exact: true }).fill("6190.62");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("10.13");
  await expect(
    dialog.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  expect(checks.every((body) => body.mode === "check" && body.dry_run)).toBe(
    true,
  );
});

test("desktop live previews preserve typing and discard an older response", async ({
  page,
}) => {
  await setup(page);
  const previews = [];
  await page.route("**/transactions", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    const body = route.request().postDataJSON();
    previews.push(body);
    if (previews.length === 1)
      await new Promise((resolve) => setTimeout(resolve, 1200));
    return route.fulfill({
      json: {
        head: "42",
        preview: body.postings[0].amount.mantissa,
        balances: [],
        warnings: [],
      },
    });
  });
  await page
    .getByRole("button", { name: "Add", exact: true })
    .click();
  await page.getByRole("button", { name: /^Spent/ }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Payee", { exact: true }).fill("Cedar Hardware");
  await dialog.getByLabel("From").selectOption(accounts[0].account);
  await dialog.getByLabel("To").fill("expenses:groceries");
  await dialog.getByLabel("Amount (USD)").fill("84.17");
  await expect.poll(() => previews.length).toBe(1);
  await expect(dialog.getByLabel("Amount (USD)")).toBeEnabled();
  await dialog.getByLabel("Amount (USD)").fill("85.19");
  await expect(dialog.locator(".preview pre")).toHaveText("8519");
  expect(previews.every((body) => body.dry_run)).toBe(true);
  await expect(dialog.getByLabel("Amount (USD)")).toHaveValue("85.19");
});

test("native connection settings stay usable after a connection failure", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__KABACUS_LAUNCHER__ = true;
    window.__TAURI_INTERNALS__ = {
      invoke: async () => {
        throw new Error("Synthetic server unavailable");
      },
    };
  });
  await page.goto("/app/?connect=1");
  await page.getByRole("button", { name: "Connect to a Server…", exact: true }).click();
  await page.getByLabel("Server address").fill("https://books.example.org");
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Synthetic server unavailable",
  );
  await expect(page.getByLabel("Server address")).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Connect", exact: true }),
  ).toBeEnabled();
});

test("the signed-out screen is centered on a phone", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route("**/version", (route) =>
    route.fulfill({
      json: {
        ...version,
        auth: {
          mode: "oidc",
          issuer: "https://login.example.org/realms/household",
          app_client_id: "kabacus-app-7f3a",
        },
      },
    }),
  );
  await page.goto("/app/?connect=1");
  const button = page.getByRole("button", { name: "Sign in", exact: true });
  await expect(button).toBeVisible();
  const heading = await page.getByRole("heading", { name: /^Sign in to / }).boundingBox();
  const action = await button.boundingBox();
  const middle = (box) => box.x + box.width / 2;
  expect(Math.abs(middle(heading) - 195)).toBeLessThan(2);
  expect(Math.abs(middle(action) - 195)).toBeLessThan(2);
  const top = heading.y;
  const bottom = action.y + action.height;
  expect(Math.abs((top + bottom) / 2 - 422)).toBeLessThan(80);
});

test("posting review status is previewed before writing", async ({ page }) => {
  const writes = await setup(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await page.getByRole("button", { name: "Review", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByRole("combobox", { name: "Account", exact: true })
    .selectOption(accounts[0].account);
  await dialog
    .getByRole("combobox", { name: "Review", exact: true })
    .selectOption("cleared");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(dialog.locator(".preview")).toBeVisible();
  expect(writes.at(-1)).toMatchObject({
    path: "/transactions/12/status",
    body: { account: accounts[0].account, status: "cleared", dry_run: true },
  });
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).not.toBeVisible();
  expect(writes.at(-1).body.dry_run).toBe(false);
});

test("custom postings can preview an exact multi-line transaction", async ({
  page,
}) => {
  const writes = await setup(page);
  await page.getByRole("button",{name:"Add",exact:true}).click();
  await page.getByRole("button",{name:/^Postings/}).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Payee", { exact: true }).fill("Cedar Hardware");
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  const accounts = dialog.getByRole("combobox", {
    name: "Account",
    exact: true,
  });
  const amounts = dialog.getByRole("textbox", { name: "Amount", exact: true });
  await accounts.nth(0).fill("expenses:groceries");
  await amounts.nth(0).fill("42.09");
  await accounts.nth(1).fill("expenses:household");
  await amounts.nth(1).fill("42.08");
  await accounts.nth(2).fill("assets:bank:checking");
  await amounts.nth(2).fill("-84.17");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(dialog.locator(".preview")).toBeVisible();
  expect(writes.at(-1).body.postings.map((p) => p.amount.mantissa)).toEqual([
    "4209",
    "4208",
    "-8417",
  ]);
  expect(writes.at(-1).body.dry_run).toBe(true);
});

async function largeBalances(page) {
  const balance = {
    commodity: "USD",
    exact: "1234567890.25",
    formatted: "$1,234,567,890.25",
  };
  await page.route("**/cash-on-hand?**", (route) =>
    route.fulfill({
      json: {
        ...cash,
        cash_on_hand: balance,
        money_on_hand: balance,
        future_allocations: balance,
        receivables: balance,
      },
    }),
  );
  if (await page.locator(".phone-overflow").isVisible()) await page.locator(".phone-overflow summary").click();
  await page.locator('[data-action="refresh"]:visible').click();
  await expect(page.locator(".cash-total .amount").first()).toHaveText(
    balance.formatted,
  );
}
async function expectContained(locator) {
  const sizes = await locator.evaluate((node) => ({
    content: node.scrollWidth,
    available: node.clientWidth,
  }));
  expect(sizes.content).toBeLessThanOrEqual(sizes.available);
}

for (const locale of ["en-US", "de-DE", "ar"]) {
  test.describe(`phone reflow ${locale}`, () => {
    test.use({ locale, viewport: { width: 320, height: 740 } });

    test("accounts push a readable register grouped by day", async ({
      page,
    }) => {
      await setup(page);
      await page.locator('[data-destination="Register"]').click();
      await page.locator(".account-list button").filter({hasText:"Checking"}).first().click();
      await expect(page.locator(".phone-register")).toBeVisible();
      await expect(page.locator(".register-row").first()).toContainText("Maple Market");
      await expectContained(page.locator(".register-row").first());
      await expectContained(page.locator("html"));
    });

    test("large exact balances do not widen the page", async ({ page }) => {
      await setup(page);
      await largeBalances(page);
      await expectContained(page.locator("html"));
      for (const metric of await page.locator(".metric").all())
        await expectContained(metric);
    });

    test("floating Add choice stays within thumb reach", async ({ page }) => {
      await setup(page);
      const fab = await page.locator(".phone-fab").boundingBox();
      const tabs = await page.locator(".phone-tabs").boundingBox();
      expect(fab.width).toBeGreaterThanOrEqual(48);
      expect(fab.y + fab.height).toBeLessThan(tabs.y);
      await expectContained(page.locator("html"));
    });

    test("settings expose the version and corresponding source", async ({ page }) => {
      await setup(page);
      await page.locator('[data-destination="More"]').click();
      await page.locator(".settings-action").click();
      await expect(page.getByText(`Kabacus ${version.server_version}`, {exact:true})).toBeVisible();
      await expect(page.locator(`main a[href="${version.source_url}"]`)).toBeVisible();
      await expect(page.locator("main")).not.toContainText(version.commit);
    });
  });
}

test("desktop balances and connection details remain contained", async ({
  page,
}) => {
  await setup(page);
  await largeBalances(page);
  await expectContained(page.locator("html"));
  for (const metric of await page.locator(".metric").all())
    await expectContained(metric);
  await page.getByRole("button",{name:/^Settings/}).click();
  await expectContained(page.locator("main"));
  await expect(page.getByRole("link",{name:"Open the source code"})).toBeVisible();
});

for (const width of [390, 1440]) {
  test(`received money uses named income and money accounts at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const writes = await setup(page);
    await page.getByRole("button",{name:"Add",exact:true}).click();
    await page.getByRole("button",{name:/^Received/}).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Received", exact: true })).toBeVisible();
    await dialog.getByLabel("Payee", { exact: true }).fill("Cedar Cafe payroll");
    await dialog.locator('select[name="from"]').selectOption({ label: "Salary" });
    await dialog.locator('select[name="to"]').selectOption({ label: "Checking" });
    await dialog.getByLabel("Amount (USD)").fill("650.25");
    await dialog.getByRole("button", { name: width < 600 ? "Review" : "Preview", exact: true }).click();
    await expect(dialog.locator(".preview")).toBeVisible();
    expect(writes.at(-1).body).toMatchObject({ dry_run: true, postings: [
      { account: accounts[0].account, amount: { mantissa: "65025", places: 2 } },
      { account: "income:salary", amount: { mantissa: "-65025", places: 2 } },
    ] });
    await dialog.getByRole("button", { name: "Back to edit" }).click();
    await expect(dialog.locator('select[name="to"]')).toHaveValue(accounts[0].account);
    await expect(dialog.getByLabel("Amount (USD)")).toHaveValue("650.25");
  });
  test(`reserve display names resolve to the account at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const writes = await setup(page);
    await page.getByRole("button",{name:"Add",exact:true}).click();
  await page.getByRole("button",{name:/^Set aside/}).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("From").selectOption(accounts[0].account);
    await dialog.locator('input[name="to"]').fill("Travel");
    await dialog.getByLabel("Amount (USD)").fill("100.00");
    await dialog.getByRole("button", { name: width < 600 ? "Review" : "Preview", exact: true }).click();
    await expect(dialog.locator(".preview")).toBeVisible();
    expect(writes.at(-1).body).toMatchObject({ to: accounts[1].account, new_fund: false, dry_run: true });
  });
}

test("phone controls have readable input text and comfortable tap targets", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page);
  for (const button of await page.locator("nav button").all())
    expect((await button.boundingBox()).height).toBeGreaterThanOrEqual(48);
  await page.getByRole("button",{name:"Add",exact:true}).click();
  await page.getByRole("button",{name:/^Set aside/}).click();
  for (const field of await page.locator('dialog input:not([type="checkbox"]), dialog select').all()) {
    expect(await field.evaluate(e => parseFloat(getComputedStyle(e).fontSize))).toBeGreaterThanOrEqual(16);
    expect((await field.boundingBox()).height).toBeGreaterThanOrEqual(48);
  }
});

for (const width of [320, 1440]) {
  test(`card actions remain inside their panel at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await setup(page);
    await page.getByRole("navigation").getByRole("button", { name: "Credit", exact: true }).click();
    const heading = page.locator(".panel-heading").first();
    await expectContained(heading);
    const panel = await heading.boundingBox();
    for (const button of await heading.getByRole("button").all()) {
      const bounds = await button.boundingBox();
      expect(bounds.x).toBeGreaterThanOrEqual(panel.x);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(panel.x + panel.width);
    }
  });
}


test.describe("phone layout", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  test("shared tabs, More, Books and register Back", async ({ page }) => {
    await setup(page);
    const tabs = page.getByRole("navigation", { name: "Go" });
    await expect(tabs.getByRole("button")).toHaveText(["Overview", "Transactions", "Reserves", "Credit", "More"]);
    for (const tab of await tabs.getByRole("button").all()) {
      const box = await tab.boundingBox();
      expect(box.height).toBeGreaterThanOrEqual(48);
      expect(box.width).toBeGreaterThanOrEqual(48);
      expect(box.y).toBeGreaterThan(740);
    }
    await tabs.getByRole("button", { name: "More", exact: true }).click();
    for (const label of ["Loans", "People", "Reports", "Activity", "Settings", "Books"])
      await expect(page.getByRole("main").getByRole(label === "Books" ? "heading" : "button", { name: label, exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Switch Books…", exact: true }).click();
    const books = page.getByRole("dialog", { name: "Books", exact: true });
    for (const label of ["New Books…", "Open Books…", "Save a Copy As…", "Connect to a Server…"])
      await expect(books.getByRole("button", { name: label, exact: true })).toBeVisible();
    for (const label of ["New Window", "Close Window", "Quit"])
      await expect(books.getByRole("button", { name: label, exact: true })).toHaveCount(0);
    await books.getByRole("button", { name: "Close", exact: true }).click();
    await tabs.getByRole("button", { name: "Transactions", exact: true }).click();
    await page.getByRole("button", { name: /Checking/ }).first().click();
    await expect(page.locator(".phone-register")).toBeVisible();
    await page.goBack();
    await expect(page.locator(".account-list")).toBeVisible();
    await expect(page.locator(".phone-register")).toHaveCount(0);
  });
  test("browser Back returns from Review and protects an unsaved form", async ({ page }) => {
    await setup(page);
    await page.getByRole("button", { name: "Add", exact: true }).click();
    await page.getByRole("button", { name: /^Spent/ }).click();
    const form = page.locator(".bookkeeping-dialog");
    await form.getByLabel("Amount (USD)").fill("84.25");
    await form.getByLabel("Payee", { exact: true }).fill("Maple Market");
    await form.locator('select[name="from"]').selectOption(accounts[0].account);
    await form.locator('input[name="to"]').fill("Groceries");
    await form.getByRole("button", { name: "Review", exact: true }).click();
    await expect(form.locator(".preview")).toBeVisible();
    await page.goBack();
    await expect(form.getByLabel("Amount (USD)")).toHaveValue("84.25");
    page.once("dialog", dialog => dialog.dismiss());
    await page.goBack();
    await expect(form).toBeVisible();
    page.once("dialog", dialog => dialog.accept());
    await page.goBack();
    await expect(form).toHaveCount(0);
    await expect(page.locator(".overview-layout")).toBeVisible();
  });
  test("Void stays in the edit overflow and previews before writing", async ({ page }) => {
    const writes = await setup(page);
    await page.getByRole("navigation").getByRole("button", {name:"Transactions",exact:true}).click();
    await page.locator(".account-list button").filter({hasText:"Checking"}).first().click();
    await page.locator(".register-row").first().click();
    const form = page.getByRole("dialog");
    await form.locator(".operation-overflow summary").click();
    await form.getByRole("button", {name:"Void",exact:true}).click();
    await form.getByRole("button", {name:"Review",exact:true}).click();
    expect(writes.at(-1).path).toBe("/transactions/12/corrections");
    expect(writes.at(-1).body.void).toBe(true);
    expect(writes.at(-1).body.dry_run).toBe(true);
  });
  for (const [label, kind, field, path] of [
    ["Rainy day savings", "Spent", "from", "assets:bank:savings"],
    ["Rainy day savings", "Received", "to", "assets:bank:savings"],
    ["Rainy day savings", "Split with someone", "paid_with", "assets:bank:savings"],
    ["Everyday card", "Card purchase", "card", "liabilities:card:everyday"],
    ["Everyday card", "Pay a card", "card", "liabilities:card:everyday"],
  ]) test(`register preset: ${kind} from ${label}`, async ({ page }) => {
    await setup(page, false, transaction, [...accounts,
      {account:"assets:bank:savings",display_name:"Rainy day savings",kind:"money"},
      {account:"liabilities:card:everyday",display_name:"Everyday card",kind:"revolving-debt"},
    ]);
    await page.route("**/register/**", route => route.fulfill({json:[]}));
    await page.getByRole("navigation").getByRole("button", {name:"Transactions",exact:true}).click();
    await page.locator(".account-list").getByRole("button", {name:label,exact:true}).click();
    await page.getByRole("button", {name:"Add",exact:true}).click();
    await page.locator(".add-chooser button[data-choice]").filter({hasText:kind}).click();
    await expect(page.locator(`.bookkeeping-dialog select[name="${field}"]`)).toHaveValue(path);
  });
  test("touch opens full-screen account and date pickers", async ({ page }) => {
    await setup(page);
    await page.getByRole("button", { name: "Add", exact: true }).tap();
    await page.getByRole("button", { name: /^Received/ }).tap();
    const form = page.locator(".bookkeeping-dialog");
    await form.locator('select[name="from"]').tap();
    let picker = page.locator(".phone-picker");
    await expect(picker).toBeVisible();
    expect((await picker.boundingBox()).height).toBe(844);
    await picker.getByRole("button", { name: "Salary", exact: true }).tap();
    await expect(form.locator('select[name="from"]')).toHaveValue("income:salary");
    await form.locator('input[name="date"]').tap();
    await expect(picker).toBeVisible();
    await picker.getByRole("button", { name: "Back", exact: true }).tap();
    await expect(picker).toHaveCount(0);
  });
  test("Add sheet, Amount first, Review and Save above the tabs", async ({ page }) => {
    const writes = await setup(page);
    await page.getByRole("button", { name: "Add", exact: true }).click();
    const sheet = page.getByRole("dialog");
    await expect(sheet.locator("button[data-choice]")).toHaveCount(8);
    await sheet.getByRole("button", { name: /^Spent/ }).click();
    const form = page.getByRole("dialog");
    await expect(form.locator(".form-fields input").first()).toHaveAttribute("name", "amount");
    await form.getByLabel("Amount (USD)").fill("84.25");
    await form.getByLabel("Payee", { exact: true }).fill("Maple Market");
    await form.locator('select[name="from"]').selectOption(accounts[0].account);
    await form.locator('input[name="to"]').fill("Groceries");
    await form.getByRole("button", { name: "Review", exact: true }).click();
    expect(writes.at(-1).body.dry_run).toBe(true);
    await expect(form.getByText("What changes", { exact: true })).toBeVisible();
    await form.getByRole("button", { name: "Save", exact: true }).click();
    expect(writes.at(-1).body.dry_run).toBe(false);
    const toast = await page.locator(".save-toast").boundingBox();
    const tabs = await page.getByRole("navigation").boundingBox();
    expect(toast.y + toast.height).toBeLessThanOrEqual(tabs.y);
  });
});


test("shared breakpoint boundaries switch between tabs, rail and sidebar", async ({ page }) => {
  await setup(page);
  for (const [width, sidebar] of [[599, 0], [600, 80], [1199, 80], [1200, 240]]) {
    await page.setViewportSize({width, height: 844});
    if (!sidebar) {
      await expect(page.locator("aside")).toHaveCount(0);
      await expect(page.locator(".phone-tabs")).toBeVisible();
    } else {
      await expect(page.locator(".phone-tabs")).toHaveCount(0);
      await expect(page.locator("aside")).toBeVisible();
      expect((await page.locator("aside").boundingBox()).width).toBe(sidebar);
    }
  }
});

test("desktop split editing previews every posting and preserves the reviewed draft", async ({ page }) => {
  const split = { ...transaction, tags: [["trip", "Bergen"]], postings: [
    { account: "expenses:groceries", status: "!", amount: amount("60.00") },
    { account: "assets:receivable:jordan", status: "", amount: amount("24.25") },
    { account: "assets:bank:checking", status: "*", amount: amount("-84.25") },
  ] };
  const writes = await setup(page, false, split);
  await page.getByRole("navigation").getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const amounts = dialog.getByLabel("Amount", { exact: true });
  await expect(amounts).toHaveCount(3);
  await expect(amounts.nth(2)).toHaveValue("-84.25");
  await amounts.nth(0).fill("50.00");
  await amounts.nth(1).fill("34.25");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(dialog.getByRole("heading", { name: "Review before saving" })).toBeVisible();
  await dialog.getByRole("button", { name: "Back to edit", exact: true }).click();
  await expect(amounts.nth(0)).toHaveValue("50.00");
  await dialog.getByRole("button", { name: "Postings · 1", exact: true }).click();
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  await dialog.getByLabel("Account", { exact: true }).nth(2).fill("expenses:groceries");
  await amounts.nth(2).fill("50.00");
  await dialog.getByRole("button", { name: "Preview", exact: true }).click();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).not.toBeVisible();
  const saved = writes.at(-1);
  expect(saved.headers["if-match"]).toBe('"42"');
  expect(saved.body).toMatchObject({ dry_run: false, tags: split.tags, postings: [
    { account: split.postings[1].account, status: "", amount: { mantissa: "3425" } },
    { account: split.postings[2].account, status: "*", amount: { mantissa: "-8425" } },
    { account: split.postings[0].account, status: "", amount: { mantissa: "5000" } },
  ] });
});

test("remote books reopen in the PWA window after session storage is lost", async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page);
  await page.locator('[data-destination="Register"]').click();
  await expect(page.getByRole('heading', { name: 'Transactions', exact: true, level: 1 })).toBeVisible();
  const rememberedPage = () => page.evaluate(() => new Promise((resolve, reject) => {
    const request = indexedDB.open('kabacus-local-books', 1);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const query = db.transaction('state').objectStore('state').get('workspace');
      query.onsuccess = () => { resolve(query.result?.windows?.find(w => w.id === sessionStorage.getItem('kabacus.window'))?.page); db.close(); };
      query.onerror = () => { reject(query.error); db.close(); };
    };
  }));
  await expect.poll(rememberedPage).toBe('Register');
  await page.evaluate(() => sessionStorage.clear());
  await page.goto('/app/');
  await expect(page.getByRole('dialog', { name: 'Pick up where you left off' })).toBeVisible();
  let navigations = 0;
  page.on('request', request => { if (request.isNavigationRequest()) navigations++; });
  await page.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Transactions', exact: true, level: 1 })).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(navigations).toBe(0);
  expect(context.pages()).toHaveLength(1);
  await expect.poll(rememberedPage).toBe('Register');
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Transactions', exact: true, level: 1 })).toBeVisible();
});


test.describe("mobile exploration regressions", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("opening an account picker does not select the option under the finger", async ({ page }) => {
    await setup(page);
    await page.getByRole("button", { name: "Add", exact: true }).tap();
    await page.getByRole("button", { name: /^Spent/ }).tap();
    const form = page.locator(".bookkeeping-dialog");
    const from = form.locator('select[name="from"]');
    await from.tap();
    const picker = page.locator(".phone-picker");
    await expect(picker).toBeVisible();
    await expect(from).toHaveValue("");
    await picker.getByRole("button", { name: "Checking", exact: true }).tap();
    await expect(picker).toHaveCount(0);
    await expect(from).toHaveValue(accounts[0].account);
    await form.locator('input[name="to"]').tap();
    await expect(picker).toBeVisible();
    await page.goBack();
    await expect(picker).toHaveCount(0);
    await expect(form).toBeVisible();
    await expect(from).toHaveValue(accounts[0].account);
  });

  for (const sheet of ["Add", "Books"]) {
    test(`browser Back closes the ${sheet} sheet and choosing an action keeps the form open`, async ({ page }) => {
      await setup(page);
      const open = async () => sheet === "Add"
        ? page.getByRole("button", { name: "Add", exact: true }).tap()
        : page.locator(".phone-books-name").tap();
      await open();
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.goBack();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(page.locator(".overview-layout")).toBeVisible();
      await expect(page).toHaveURL(/\/app\/\?connect=1$/);
      await open();
      if (sheet === "Add") {
        await page.getByRole("button", { name: /^Received/ }).tap();
        await expect(page.locator(".bookkeeping-dialog")).toBeVisible();
        await page.getByLabel("Amount (USD)").fill("48.73");
        page.once("dialog", dialog => dialog.dismiss());
        await page.goBack();
        await expect(page.getByLabel("Amount (USD)")).toHaveValue("48.73");
        page.once("dialog", dialog => dialog.accept());
        await page.goBack();
      } else {
        await page.getByRole("button", { name: "Connect to a Server…", exact: true }).tap();
        await expect(page.getByRole("dialog", { name: "Connect to a Server…", exact: true })).toBeVisible();
        await page.goBack();
      }
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(page.locator(".overview-layout")).toBeVisible();
    });
  }

  for (const choice of ["Received", "Card purchase"]) {
    test(`rotation preserves the ${choice} draft and reviewed amount`, async ({ page }) => {
      const writes = await setup(page);
      await page.getByRole("button", { name: "Add", exact: true }).tap();
      await page.locator(".add-chooser button[data-choice]").filter({ hasText: choice }).tap();
      const form = page.locator(".bookkeeping-dialog");
      const amount = form.locator('input[name="amount"]');
      await amount.fill("48.73");
      await form.getByLabel("Payee", { exact: true }).fill("Cedar Cafe refund");
      await form.locator('input[name="date"]').fill("2026-10-25");
      if (choice === "Card purchase")
        await form.locator('select[name="card"]').selectOption("liabilities:card:everyday");
      for (const [width, height] of [[844, 390], [1280, 800], [390, 844]]) {
        await page.setViewportSize({ width, height });
        await expect(amount).toHaveValue("48.73");
        await expect(form.getByLabel("Payee", { exact: true })).toHaveValue("Cedar Cafe refund");
        await expect(form.locator('input[name="date"]')).toHaveValue("2026-10-25");
      }
      await form.getByRole("button", { name: "Review", exact: true }).click();
      await expect(form.locator(".preview")).toBeVisible();
      const body = writes.at(-1).body;
      expect(body.dry_run).toBe(true);
      if (choice === "Received") expect(body.postings[0].amount).toEqual({ commodity: "USD", mantissa: "4873", places: 2 });
      else expect(body.amount).toBe("48.73");
    });
  }
});

for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
  test.describe(`stakeholder review at ${viewport.width}px`, () => {
    test.use({ viewport });
    test("zero-balance reserves offer a next action", async ({ page }) => {
      await setup(page);
      await page.route("**/cash-on-hand?**", route => route.fulfill({ json: {
        ...cash, funds: cash.funds.map(fund => ({ ...fund, balance: amount("0.00") })),
      } }));
      await page.keyboard.press("r");
      const reserves = page.locator(".overview-reserves");
      await expect(reserves.getByText("Nothing is set aside yet", { exact: true })).toBeVisible();
      await reserves.getByRole("button", { name: "Set aside", exact: true }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
    });
    test("activity filters CLI changes and explains an empty filter", async ({ page }) => {
      await setup(page);
      await page.route("**/history", route => route.fulfill({ json: [
        { hash: "43", revision: 43, summary: "Recorded Northwind Logistics", actor: "cli", user: "Dana Okafor", date: "2026-10-24T12:00:00Z" },
        { hash: "42", revision: 42, summary: "Recorded Maple Market", actor: "app", user: "Dana Okafor", date: "2026-10-23T12:00:00Z" },
      ] }));
      await page.keyboard.press("r");
      if (viewport.width < 600) await page.locator('[data-destination="More"]').click();
      await page.locator('[data-destination="Activity"]').click();
      const filter = page.getByRole("combobox", { name: "Activity", exact: true });
      await filter.selectOption("cli");
      await expect(page.getByRole("heading", { name: "Recorded Northwind Logistics" })).toBeVisible();
      await expect(page.getByRole("heading", { name: "Recorded Maple Market" })).toHaveCount(0);
      await filter.selectOption("mcp");
      await expect(page.getByText("No changes", { exact: true })).toBeVisible();
      await filter.selectOption("");
      await expect(page.getByRole("heading", { name: "Recorded Maple Market" })).toBeVisible();
    });
    test("cash trend clears errors after a successful refresh", async ({ page }) => {
      await setup(page);
      await page.route("**/reports/cash-on-hand-history?**", route => route.fulfill({ status: 503, json: { message: "History temporarily unavailable" } }));
      await page.route("**/version", route => route.fulfill({ json: { ...version, head: "43", books: "43" } }));
      await page.keyboard.press("r");
      const trend = page.locator(".overview-trend");
      await expect(trend.locator(".error")).toBeVisible();
      await page.route("**/reports/cash-on-hand-history?**", route => route.fulfill({ json: { commodity: "USD", points: [{ date: "2026-10-25", cash_on_hand: "4520.75" }] } }));
      await page.route("**/version", route => route.fulfill({ json: { ...version, head: "43", books: "43" } }));
      await page.keyboard.press("r");
      await expect(trend.getByRole("img")).toBeVisible();
      await expect(trend.locator(".error")).toHaveCount(0);
    });
  });
}

const walletCommodity = { symbol: "USD", places: 2, commodities: [
  {symbol:"USD",places:2},{symbol:"ETH",places:18},{symbol:"XMR",places:12},
] };
const walletAccounts = [
  {...accounts[0], balances:[amount("4193.46")]},
  {account:"assets:investments:ethereum",display_name:"Ethereum wallet",kind:"non-liquid",balances:[{commodity:"ETH",exact:"1.634567890123456789",formatted:"ETH 1.634567890123456789"}]},
  {account:"assets:investments:monero",display_name:"Monero wallet",kind:"non-liquid",balances:[{commodity:"XMR",exact:"12.345678901234",formatted:"XMR 12.345678901234"}]},
  {account:"expenses:fees:platform",display_name:"Platform fees",kind:"expense"},
  {account:"expenses:fees:broker",display_name:"Broker fees",kind:"expense"},
];
for (const phone of [false, true]) test.describe(`wallets on ${phone ? "phone" : "desktop"}`, () => {
  test.use({viewport:phone ? {width:390,height:844}:{width:1280,height:800},isMobile:phone,hasTouch:phone});
  test("transfer records exact wallet units, conversion price and separate fees", async({page}) => {
    const writes = await setup(page,false,transaction,walletAccounts,walletCommodity);
    const select = async(name,value,label=value) => {
      const field=page.locator(`[name="${name}"]`);
      if (!phone) return field.selectOption(value);
      await field.tap();
      if (/^(currency|cost_currency)/.test(name)) await expect(page.locator('.phone-picker').getByRole('heading')).toHaveText('Currency');
      await page.locator('.phone-picker').getByRole('button',{name:label,exact:true}).tap();
    };
    await page.getByRole('button',{name:'Add',exact:true}).click();
    await page.getByRole('button',{name:/^Transfer/}).click();
    await page.getByLabel('Payee',{exact:true}).fill('Cedar Exchange | ETH to XMR');
    await page.locator('[name=account0]').fill('assets:investments:ethereum');
    await page.locator('[name=amount0]').fill('0.102500000000000001');
    await select('currency0','ETH');
    const amountBox = await page.locator('[name=amount0]').boundingBox();
    const currencyBox = await page.locator('[name=currency0]').boundingBox();
    expect(currencyBox.x).toBeGreaterThanOrEqual(amountBox.x + amountBox.width - 1);
    expect(Math.abs(currencyBox.y - amountBox.y)).toBeLessThan(2);
    if (phone) expect(currencyBox.height).toBeGreaterThanOrEqual(48);

    await page.locator('[name=account1]').fill('assets:investments:monero');
    await page.locator('[name=amount1]').fill('1.5');
    await select('currency1','XMR');
    await select('cost_kind1','total','Total price');
    await page.locator('[name=cost1]').fill('0.100000000000000001');
    await select('cost_currency1','ETH');
    for (const [row,account,value] of [[2,'expenses:fees:platform','0.0015'],[3,'expenses:fees:broker','0.001']]) {
      await page.getByRole('dialog',{name:'Transfer',exact:true}).getByRole('button',{name:'Add',exact:true}).click();
      await page.locator(`[name=account${row}]`).fill(account);
      await page.locator(`[name=amount${row}]`).fill(value);
      await select(`currency${row}`,'ETH');
    }
    if (phone) {
      await page.setViewportSize({width:844,height:390});
      await page.setViewportSize({width:390,height:844});
      await expect(page.locator('[name=amount0]')).toHaveValue('0.102500000000000001');
    }
    await page.getByRole('button',{name:phone?'Review':'Preview',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Review before saving',exact:true})).toBeVisible();
    const preview = writes.at(-1);
    expect(preview.body.postings.map(p=>[p.account,p.amount.commodity,p.amount.mantissa])).toEqual([
      ['assets:investments:ethereum','ETH','-102500000000000001'],
      ['assets:investments:monero','XMR','15'],
      ['expenses:fees:platform','ETH','15'],
      ['expenses:fees:broker','ETH','1'],
    ]);
    expect(preview.body.postings[1].cost).toEqual({kind:'total',amount:{commodity:'ETH',mantissa:'100000000000000001',places:18}});
    expect(preview.body.dry_run).toBe(true);
    await page.getByRole('button',{name:'Save',exact:true}).click();
    expect(writes.at(-1).body).toEqual({...preview.body,dry_run:false});
  });
  test("opening equity is separate from wallets and shown without a loss colour",async({page})=>{
    const equity={account:'equity:opening',display_name:'Opening balances',kind:'non-liquid',balance_inclusive:[amount('-5218.46')]};
    await setup(page,false,transaction,[...walletAccounts,equity],walletCommodity);
    await page.getByRole('navigation').getByRole('button',{name:'Transactions',exact:true}).click();
    const group=page.locator('.account-list section').filter({has:page.getByRole('heading',{name:'Other',exact:true})});
    await expect(group).toContainText('Opening balances');
    await expect(group).toContainText('$5218.46');
    await expect(group.locator('.negative')).toHaveCount(0);
  });
  test("spending from a wallet uses its currency instead of the reporting currency",async({page})=>{
    const writes = await setup(page,false,transaction,walletAccounts,walletCommodity);
    await page.getByRole('button',{name:'Add',exact:true}).click();
    await page.getByRole('button',{name:/^Spent/}).click();
    await page.getByLabel('Payee',{exact:true}).fill('Cedar Exchange network fee');
    await page.getByRole('combobox',{name:'From',exact:true}).selectOption('assets:investments:ethereum');
    await page.getByRole('combobox',{name:'To',exact:true}).fill('expenses:fees:platform');
    await page.getByLabel('Amount (ETH)',{exact:true}).fill('0.000000000000000001');
    await page.getByRole('button',{name:phone?'Review':'Preview',exact:true}).click();
    await expect.poll(()=>writes.at(-1)?.body.postings[1].amount).toEqual({commodity:'ETH',mantissa:'-1',places:18});
  });
  test("editing an exchange keeps every currency and its conversion price",async({page})=>{
    const tx={...transaction,postings:[
      {account:'assets:bank:checking',amount:amount('-1025.00')},
      {account:'assets:investments:ethereum',amount:{commodity:'ETH',exact:'0.4',formatted:'ETH 0.4'},cost:{kind:'total',amount:amount('1000.00')}},
      {account:'expenses:fees:platform',amount:amount('15.00')},
      {account:'expenses:fees:broker',amount:amount('10.00')},
    ]};
    const writes=await setup(page,false,tx,walletAccounts,walletCommodity);
    await page.getByRole('navigation').getByRole('button',{name:'Transactions',exact:true}).click();
    if (phone) {
      await page.getByRole('button',{name:'All transactions',exact:true}).click();
      await page.locator('.register-row').filter({hasText:'Maple Market'}).click();
    }
    else await page.getByRole('button',{name:'Edit',exact:true}).click();
    await page.getByLabel('Payee',{exact:true}).fill('Cedar Exchange | Fees checked');
    await page.getByRole('button',{name:phone?'Review':'Preview',exact:true}).click();
    await expect.poll(()=>writes.at(-1)?.body.postings[1]).toMatchObject({amount:{commodity:'ETH',mantissa:'4',places:1},cost:{kind:'total',amount:{commodity:'USD',mantissa:'100000',places:2}}});
    await page.getByRole('button',{name:'Save',exact:true}).click();
    expect(writes.at(-1).body.dry_run).toBe(false);
  });
  test("fee reports keep platform and broker separate in the selected currency",async({page})=>{
    await setup(page,false,transaction,walletAccounts,walletCommodity);
    await page.route('**/reports/categories?*',route=>{
      const query=new URL(route.request().url()).searchParams;
      expect(query.get('depth')).toBe('99');
      const currency=query.get('commodity');
      return route.fulfill({json:{commodity:currency,rows:[
        {account:'expenses:fees:platform',display_name:'Platform fees',amount:currency==='ETH'?'0.0015':'15.00'},
        {account:'expenses:fees:broker',display_name:'Broker fees',amount:currency==='ETH'?'0.001':'10.00'},
      ]}});
    });
    if(phone) await page.getByRole('navigation').getByRole('button',{name:'More',exact:true}).click();
    await page.getByRole('button',{name:'Reports',exact:true}).click();
    await page.getByRole('combobox',{name:'Reports',exact:true}).selectOption('categories');
    await expect(page.getByRole('row').filter({hasText:'Platform fees'})).toContainText('USD 15.00');
    await page.getByRole('combobox',{name:'Currency',exact:true}).selectOption('ETH');
    await expect(page.getByRole('row').filter({hasText:'Platform fees'})).toContainText('ETH 0.0015');
    await expect(page.getByRole('row').filter({hasText:'Broker fees'})).toContainText('ETH 0.001');
  });
  test("reconciliation sends the wallet currency and full precision",async({page})=>{
    const writes=await setup(page,false,transaction,walletAccounts,walletCommodity);
    await page.getByRole('navigation').getByRole('button',{name:'Transactions',exact:true}).click();
    if(phone) {
      await page.locator('.account-list button').filter({hasText:'Monero wallet'}).click();
      await page.locator('.phone-overflow summary').click();
    } else await page.getByRole('combobox',{name:'Account',exact:true}).selectOption('assets:investments:monero');
    await page.getByRole('button',{name:'Reconcile…',exact:true}).click();
    await page.getByLabel('Amount',{exact:true}).fill('12.345678901234');
    await page.getByRole('button',{name:phone?'Review':'Preview',exact:true}).click();
    await expect.poll(()=>writes.find(w=>w.path==='/reconcile')?.body.balance).toEqual({commodity:'XMR',mantissa:'12345678901234',places:12});
  });
});

for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
  for (const colorScheme of ["light", "dark"]) {
    test.describe(`spendable overview ${viewport.width}px ${colorScheme}`, () => {
      test.use({ viewport, colorScheme });
      test("distinguishes available money from its dated change", async ({ page }) => {
        await setup(page);
        await page.route("**/cash-on-hand?**", route => route.fulfill({ json: {
          ...cash, change: { compare_to: "2026-09-25", cash_on_hand: amount("393.95") },
        } }));
        await page.reload();
        const summary = page.locator(".cash-summary");
        await expect(summary.getByRole("heading", { name: "Available to spend", exact: true })).toBeVisible();
        await expect(summary.locator(".cash-total")).toHaveText("$4520.75");
        await expect(summary.locator(".cash-change")).toHaveText("+$393.95 since 2026-09-25");
        await expect(page.locator(".page-heading")).not.toContainText(/at a glance/i);
        await expect(page.getByRole("heading", { name: "Available by account" })).toBeVisible();
        const typography = await summary.locator(".cash-total").evaluate(node => {
          const style = getComputedStyle(node);
          return { family: style.fontFamily, numbers: style.fontVariantNumeric };
        });
        expect(typography.family).not.toContain("Playfair");
        expect(typography.numbers).toContain("lining-nums");
        expect(typography.numbers).toContain("tabular-nums");
        const contrasts = await summary.evaluate(panel => {
          const luminance = color => {
            const channels = color.match(/[\d.]+/g).slice(0, 3).map(Number).map(c => {
              const v = c / 255;
              return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
            });
            return channels.reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
          };
          const background = luminance(getComputedStyle(panel).backgroundColor);
          return [panel.querySelector("h2"), panel.querySelector(".cash-total"), panel.querySelector(".cash-change")].map(node => {
            const foreground = luminance(getComputedStyle(node).color);
            return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
          });
        });
        for (const contrast of contrasts) expect(contrast).toBeGreaterThanOrEqual(4.5);
        await summary.getByText("How it's worked out", { exact: true }).click();
        await expect(summary.locator("dl")).toBeVisible();
        await expect(summary.locator(".total-line")).toContainText("Available to spend");
        await summary.getByLabel("About Cash on Hand", { exact: true }).click();
        const explanation = summary.locator(".cash-label .info-button p");
        await expect(explanation).toBeVisible();
        const bounds = await explanation.boundingBox();
        expect(bounds.x).toBeGreaterThanOrEqual(0);
        expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width);
        await summary.getByLabel("About Cash on Hand", { exact: true }).click();
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await page.screenshot({ path: `test-results/spendable-${viewport.width}-${colorScheme}.png`, fullPage: true });
      });
    });
  }
}

test("spendable changes retain negative and zero meanings", async ({ page }) => {
  await setup(page);
  for (const [exact, expected] of [["-93.95", "-$93.95"], ["0.00", "$0.00"]]) {
    await page.route("**/cash-on-hand?**", route => route.fulfill({ json: {
      ...cash, change: { compare_to: "2026-09-25", cash_on_hand: amount(exact) },
    } }));
    await page.reload();
    await expect(page.locator(".cash-change")).toHaveText(`${expected} since 2026-09-25`);
  }
});
