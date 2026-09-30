// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, expect, test, vi } from "vitest";
const state = vi.hoisted(() => ({
  settings: [],
  user: null,
  callback: vi.fn(),
  revoke: vi.fn(),
  silent: vi.fn(),
}));
vi.mock("oidc-client-ts", async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    UserManager: class {
      constructor(settings) {
        state.settings.push(settings);
        this.store = settings.userStore;
        this.key = `user:${settings.authority}:${settings.client_id}`;
      }
      async getUser() {
        if (state.user) return state.user;
        const stored = await this.store.get(this.key);
        return stored ? actual.User.fromStorageString(stored) : null;
      }
      signinSilent() {
        return state.silent();
      }
      signinRedirectCallback() {
        return state.callback();
      }
      signinRedirect() {
        return Promise.resolve();
      }
      revokeTokens() {
        return state.revoke();
      }
      async removeUser() {
        state.user = null;
        await this.store.remove(this.key);
      }
    },
  };
});
import { User } from "oidc-client-ts";
import { initializeAuth } from "./auth";
afterEach(() => {
  vi.unstubAllGlobals();
  state.settings.length = 0;
  state.user = null;
  state.callback.mockReset();
  state.revoke.mockReset();
  state.silent.mockReset();
});
function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    get length() {
      return values.size;
    },
  };
}
function storedUser(expiresIn = 3600) {
  return new User({
    access_token: "household-access-6c47a1",
    refresh_token: "household-refresh-8d20b5",
    token_type: "Bearer",
    scope: "openid profile offline_access",
    expires_at: Math.floor(Date.now() / 1000) + expiresIn,
    profile: {
      sub: "e6f371c2-514b-4abf-89a3-816fd93a8b72",
      preferred_username: "maya.chen",
    },
  });
}
function fixture(issuer, search = "") {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockImplementation(async () =>
      new Response(
        JSON.stringify({
          auth: {
            mode: issuer ? "oauth" : "none",
            issuer,
            app_client_id: "household-web",
          },
        }),
      ),
    ),
  );
  vi.stubGlobal("window", {});
  vi.stubGlobal("sessionStorage", storage());
  vi.stubGlobal("localStorage", storage());
  vi.stubGlobal("location", { origin: "https://books.example.org", search });
  vi.stubGlobal("history", { replaceState: vi.fn() });
}
test("local no-auth mode does not create an OAuth session", async () => {
  fixture(null);
  const auth = await initializeAuth();
  expect(auth.signedIn).toBe(true);
  expect(await auth.token()).toBe(null);
  expect(state.settings).toHaveLength(0);
});
test.each([
  "https://identity.example.org/realms/household",
  "https://login.example.net",
])("discovers the configured issuer %s and uses code flow", async (issuer) => {
  fixture(issuer);
  state.user = { expired: false, access_token: "bearer" };
  const auth = await initializeAuth();
  expect(state.settings[0]).toMatchObject({
    authority: issuer,
    response_type: "code",
    redirect_uri: "https://books.example.org/app/",
    client_id: "household-web",
    scope: "openid profile offline_access",
  });
  expect(await auth.token()).toBe("bearer");
  state.user.expired = true;
  await expect(auth.token()).rejects.toThrow("session ended");
});
test.each([
  "https://identity.example.org/realms/household",
  "https://login.example.net",
])("persists user and redirect state across app restarts for %s", async (issuer) => {
  fixture(issuer);
  await initializeAuth();
  const settings = state.settings[0];
  const user = storedUser();
  const userKey = `user:${issuer}:household-web`;
  await settings.userStore.set(userKey, user.toStorageString());
  const redirectState = JSON.stringify({ client_id: "household-web" });
  await settings.stateStore.set("signin-7c91a4", redirectState);
  expect(localStorage.getItem(`oidc.${userKey}`)).toBe(user.toStorageString());
  expect(localStorage.getItem("oidc.signin-7c91a4")).toBe(redirectState);
  expect(sessionStorage.length).toBe(0);

  sessionStorage.clear();
  const restarted = await initializeAuth();
  expect(restarted.signedIn).toBe(true);
  expect(restarted.identity).toBe("maya.chen");
  expect(await restarted.token()).toBe(user.access_token);
  expect(await state.settings[1].stateStore.get("signin-7c91a4")).toBe(
    redirectState,
  );
});

test("renews a persisted expired access token using its refresh token", async () => {
  const issuer = "https://login.example.net";
  fixture(issuer);
  const expired = storedUser(-120);
  localStorage.setItem(
    `oidc.user:${issuer}:household-web`,
    expired.toStorageString(),
  );
  const renewed = storedUser();
  renewed.access_token = "household-access-9a73e2";
  state.silent.mockResolvedValue(renewed);
  const auth = await initializeAuth();
  await expect(auth.token()).resolves.toBe(renewed.access_token);
  expect(state.silent).toHaveBeenCalledOnce();
});

test("validates callbacks through the protocol library and removes URL parameters even on failure", async () => {
  fixture("https://identity.example.org", "?state=untrusted&code=code");
  state.callback.mockRejectedValue(new Error("Invalid state"));
  await expect(initializeAuth()).rejects.toThrow("Invalid state");
  expect(history.replaceState).toHaveBeenCalledWith({}, "", "/app/");
});

test("native sessions use the loopback-authenticated token without creating a web sign-in", async () => {
  const values = new Map([["kabacus.native-token", "native-access-token"]]);
  vi.stubGlobal("window", { __KABACUS_NATIVE__: true });
  vi.stubGlobal("sessionStorage", { getItem: (key) => values.get(key) });
  vi.stubGlobal("location", { href: "" });
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  const auth = await initializeAuth();
  expect(auth.signedIn).toBe(true);
  expect(await auth.token()).toBe("native-access-token");
  values.set("kabacus.native-token", "renewed-token");
  expect(await auth.token()).toBe("renewed-token");
  expect(fetch).not.toHaveBeenCalled();
  expect(state.settings).toHaveLength(0);
  await auth.signOut();
  expect(location.href).toBe("/app/?native-signout=1");
});

test.each([false, true])("sign-out removes the persisted user (revocation fails: %s)", async (fails) => {
  const issuer = "https://identity.example.org";
  fixture(issuer);
  location.reload = vi.fn();
  const key = `oidc.user:${issuer}:household-web`;
  localStorage.setItem(key, storedUser().toStorageString());
  if (fails)
    state.revoke.mockRejectedValue(new Error("Synthetic provider unavailable"));
  const auth = await initializeAuth();
  expect(auth.signedIn).toBe(true);
  if (fails) {
    await expect(auth.signOut()).rejects.toThrow("Synthetic provider unavailable");
  } else {
    await auth.signOut();
  }
  expect(state.revoke).toHaveBeenCalledOnce();
  expect(localStorage.getItem(key)).toBe(null);
  expect(location.reload).toHaveBeenCalledOnce();
  expect((await initializeAuth()).signedIn).toBe(false);
});
