import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";
import LogsPage from "../pages/LogsPage.vue";

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
      { path: "/", name: "main", component: () => import("../pages/SearchPage.vue") },
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

function configOr(fetchHandler) {
  return vi.fn((input, init) => {
    const path = String(input);
    if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
    return fetchHandler(path, init);
  });
}

async function mountAppAt(path, fetchMock, { flush = true } = {}) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push(path);
  await router.isReady();
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  if (flush) await flushPromises();
  return { wrapper: activeWrapper, router };
}

async function mountLogsPageAt(path, fetchMock) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push(path);
  await router.isReady();
  activeWrapper = mount(LogsPage, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return { wrapper: activeWrapper, router };
}

describe("LogsPage component", () => {
  it("loads on mount and appends the next page", async () => {
    const fetchMock = vi.fn((input) => {
      const path = String(input);
      if (path === "/api/operation-logs?page=1&page_size=30") {
        return Promise.resolve(
          jsonResponse({
            items: [{ id: 1, summary: "第一页日志", category: "search", status: "success" }],
            page: 1,
            page_size: 30,
            total: 2,
            has_more: true,
          }),
        );
      }
      if (path === "/api/operation-logs?page=2&page_size=30") {
        return Promise.resolve(
          jsonResponse({
            items: [{ id: 2, summary: "第二页日志", category: "qb_push", status: "failed" }],
            page: 2,
            page_size: 30,
            total: 2,
            has_more: false,
          }),
        );
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountLogsPageAt("/logs", fetchMock);

    expect(wrapper.get("#page-logs").exists()).toBe(true);
    expect(wrapper.get(".operation-log-card h2").text()).toBe("第一页日志");
    expect(wrapper.get(".operation-log-summary").text()).toContain("已显示 1 条");

    await wrapper.get(".operation-log-pager button").trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".operation-log-card h2").map((node) => node.text())).toEqual([
      "第一页日志",
      "第二页日志",
    ]);
    expect(wrapper.get(".operation-log-summary").text()).toContain("已显示 2 条");
    expect(wrapper.get(".operation-log-pager button").attributes("disabled")).toBeDefined();
  });
});

describe("LogsPage router behavior", () => {
  it("renders through the lazy route and keeps the latest filter result", async () => {
    const older = deferred();
    const newer = deferred();
    const fetchMock = configOr((path) => {
      if (path === "/api/operation-logs?page=1&page_size=30") {
        return Promise.resolve(jsonResponse({ items: [], page: 1, page_size: 30, total: 0 }));
      }
      if (path.includes("q=older")) return older.promise;
      if (path.includes("q=newer")) return newer.promise;
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountAppAt("/logs", fetchMock);

    expect(router.currentRoute.value.name).toBe("logs");
    expect(wrapper.get("#page-logs").exists()).toBe(true);
    const input = wrapper.get(".operation-log-search input");

    await input.setValue("older");
    await input.trigger("keydown.enter");
    await vi.waitFor(() => expect(router.currentRoute.value.query.q).toBe("older"));
    await input.setValue("newer");
    await input.trigger("keydown.enter");
    await vi.waitFor(() => expect(router.currentRoute.value.query.q).toBe("newer"));

    newer.resolveJson({
      items: [{ id: 2, summary: "新日志", category: "search", status: "success" }],
      page: 1,
      page_size: 30,
      total: 1,
    });
    await flushPromises();
    expect(wrapper.get(".operation-log-card h2").text()).toBe("新日志");

    older.resolveJson({
      items: [{ id: 1, summary: "旧日志", category: "search", status: "success" }],
      page: 1,
      page_size: 30,
      total: 1,
    });
    await flushPromises();
    expect(wrapper.get(".operation-log-card h2").text()).toBe("新日志");
    expect(
      fetchMock.mock.calls
        .map(([input]) => String(input))
        .filter((path) => path.startsWith("/api/operation-logs")),
    ).toEqual([
      "/api/operation-logs?page=1&page_size=30",
      "/api/operation-logs?page=1&page_size=30&q=older",
      "/api/operation-logs?page=1&page_size=30&q=newer",
    ]);
  });

  it("hydrates a deep link and follows browser Back/Forward filter history", async () => {
    const fetchMock = configOr((path) => {
      const query = new URL(path, "http://localhost").searchParams;
      const q = query.get("q") || "";
      if (!path.startsWith("/api/operation-logs?")) {
        throw new Error(`Unexpected request: ${path}`);
      }
      return Promise.resolve(
        jsonResponse({
          items: [{ id: q || "none", summary: q || "无筛选" }],
          page: 1,
          page_size: 30,
          total: 1,
          has_more: false,
        }),
      );
    });
    const { wrapper, router } = await mountAppAt(
      "/logs?category=search&status=failed&q=first",
      fetchMock,
    );

    expect(wrapper.get("select").element.value).toBe("search");
    expect(wrapper.findAll("select")[1].element.value).toBe("failed");
    expect(wrapper.get(".operation-log-search input").element.value).toBe("first");
    expect(wrapper.get(".operation-log-card h2").text()).toBe("first");

    const input = wrapper.get(".operation-log-search input");
    await input.setValue("second");
    await input.trigger("keydown.enter");
    await vi.waitFor(() => expect(router.currentRoute.value.query.q).toBe("second"));
    await vi.waitFor(() => expect(wrapper.get(".operation-log-card h2").text()).toBe("second"));

    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.query.q).toBe("first"));
    await vi.waitFor(() => expect(wrapper.get(".operation-log-card h2").text()).toBe("first"));

    router.forward();
    await vi.waitFor(() => expect(router.currentRoute.value.query.q).toBe("second"));
    await vi.waitFor(() => expect(wrapper.get(".operation-log-card h2").text()).toBe("second"));
  });

  it("cancels the page request when browser Back leaves the logs route", async () => {
    let logsSignal;
    const fetchMock = configOr((path, init) => {
      if (path !== "/api/operation-logs?page=1&page_size=30") {
        throw new Error(`Unexpected request: ${path}`);
      }
      logsSignal = init.signal;
      return new Promise((_resolve, reject) => {
        const rejectOnAbort = () => reject(logsSignal.reason);
        if (logsSignal.aborted) rejectOnAbort();
        else logsSignal.addEventListener("abort", rejectOnAbort, { once: true });
      });
    });
    const { wrapper, router } = await mountAppAt("/", fetchMock);
    const logsNav = wrapper.findAll(".nav-item").find((button) => button.text() === "日志");

    await logsNav.trigger("click");
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("logs"));
    await vi.waitFor(() => expect(logsSignal).toBeDefined());

    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("main"));
    await flushPromises();

    expect(logsSignal.aborted).toBe(true);
    expect(wrapper.find("#page-logs").exists()).toBe(false);
    expect(wrapper.get("#page-main").isVisible()).toBe(true);
  });
});
