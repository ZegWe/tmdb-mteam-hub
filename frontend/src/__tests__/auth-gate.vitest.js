import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import AuthGate from "../app/AuthGate.vue";
import { notifyAuthenticationRequired } from "../shared/api/auth-session.js";

const EmptyRoute = { template: "" };
const CONFIG_RESPONSE = {
  revision: 1,
  has_tmdb_api_key: false,
  has_mteam_api_key: false,
  has_douban_cookie: false,
  qb_servers: [],
  subscription_categories: [],
  torrent_match_rules: [],
};

let activeWrapper = null;

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
});

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "main", component: EmptyRoute },
      { path: "/detail/:mediaType/:id", name: "media-detail", component: EmptyRoute },
      { path: "/subscriptions", name: "subscriptions", component: EmptyRoute },
      { path: "/subscriptions/:id", name: "subscription-detail", component: EmptyRoute },
      { path: "/logs", name: "logs", component: () => import("../pages/LogsPage.vue") },
      { path: "/settings", name: "settings", component: EmptyRoute },
    ],
  });
}

function jsonResponse(value, init = {}) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return {
    promise,
    resolveJson(value, init) {
      resolve(jsonResponse(value, init));
    },
  };
}

async function mountGate(fetchMock, { flush = true } = {}) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push("/");
  await router.isReady();
  activeWrapper = mount(AuthGate, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  if (flush) await flushPromises();
  return { wrapper: activeWrapper, router };
}

function authStatus({ authenticated, tokenConfigured = true, bootstrapAllowed = false }) {
  return {
    authenticated,
    token_configured: tokenConfigured,
    bootstrap_allowed: bootstrapAllowed,
  };
}

describe("AuthGate", () => {
  it("checks status before rendering App or requesting protected configuration", async () => {
    const status = deferred();
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") return status.promise;
      throw new Error(`Protected request started before auth status resolved: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock, { flush: false });

    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(String(fetchMock.mock.calls[0][0])).toBe("/api/auth/status");
    expect(wrapper.get('[role="status"]').text()).toContain("正在检查登录状态");
    expect(wrapper.find(".app-shell").exists()).toBe(false);

    status.resolveJson(authStatus({ authenticated: false }));
    await flushPromises();

    expect(wrapper.get(".auth-form").exists()).toBe(true);
    expect(wrapper.find(".app-shell").exists()).toBe(false);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("sends the token once to login and mounts App only after authentication succeeds", async () => {
    const token = "login-token-value-123456789";
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: false })));
      }
      if (path === "/api/auth/login") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: true })));
      }
      if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock);

    await wrapper.get("#management-token").setValue(token);
    await wrapper.get(".auth-form").trigger("submit");
    await flushPromises();

    expect(wrapper.get(".app-shell").exists()).toBe(true);
    const requestPaths = fetchMock.mock.calls.map(([input]) => String(input));
    expect(requestPaths).toEqual(["/api/auth/status", "/api/auth/login", "/api/config"]);
    const tokenCalls = fetchMock.mock.calls.filter(([, init]) =>
      String(init?.body || "").includes(token),
    );
    expect(tokenCalls).toHaveLength(1);
    expect(String(tokenCalls[0][0])).toBe("/api/auth/login");
    expect(tokenCalls[0][1]).toMatchObject({ method: "POST", credentials: "same-origin" });
    expect(window.localStorage.length).toBe(0);
  });

  it("shows a generic login error without echoing or retaining the submitted token", async () => {
    const token = "wrong-token-value-987654321";
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: false })));
      }
      if (path === "/api/auth/login") {
        return Promise.resolve(
          jsonResponse({ error: `rejected ${token}` }, { status: 401, statusText: "Unauthorized" }),
        );
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock);

    await wrapper.get("#management-token").setValue(token);
    await wrapper.get(".auth-form").trigger("submit");
    await flushPromises();

    expect(wrapper.get('[role="alert"]').text()).toBe("登录失败，请检查管理 Token 后重试");
    expect(wrapper.text()).not.toContain(token);
    expect(wrapper.get("#management-token").element.value).toBe("");
    expect(window.localStorage.length).toBe(0);
    expect(fetchMock.mock.calls.map(([input]) => String(input))).not.toContain("/api/config");
  });

  it("logs out through the server and returns to the token gate", async () => {
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: true })));
      }
      if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
      if (path === "/api/auth/logout") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: false })));
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock);

    expect(wrapper.get(".app-shell").exists()).toBe(true);
    await wrapper.get(".auth-logout").trigger("click");
    await flushPromises();

    expect(wrapper.find(".app-shell").exists()).toBe(false);
    expect(wrapper.get(".auth-form").exists()).toBe(true);
    const logoutCall = fetchMock.mock.calls.find(([input]) => String(input) === "/api/auth/logout");
    expect(logoutCall[1]).toMatchObject({ method: "POST", credentials: "same-origin" });
  });

  it("returns to the login gate when any protected request reports 401", async () => {
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") {
        return Promise.resolve(jsonResponse(authStatus({ authenticated: true })));
      }
      if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock);

    expect(wrapper.get(".app-shell").exists()).toBe(true);
    notifyAuthenticationRequired();
    await flushPromises();

    expect(wrapper.find(".app-shell").exists()).toBe(false);
    expect(wrapper.get(".auth-form").exists()).toBe(true);
  });

  it("enters App directly for authenticated loopback bootstrap without showing logout", async () => {
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/auth/status") {
        return Promise.resolve(
          jsonResponse(
            authStatus({
              authenticated: true,
              tokenConfigured: false,
              bootstrapAllowed: true,
            }),
          ),
        );
      }
      if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountGate(fetchMock);

    expect(wrapper.get(".app-shell").exists()).toBe(true);
    expect(wrapper.find(".auth-form").exists()).toBe(false);
    expect(wrapper.find(".auth-logout").exists()).toBe(false);
    expect(wrapper.get(".auth-bootstrap-warning").text()).toContain("loopback");
    expect(wrapper.get(".auth-bootstrap-warning").text()).toContain("管理 Token");
    expect(fetchMock.mock.calls.map(([input]) => String(input))).toEqual([
      "/api/auth/status",
      "/api/config",
    ]);
  });
});
