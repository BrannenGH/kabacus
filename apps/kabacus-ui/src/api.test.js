// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, expect, test, vi } from "vitest";
import { Api, exactAmount, previousMonthDate } from "./api";
afterEach(() => vi.unstubAllGlobals());
test("preserves large decimals and rejects rounding", () => {
  expect(exactAmount("-9007199254740993.27", "USD", 2)).toEqual({
    commodity: "USD",
    mantissa: "-900719925474099327",
    places: 2,
  });
  for (const amount of ["1e3", "1.005", "", "NaN", "1,000"])
    expect(() => exactAmount(amount, "USD", 2)).toThrow();
});
test("saves against the preview revision with bearer authorization", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValue(new Response(JSON.stringify({ head: "43" })));
  vi.stubGlobal("fetch", fetch);
  await new Api(async () => "session-token").write(
    "/allocations",
    { amount: "100.00" },
    "42",
    false,
  );
  expect(fetch.mock.calls[0][1].headers).toMatchObject({
    "If-Match": '"42"',
    Authorization: "Bearer session-token",
  });
  expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({
    amount: "100.00",
    dry_run: false,
  });
});
test("conflicts never retry a financial write", async () => {
  const fetch = vi.fn().mockResolvedValue(new Response("{}", { status: 409 }));
  vi.stubGlobal("fetch", fetch);
  await expect(
    new Api().write("/transactions", {}, "42", false),
  ).rejects.toThrow("books changed");
  expect(fetch).toHaveBeenCalledTimes(1);
});
test("does not associate stale data with a new revision", async () => {
  let versions = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async (path) =>
        new Response(
          JSON.stringify(
            path === "/version"
              ? { head: String(++versions), books: String(versions) }
              : [],
          ),
        ),
    ),
  );
  await expect(new Api().snapshot()).rejects.toThrow("books changed");
});

test("month comparisons clamp at month end, including leap years", () => {
  expect(previousMonthDate(new Date(2024, 2, 31))).toBe("2024-02-29");
  expect(previousMonthDate(new Date(2025, 2, 31))).toBe("2025-02-28");
  expect(previousMonthDate(new Date(2026, 0, 15))).toBe("2025-12-15");
});

test('a conflicting save names the current revision actor without retrying the write', async () => {
  const fetch = vi.fn(async (_path,options) => new Response(JSON.stringify(options.method==='PATCH'?{current:'43'}:[{hash:'43',user:'Morgan Reed',summary:'Recorded Cedar Hardware'}]),{status:options.method==='PATCH'?409:200}));
  vi.stubGlobal('fetch',fetch);
  await expect(new Api().request('/transactions/12',{method:'PATCH',head:'42',body:{dry_run:false}})).rejects.toThrow('Morgan Reed saved a change');
  expect(fetch.mock.calls.filter(([,options])=>options.method==='PATCH')).toHaveLength(1);
});

test("document checkpoint failures preserve the host message and never retry", async () => {
  const body = {
    code: "document_conflict",
    message: "Your changes are saved locally; the file host could not be updated.",
    details: { committed_locally: true, retry_write: false },
  };
  const fetch = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(body), { status: 409 }),
  );
  vi.stubGlobal("fetch", fetch);
  await expect(
    new Api().write("/transactions", {}, "42", false),
  ).rejects.toThrow(body.message);
  expect(fetch).toHaveBeenCalledTimes(1);
});
