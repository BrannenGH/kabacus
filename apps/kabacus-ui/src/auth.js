// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { t } from "./i18n";
import { UserManager, WebStorageStateStore } from "oidc-client-ts";
export async function initializeAuth() {
  if (window.__KABACUS_NATIVE__) {
    if (!sessionStorage.getItem("kabacus.native-token")) {
      await new Promise((resolve, reject) => {
        const ready = () => {
          clearTimeout(timer);
          window.removeEventListener("kabacus-native-ready", ready);
          resolve();
        };
        const timer = setTimeout(() => {
          window.removeEventListener("kabacus-native-ready", ready);
          reject(new Error(t("Not signed in")));
        }, 30000);
        window.addEventListener("kabacus-native-ready", ready);
      });
    }
    if (!sessionStorage.getItem("kabacus.native-token"))
      throw new Error(t("Not signed in"));
    return {
      signedIn: !!sessionStorage.getItem("kabacus.native-token"),
      identity: sessionStorage.getItem("kabacus.native-user") || "",
      token: async () => {
        const token = sessionStorage.getItem("kabacus.native-token");
        if (!token) throw new Error(t("Your session ended. Sign in again."));
        return token;
      },
      signOut: async () => {
        location.href = "/app/?native-signout=1";
      },
    };
  }
  const response = await fetch("/version", {
    signal: AbortSignal.timeout(30000),
    cache: "no-store",
    credentials: "omit",
  });
  if (!response.ok) throw new Error(t("Can't reach kabacus-server"));
  const version = await response.json();
  if (version.auth.mode === "none")
    return {
      token: async () => null,
      signedIn: true,
    };
  if (!version.auth.issuer || !version.auth.app_client_id)
    throw new Error(
      t("The server uses OAuth but has no client id for the app"),
    );
  const manager = new UserManager({
    authority: version.auth.issuer,
    client_id: version.auth.app_client_id,
    redirect_uri: `${location.origin}/app/`,
    response_type: "code",
    scope: "openid profile offline_access",
    userStore: new WebStorageStateStore({
      store: localStorage,
    }),
    stateStore: new WebStorageStateStore({
      store: localStorage,
    }),
    automaticSilentRenew: true,
    loadUserInfo: false,
  });
  if (new URLSearchParams(location.search).has("state")) {
    try {
      await manager.signinRedirectCallback();
    } finally {
      history.replaceState({}, "", "/app/");
    }
  }
  const user = await manager.getUser();
  return {
    signedIn: !!user && !user.expired,
    identity:
      user?.profile?.preferred_username ||
      user?.profile?.name ||
      user?.profile?.sub ||
      "",
    token: async () => {
      let current = await manager.getUser();
      if (current?.expired && current.refresh_token)
        current = await manager.signinSilent();
      if (!current || current.expired)
        throw new Error(t("Your session ended. Sign in again."));
      return current.access_token;
    },
    signIn: () => manager.signinRedirect(),
    signOut: async () => {
      try {
        await manager.revokeTokens();
      } finally {
        await manager.removeUser();
        location.reload();
      }
    },
  };
}
