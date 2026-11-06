// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { t } from "./i18n";
export class ApiError extends Error {
  constructor(status, body) {
    super(
      status === 409
        ? t("The books changed while you were editing. Review again.")
        : body.message || t("The server had a problem."),
    );
    this.status = status;
    this.detail = JSON.stringify(body, null, 2);
  }
}
export class Api {
  constructor(token = async () => null) {
    this.token = token;
  }
  async request(path, { method = "GET", body, head, signal } = {}) {
    const token = await this.token();
    const headers = {
      Accept: "application/json",
      "Kabacus-Client-Hint": "app",
    };
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (head !== undefined) headers["If-Match"] = `"${head}"`;
    const readSignal =
      method === "GET"
        ? signal
          ? AbortSignal.any([signal, AbortSignal.timeout(30000)])
          : AbortSignal.timeout(30000)
        : signal;
    let response;
    try {
      response = await fetch(path, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: readSignal,
        cache: "no-store",
        credentials: "omit",
      });
    } catch (error) {
      if (error.name === "TimeoutError")
        throw new ApiError(408, {
          message: t("The server is taking too long to answer."),
        });
      if (error.name === "AbortError") throw error;
      const failure = new ApiError(0, {
        message: t("Can't reach kabacus-server"),
      });
      failure.detail = error.message;
      throw failure;
    }
    let data;
    try {
      data = await response.json();
    } catch (error) {
      const failure = new ApiError(response.status, {
        message: t("The server had a problem."),
      });
      failure.detail = error.message;
      throw failure;
    }
    if (!response.ok) {
      const error = new ApiError(response.status, data);
      if (response.status === 409 && data.current) {
        try {
          const history = await this.request("/history", {
            signal: AbortSignal.timeout(3000),
          });
          const change = history.find(
            (item) => String(item.hash) === String(data.current),
          );
          if (change)
            error.message = t(
              "%1 saved a change while you were reviewing: %2. Yours hasn't been written.",
              change.user || change.actor,
              change.summary,
            );
        } catch {
          /* Keep the conflict even if its attribution cannot be read. */
        }
      }
      throw error;
    }
    return data;
  }
  async text(path, { signal } = {}) {
    const token = await this.token();
    const response = await fetch(path, {
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(30000)])
        : AbortSignal.timeout(30000),
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      credentials: "omit",
      cache: "no-store",
    });
    if (!response.ok)
      throw new ApiError(response.status, await response.json());
    return response.text();
  }
  async download(path, filename) {
    const token = await this.token();
    const response = await fetch(path, {
      signal: AbortSignal.timeout(30000),
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      credentials: "omit",
      cache: "no-store",
    });
    if (!response.ok)
      throw new ApiError(response.status, await response.json());
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async saveCopy() {
    const version=await this.request("/version");
    await this.download(`/ledgers/${version.ledger}/export`, "Books.kabacus");
  }
  async snapshot(signal) {
    // A changing revision must never lend a newer head to an older screen.
    for (let attempt = 0; attempt < 3; attempt++) {
      const start = await this.request("/version", {
        signal,
      });
      const [cash, accounts, transactions, history, commodity, commands] =
        await Promise.all(
          [
            `/cash-on-hand?compare_to=${previousMonthDate()}`,
            "/accounts",
            "/transactions",
            "/history",
            "/commodity", "/commands",
          ].map((path) =>
            this.request(path, {
              signal,
            }),
          ),
        );
      const end = await this.request("/version", {
        signal,
      });
      if (start.head === end.head && start.books === end.books)
        return {
          version: end,
          cash,
          accounts,
          transactions,
          history,
          commodity, commands,
        };
    }
    throw new Error(
      t("The books changed while you were editing. Review again."),
    );
  }
  write(path, body, head, dryRun) {
    return this.request(path, {
      method: "POST",
      head,
      body: {
        ...body,
        dry_run: dryRun,
      },
    });
  }
}
export function exactAmount(text, commodity, maxPlaces) {
  if (!/^-?\d+(\.\d+)?$/.test(text))
    throw new Error(
      t("Not an amount: use digits and a decimal point, like 12.34"),
    );
  const negative = text.startsWith("-");
  const [whole, fraction = ""] = text.replace(/^-/, "").split(".");
  if (fraction.length > maxPlaces)
    throw new Error(
      t("Not an amount: use digits and a decimal point, like 12.34"),
    );
  const digits = BigInt(whole + fraction);
  return {
    commodity,
    mantissa: (negative ? -digits : digits).toString(),
    places: fraction.length,
  };
}

export function previousMonthDate(now = new Date()) {
  const date = new Date(now.getFullYear(), now.getMonth(), 0);
  date.setDate(Math.min(now.getDate(), date.getDate()));
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
