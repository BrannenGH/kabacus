// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React from "react";
export function Amount({ value }) {
  return <span className={`amount ${value?.exact?.startsWith("-") ? "negative" : ""}`}>{value?.formatted ?? "—"}</span>;
}
