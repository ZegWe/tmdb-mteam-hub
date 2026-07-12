import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";
import { APP_NOTIFICATIONS_KEY } from "../app/notifications.js";
import { createSettingsStore, SETTINGS_STORE_KEY } from "../features/settings/store.js";
import SettingsPage from "../pages/SettingsPage.vue";

const EmptyRoute = { template: "" };
let activeWrapper = null;

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  vi.restoreAllMocks();
});

function subscriptionWatcher(overrides = {}) {
  return {
    enabled: false,
    dry_run: true,
    poll_interval_secs: 4321,
    library_limit: 77,
    max_retries: 5,
    search_interval_secs: 654,
    progress_interval_secs: 9,
    link_retry_interval_secs: 876,
    system_retry_interval_secs: 321,
    bootstrap_existing_as_skipped: false,
    ...overrides,
  };
}

function configSnapshot(overrides = {}) {
  return {
    revision: 7,
    has_tmdb_api_key: true,
    has_mteam_api_key: true,
    has_douban_cookie: false,
    has_admin_token: true,
    qb_servers: [
      {
        id: "nas",
        name: "NAS",
        base_url: "http://127.0.0.1:8080",
        username: "admin",
        insecure_tls: false,
        has_password: true,
      },
    ],
    subscription_categories: [{ name: "电影", wanted_tag: "movie", qb_server_id: "nas" }],
    subscription_watcher: subscriptionWatcher(),
    torrent_match_rules: [],
    restart_required: false,
    ...overrides,
  };
}

function jsonResponse(value) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "main", component: EmptyRoute },
      { path: "/detail/:mediaType/:id", name: "media-detail", component: EmptyRoute },
      { path: "/subscriptions", name: "subscriptions", component: EmptyRoute },
      { path: "/subscriptions/:id", name: "subscription-detail", component: EmptyRoute },
      { path: "/logs", name: "logs", component: () => import("../pages/LogsPage.vue") },
      { path: "/settings", name: "settings", component: () => import("../pages/SettingsPage.vue") },
    ],
  });
}

describe("SettingsPage boundary", () => {
  it("loads its draft on mount and clears it when unmounted", async () => {
    const transport = {
      load: vi.fn().mockResolvedValue(configSnapshot()),
      save: vi.fn(),
      testQb: vi.fn(),
      startQr: vi.fn(),
      pollQr: vi.fn(),
    };
    const store = createSettingsStore({ transport });
    const notifications = {
      clearError: vi.fn(),
      showError: vi.fn(),
      showToast: vi.fn(),
    };

    activeWrapper = mount(SettingsPage, {
      attachTo: document.body,
      global: {
        provide: {
          [SETTINGS_STORE_KEY]: store,
          [APP_NOTIFICATIONS_KEY]: notifications,
        },
      },
    });
    await flushPromises();

    expect(transport.load).toHaveBeenCalledOnce();
    expect(activeWrapper.get("#page-settings").exists()).toBe(true);
    expect(activeWrapper.get("#settings-form").exists()).toBe(true);
    expect(store.pageLoaded.value).toBe(true);
    expect(store.form.qb_servers[0].id).toBe("nas");
    const managementToken = activeWrapper
      .findAll("label")
      .find((label) => label.text().includes("管理 Token"))
      ?.find('input[type="password"]');
    expect(managementToken?.exists()).toBe(true);
    expect(managementToken.element.value).toBe("");
    expect(managementToken.attributes("placeholder")).toContain("已配置");

    const enableAutomation = activeWrapper
      .findAll("label")
      .find((label) => label.text().includes("启用自动订阅"))
      ?.find('input[type="checkbox"]');
    const dryRun = activeWrapper
      .findAll("label")
      .find((label) => label.text().includes("试运行（dry-run）"))
      ?.find('input[type="checkbox"]');
    expect(enableAutomation?.exists()).toBe(true);
    expect(dryRun?.exists()).toBe(true);
    expect(enableAutomation.element.checked).toBe(false);
    expect(dryRun.element.checked).toBe(true);

    await enableAutomation.setValue(true);
    await dryRun.setValue(false);
    expect(store.form.subscription_watcher).toEqual({ enabled: true, dry_run: false });

    activeWrapper.unmount();
    activeWrapper = null;
    expect(store.pageLoaded.value).toBe(false);
    expect(store.form.qb_servers).toEqual([]);
  });
});

describe("SettingsPage router behavior", () => {
  it("loads lazily and clears its QR timer when navigation leaves settings", async () => {
    const setIntervalSpy = vi.spyOn(globalThis, "setInterval").mockImplementation(() => 73);
    const clearIntervalSpy = vi.spyOn(globalThis, "clearInterval").mockImplementation(() => {});
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") return jsonResponse(configSnapshot());
      if (path === "/api/douban/qr/start" && method === "POST") {
        return jsonResponse({
          session_id: "qr-session",
          image_url: "/api/douban/qr/image?session_id=qr-session",
        });
      }
      if (path.startsWith("/api/douban/qr/poll?") && method === "GET") {
        return jsonResponse({ done: false, description: "等待扫码" });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const router = createTestRouter();
    await router.push("/");
    await router.isReady();
    activeWrapper = mount(App, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();

    const settingsNav = activeWrapper
      .findAll(".nav-item")
      .find((button) => button.text() === "设置");
    await settingsNav.trigger("click");
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("settings"));
    await flushPromises();
    expect(activeWrapper.get("#page-settings").exists()).toBe(true);

    const qrButton = activeWrapper
      .findAll("button")
      .find((button) => button.text().includes("QR 登录获取 Cookie"));
    await qrButton.trigger("click");
    await flushPromises();
    expect(setIntervalSpy).toHaveBeenCalledOnce();

    await router.push("/");
    await flushPromises();
    expect(activeWrapper.find("#page-settings").exists()).toBe(false);
    expect(clearIntervalSpy).toHaveBeenCalledWith(73);
  });
});
