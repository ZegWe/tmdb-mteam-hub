import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";

const EmptyRoute = { template: "" };
let activeWrapper = null;

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
});

function configSnapshot() {
  return {
    revision: 1,
    has_tmdb_api_key: false,
    has_mteam_api_key: false,
    has_douban_cookie: false,
    qb_servers: [],
    subscription_categories: [],
    torrent_match_rules: [],
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
      { path: "/", name: "main", component: () => import("../pages/SearchPage.vue") },
      {
        path: "/detail/:mediaType/:id",
        name: "media-detail",
        component: () => import("../pages/MediaDetailPage.vue"),
      },
      { path: "/subscriptions", name: "subscriptions", component: EmptyRoute },
      { path: "/subscriptions/:id", name: "subscription-detail", component: EmptyRoute },
      { path: "/logs", name: "logs", component: EmptyRoute },
      { path: "/settings", name: "settings", component: EmptyRoute },
    ],
  });
}

async function mountSearch(fetchMock, path = "/") {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push(path);
  await router.isReady();
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return { wrapper: activeWrapper, router };
}

function buttonByText(wrapper, text) {
  const button = wrapper.findAll("button").find((candidate) => candidate.text().includes(text));
  if (!button) throw new Error(`Missing button: ${text}`);
  return button;
}

describe("SearchPage route", () => {
  it("rejects an empty query without issuing a search request", async () => {
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path === "/api/config") return jsonResponse(configSnapshot());
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountSearch(fetchMock);

    await buttonByText(wrapper, "搜索").trigger("click");
    await flushPromises();

    expect(wrapper.get("#err").text()).toBe("请输入搜索词");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("keeps Douban source, query, results, and page after media detail Back", async () => {
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path.startsWith("/api/douban/search?")) {
        const page = Number(new URL(path, "http://local").searchParams.get("page"));
        return jsonResponse({
          items: [{ id: `douban-${page}`, title: `豆瓣第 ${page} 页`, source: "douban" }],
          page,
          page_size: 20,
          has_more: page < 2,
        });
      }
      if (path === "/api/tmdb/movie/42") {
        return jsonResponse({ id: 42, title: "详情", original_title: "" });
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountSearch(fetchMock);
    const input = wrapper.get('input[type="search"]');

    await buttonByText(wrapper, "豆瓣").trigger("click");
    await input.setValue("电影");
    await input.trigger("keydown.enter");
    await flushPromises();
    expect(router.currentRoute.value.query).toEqual({ source: "douban", q: "电影" });
    await buttonByText(wrapper, "下一页").trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.query).toEqual({
      source: "douban",
      q: "电影",
      page: "2",
    });
    expect(wrapper.get(".media-card-search .title").text()).toBe("豆瓣第 2 页");
    expect(wrapper.get(".search-pager-status").text()).toBe("第 2 页 · 21-21");
    expect(wrapper.get(".media-card-search").attributes()).toMatchObject({
      role: "button",
      tabindex: "0",
      "aria-label": "打开详情 豆瓣第 2 页",
    });

    await router.push("/detail/movie/42");
    await flushPromises();
    expect(wrapper.get(".d-head h3").text()).toBe("详情");
    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("main"));
    await flushPromises();

    expect(wrapper.get('input[type="search"]').element.value).toBe("电影");
    expect(buttonByText(wrapper, "豆瓣").classes()).toContain("is-active");
    expect(wrapper.get(".media-card-search .title").text()).toBe("豆瓣第 2 页");
    expect(wrapper.get(".search-pager-status").text()).toBe("第 2 页 · 21-21");
    expect(router.currentRoute.value.query).toEqual({
      source: "douban",
      q: "电影",
      page: "2",
    });
  });

  it("hydrates a shareable search deep link and keeps cached results on a detail round trip", async () => {
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path.startsWith("/api/douban/search?")) {
        return jsonResponse({
          items: [{ id: "douban-2", title: "深链结果", source: "douban" }],
          page: 2,
          page_size: 20,
          has_more: false,
        });
      }
      if (path === "/api/tmdb/movie/42") {
        return jsonResponse({ id: 42, title: "详情", original_title: "" });
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountSearch(
      fetchMock,
      "/?source=douban&q=%E7%94%B5%E5%BD%B1&page=2",
    );

    expect(wrapper.get('input[type="search"]').element.value).toBe("电影");
    expect(buttonByText(wrapper, "豆瓣").attributes("aria-pressed")).toBe("true");
    expect(wrapper.get(".media-card-search .title").text()).toBe("深链结果");
    expect(wrapper.get(".search-pager-status").text()).toBe("第 2 页 · 21-21");

    await router.push("/detail/movie/42");
    await flushPromises();
    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("main"));
    await flushPromises();

    expect(wrapper.get(".media-card-search .title").text()).toBe("深链结果");
    expect(
      fetchMock.mock.calls
        .map(([input]) => String(input))
        .filter((path) => path.startsWith("/api/douban/search?")),
    ).toHaveLength(1);
  });

  it("gives search result cards a visible-focus target and keyboard activation", async () => {
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/search?q=movie") {
        return jsonResponse({ movies: [{ id: 42, title: "键盘电影" }], tv: [] });
      }
      if (path === "/api/tmdb/movie/42") {
        return jsonResponse({ id: 42, title: "键盘电影", original_title: "" });
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountSearch(fetchMock);
    const input = wrapper.get('input[type="search"]');
    await input.setValue("movie");
    await input.trigger("keydown.enter");
    await flushPromises();

    const card = wrapper.get(".media-card-search");
    card.element.focus();
    expect(document.activeElement).toBe(card.element);
    await card.trigger("keydown", { key: " " });
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("media-detail"));
    await flushPromises();
    expect(wrapper.get(".d-head h3").text()).toBe("键盘电影");
  });
});
