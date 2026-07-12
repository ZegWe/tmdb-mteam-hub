import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";

const EmptyRoute = { template: "" };
let activeWrapper = null;

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "main", component: EmptyRoute },
      { path: "/detail/:mediaType/:id", name: "media-detail", component: EmptyRoute },
      { path: "/subscriptions", name: "subscriptions", component: EmptyRoute },
      { path: "/subscriptions/:id", name: "subscription-detail", component: EmptyRoute },
      { path: "/logs", name: "logs", component: EmptyRoute },
      { path: "/settings", name: "settings", component: () => import("../pages/SettingsPage.vue") },
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
    has_douban_cookie: true,
    qb_servers: [
      {
        id: "nas",
        name: "NAS",
        base_url: "http://127.0.0.1:8080",
        username: "admin",
        insecure_tls: false,
        has_password: true,
        password: "LEGACY_RESPONSE_SECRET_MUST_BE_DROPPED",
      },
    ],
    subscription_categories: [],
    subscription_watcher: subscriptionWatcher(),
    torrent_match_rules: [],
    restart_required: false,
    ...overrides,
  };
}

async function mountSettings(fetchMock) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push("/settings");
  await router.isReady();
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return activeWrapper;
}

function labeledField(wrapper, text, selector) {
  const label = wrapper.findAll("label").find((candidate) => candidate.text().includes(text));
  if (!label) throw new Error(`Missing label: ${text}`);
  return label.get(selector);
}

function buttonByText(wrapper, text) {
  const button = wrapper.findAll("button").find((candidate) => candidate.text().includes(text));
  if (!button) throw new Error(`Missing button: ${text}`);
  return button;
}

describe("settings security contract", () => {
  it("keeps redacted secrets, requires revision, and sends only saved qB IDs to actions", async () => {
    let savedPayload = null;
    let qbTestPayload = null;
    let snapshot = configSnapshot();
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") return jsonResponse(snapshot);
      if (path === "/api/config" && method === "PUT") {
        savedPayload = JSON.parse(init.body);
        snapshot = configSnapshot({
          revision: 8,
          qb_servers: savedPayload.qb_servers.map((server) => ({
            ...server,
            has_password: true,
          })),
        });
        return jsonResponse(snapshot);
      }
      if (path === "/api/qb/test" && method === "POST") {
        qbTestPayload = JSON.parse(init.body);
        return jsonResponse({ version: "5.0.4" });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    const tmdbInput = labeledField(wrapper, "TMDB API Key", "input");
    const mteamInput = labeledField(wrapper, "M-Team OpenAPI Key", "input");
    const cookieInput = labeledField(wrapper, "豆瓣 Cookie", "textarea");

    expect(tmdbInput.element.value).toBe("");
    expect(mteamInput.element.value).toBe("");
    expect(cookieInput.element.value).toBe("");
    expect(wrapper.html()).not.toContain("LEGACY_RESPONSE_SECRET_MUST_BE_DROPPED");

    const testButton = buttonByText(wrapper, "测试连接");
    expect(testButton.element.disabled).toBe(false);
    await testButton.trigger("click");
    await flushPromises();
    expect(qbTestPayload).toEqual({ server_id: "nas" });

    await mteamInput.setValue("replacement-mteam-key");
    await wrapper.get("#settings-form").trigger("submit");
    await flushPromises();

    expect(savedPayload).toMatchObject({
      expected_revision: 7,
      mteam_api_key: "replacement-mteam-key",
      subscription_categories: [],
      torrent_match_rules: [],
    });
    expect(savedPayload).not.toHaveProperty("tmdb_api_key");
    expect(savedPayload).not.toHaveProperty("douban_cookie");
    expect(savedPayload.qb_servers).toEqual([
      {
        id: "nas",
        name: "NAS",
        base_url: "http://127.0.0.1:8080",
        username: "admin",
        insecure_tls: false,
      },
    ]);
    expect(JSON.stringify(savedPayload)).not.toContain("LEGACY_RESPONSE_SECRET_MUST_BE_DROPPED");
    expect(mteamInput.element.value).toBe("");
  });

  it("does not send a PUT when first-time automation enablement is cancelled", async () => {
    const confirmEnable = vi.fn(() => false);
    vi.stubGlobal("confirm", confirmEnable);
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") return jsonResponse(configSnapshot());
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    await labeledField(wrapper, "启用自动订阅", 'input[type="checkbox"]').setValue(true);
    await wrapper.get("#settings-form").trigger("submit");
    await flushPromises();

    expect(confirmEnable).toHaveBeenCalledOnce();
    const warning = String(confirmEnable.mock.calls[0][0]);
    expect(warning).toMatch(/订阅自动化/);
    expect(warning).toMatch(/下载|推送/);
    expect(warning).toMatch(/硬链接/);
    expect(
      fetchMock.mock.calls.filter(([input, init = {}]) => {
        return String(input) === "/api/config" && String(init.method).toUpperCase() === "PUT";
      }),
    ).toHaveLength(0);
  });

  it("sends a top-level confirmation flag after first-time automation enablement is confirmed", async () => {
    const confirmEnable = vi.fn(() => true);
    vi.stubGlobal("confirm", confirmEnable);
    let savedPayload = null;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") return jsonResponse(configSnapshot());
      if (path === "/api/config" && method === "PUT") {
        savedPayload = JSON.parse(init.body);
        return jsonResponse(
          configSnapshot({
            revision: 8,
            subscription_watcher: savedPayload.subscription_watcher,
          }),
        );
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    await labeledField(wrapper, "启用自动订阅", 'input[type="checkbox"]').setValue(true);
    await labeledField(wrapper, "试运行（dry-run）", 'input[type="checkbox"]').setValue(false);
    await wrapper.get("#settings-form").trigger("submit");
    await flushPromises();

    expect(confirmEnable).toHaveBeenCalledOnce();
    expect(savedPayload.confirm_enable_automation).toBe(true);
    expect(savedPayload.subscription_watcher).toEqual(
      subscriptionWatcher({ enabled: true, dry_run: false }),
    );
  });

  it("saves an already-enabled watcher without confirmation or a confirmation field", async () => {
    const confirmEnable = vi.fn(() => true);
    vi.stubGlobal("confirm", confirmEnable);
    let savedPayload = null;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") {
        return jsonResponse(
          configSnapshot({ subscription_watcher: subscriptionWatcher({ enabled: true }) }),
        );
      }
      if (path === "/api/config" && method === "PUT") {
        savedPayload = JSON.parse(init.body);
        return jsonResponse(
          configSnapshot({
            revision: 8,
            subscription_watcher: savedPayload.subscription_watcher,
          }),
        );
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    expect(labeledField(wrapper, "启用自动订阅", 'input[type="checkbox"]').element.checked).toBe(
      true,
    );
    await labeledField(wrapper, "试运行（dry-run）", 'input[type="checkbox"]').setValue(false);
    await wrapper.get("#settings-form").trigger("submit");
    await flushPromises();

    expect(confirmEnable).not.toHaveBeenCalled();
    expect(savedPayload).not.toHaveProperty("confirm_enable_automation");
    expect(savedPayload.subscription_watcher).toEqual(
      subscriptionWatcher({ enabled: true, dry_run: false }),
    );
  });

  it("marks edited qB forms unsaved until the configuration update succeeds", async () => {
    let snapshot = configSnapshot();
    let qbTestCalls = 0;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") return jsonResponse(snapshot);
      if (path === "/api/config" && method === "PUT") {
        const payload = JSON.parse(init.body);
        snapshot = configSnapshot({
          revision: 8,
          qb_servers: payload.qb_servers.map((server) => ({ ...server, has_password: true })),
        });
        return jsonResponse(snapshot);
      }
      if (path === "/api/qb/test" && method === "POST") {
        qbTestCalls += 1;
        return jsonResponse({ version: "5.0.4" });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    const baseUrl = labeledField(wrapper, "Web UI 根地址", "input");
    const testButton = buttonByText(wrapper, "测试连接");

    await baseUrl.setValue("http://127.0.0.1:9090");
    expect(testButton.element.disabled).toBe(true);
    expect(wrapper.text()).toContain("请先保存后测试");
    await testButton.trigger("click");
    expect(qbTestCalls).toBe(0);

    await wrapper.get("#settings-form").trigger("submit");
    await flushPromises();
    expect(buttonByText(wrapper, "测试连接").element.disabled).toBe(false);
  });

  it("refreshes only redacted metadata after QR login and never puts a Cookie in the form", async () => {
    const leakedCookie = "dbcl2=COOKIE_MUST_NEVER_REACH_THE_FORM; ck=test";
    let cookieSaved = false;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config" && method === "GET") {
        return jsonResponse(
          configSnapshot({
            revision: cookieSaved ? 8 : 7,
            has_douban_cookie: cookieSaved,
          }),
        );
      }
      if (path === "/api/douban/qr/start" && method === "POST") {
        return jsonResponse({
          session_id: "qr-session",
          image_url: "/api/douban/qr/image?session_id=qr-session",
        });
      }
      if (path.startsWith("/api/douban/qr/poll?") && method === "GET") {
        cookieSaved = true;
        return jsonResponse({
          done: true,
          cookie_saved: true,
          description: "登录成功",
          cookie_header: leakedCookie,
        });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });

    const wrapper = await mountSettings(fetchMock);
    await buttonByText(wrapper, "QR 登录获取 Cookie").trigger("click");
    await flushPromises();

    expect(labeledField(wrapper, "豆瓣 Cookie", "textarea").element.value).toBe("");
    expect(wrapper.text()).toContain("Cookie 已安全保存");
    expect(wrapper.html()).not.toContain(leakedCookie);
  });
});
