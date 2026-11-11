// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState } from "react";
import tokens from "../../../design/tokens.json";
export const phoneQuery = `(width < ${tokens.breakpoint.phone.value}px)`;
export const desktopQuery = `(width >= ${tokens.breakpoint.desktop.value}px)`;
export function useMedia(query) {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setMatches(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [query]);
  return matches;
}
