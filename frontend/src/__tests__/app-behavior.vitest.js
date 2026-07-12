import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";
import { createAppRoutes } from "../app/routes.js";
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
    routes: createAppRoutes(),
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
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return {
    promise,
    resolveJson(value, init) {
      resolve(jsonResponse(value, init));
    },
    reject,
  };
}

function subscriptionSummary(id = "subject-7") {
  return {
    subject_id: id,
    revision: 1,
    active: true,
    inactive_at: null,
    last_seen_snapshot_id: "snapshot-1",
    media_kind: "movie",
    schedulable: true,
    blocked_reason: null,
    lifecycle_state: "queued",
    execution_state: "idle",
    next_attempt_at: null,
    retry_count: 0,
    max_retries: 3,
    retry_blocked: false,
    force_eligible_once: false,
    updated_at: 100,
    title: "深链订阅",
    release_year: null,
    poster_url: "",
    category_text: null,
    douban_sort_time: null,
    attention_tags: [],
  };
}

function subscriptionDetail(summary) {
  return {
    summary,
    source: {},
    observation: {},
    issues: [],
    skip_reason: null,
    candidates: [],
    tv: null,
    downloads: [],
    links: [],
  };
}

async function mountAt(path, fetchMock, { flush = true } = {}) {
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

function configOr(fetchHandler) {
  return vi.fn((input, init) => {
    const path = String(input);
    if (path === "/api/config") return Promise.resolve(jsonResponse(CONFIG_RESPONSE));
    return fetchHandler(path, init);
  });
}

describe("current App route behavior", () => {
  it("restores a media detail deep link with memory history", async () => {
    const fetchMock = configOr((path) => {
      if (path === "/api/tmdb/movie/101") {
        return Promise.resolve(jsonResponse({ id: 101, title: "深链电影", original_title: "" }));
      }
      throw new Error(`Unexpected request: ${path}`);
    });

    const { wrapper, router } = await mountAt("/detail/movie/101", fetchMock);

    expect(router.currentRoute.value.name).toBe("media-detail");
    expect(wrapper.get(".d-head h3").text()).toBe("深链电影");
    expect(wrapper.get(".nav-item.is-active").text()).toBe("主功能");
  });

  it("restores a subscription detail deep link", async () => {
    const summary = subscriptionSummary();
    const fetchMock = configOr((path) => {
      if (path === "/api/subscriptions/wanted?limit=100") {
        return Promise.resolve(jsonResponse({ items: [summary], next_cursor: null }));
      }
      if (path === "/api/subscriptions/wanted/subject-7") {
        return Promise.resolve(jsonResponse(subscriptionDetail(summary)));
      }
      throw new Error(`Unexpected request: ${path}`);
    });

    const { wrapper, router } = await mountAt("/subscriptions/subject-7", fetchMock);

    expect(router.currentRoute.value.name).toBe("subscription-detail");
    expect(wrapper.get(".subscription-detail h3").text()).toBe("深链订阅");
    expect(wrapper.get(".nav-item.is-active").text()).toBe("订阅");
  });

  it("renders not-found with no selected primary navigation item", async () => {
    const fetchMock = configOr((path) => {
      throw new Error(`Unexpected request: ${path}`);
    });

    const { wrapper, router } = await mountAt("/missing/path", fetchMock);

    expect(router.currentRoute.value.name).toBe("not-found");
    expect(router.currentRoute.value.meta.navPage).toBe("");
    expect(wrapper.get("#page-not-found h1").text()).toBe("页面不存在");
    expect(wrapper.find(".nav-item.is-active").exists()).toBe(false);
  });

  it("navigates from a result card to detail and browser Back preserves the result list", async () => {
    const fetchMock = configOr((path) => {
      if (path === "/api/search?q=card") {
        return Promise.resolve(
          jsonResponse({ movies: [{ id: 42, title: "卡片电影", media_type: "movie" }], tv: [] }),
        );
      }
      if (path === "/api/tmdb/movie/42") {
        return Promise.resolve(jsonResponse({ id: 42, title: "卡片电影详情", original_title: "" }));
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountAt("/", fetchMock);
    const input = wrapper.get('input[type="search"]');

    await input.setValue("card");
    await input.trigger("keydown.enter");
    await flushPromises();
    await wrapper.get(".media-card-search").trigger("click");
    await flushPromises();

    expect(router.currentRoute.value).toMatchObject({
      name: "media-detail",
      params: { mediaType: "movie", id: "42" },
    });
    expect(wrapper.get(".d-head h3").text()).toBe("卡片电影详情");

    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("main"));
    expect(wrapper.get(".media-card-search .title").text()).toBe("卡片电影");
  });
});

describe("current App latest response behavior", () => {
  it("does not let an older media detail response replace a newer route", async () => {
    const older = deferred();
    const newer = deferred();
    const fetchMock = configOr((path) => {
      if (path === "/api/tmdb/movie/1") return older.promise;
      if (path === "/api/tmdb/movie/2") return newer.promise;
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper, router } = await mountAt("/detail/movie/1", fetchMock);

    await vi.waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith("/api/tmdb/movie/1", expect.anything()),
    );
    await router.push("/detail/movie/2");
    await vi.waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith("/api/tmdb/movie/2", expect.anything()),
    );

    newer.resolveJson({ id: 2, title: "新详情", original_title: "" });
    await flushPromises();
    expect(wrapper.get(".d-head h3").text()).toBe("新详情");

    older.resolveJson({ id: 1, title: "旧详情", original_title: "" });
    await flushPromises();
    expect(wrapper.get(".d-head h3").text()).toBe("新详情");
  });

  it("does not let an older search response replace a newer query", async () => {
    const older = deferred();
    const newer = deferred();
    const fetchMock = configOr((path) => {
      if (path === "/api/search?q=older") return older.promise;
      if (path === "/api/search?q=newer") return newer.promise;
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountAt("/", fetchMock);
    const input = wrapper.get('input[type="search"]');

    await input.setValue("older");
    await input.trigger("keydown.enter");
    await input.setValue("newer");
    await input.trigger("keydown.enter");

    newer.resolveJson({ movies: [{ id: 2, title: "新搜索", media_type: "movie" }], tv: [] });
    await flushPromises();
    expect(wrapper.get(".media-card-search .title").text()).toBe("新搜索");

    older.resolveJson({ movies: [{ id: 1, title: "旧搜索", media_type: "movie" }], tv: [] });
    await flushPromises();
    expect(wrapper.get(".media-card-search .title").text()).toBe("新搜索");
  });

  it("does not let an older M-Team tab response replace the active source", async () => {
    const douban = deferred();
    const keyword = deferred();
    const fetchMock = configOr((path) => {
      if (path === "/api/tmdb/movie/88") {
        return Promise.resolve(
          jsonResponse({
            id: 88,
            title: "多源电影",
            original_title: "Original Title",
            imdb_id: "tt0000088",
            douban_id: "8800",
          }),
        );
      }
      if (path.includes("/api/mteam/torrents?source=imdb")) {
        return Promise.resolve(jsonResponse({ items: [], page: 1, page_size: 50 }));
      }
      if (path.includes("/api/mteam/torrents?source=douban")) return douban.promise;
      if (path.includes("/api/mteam/torrents?source=keyword")) return keyword.promise;
      if (path === "/api/douban/subject/8800") {
        return Promise.resolve(jsonResponse({ subject_id: "8800" }));
      }
      if (path.startsWith("/api/douban/tags?")) return Promise.resolve(jsonResponse({ tags: [] }));
      throw new Error(`Unexpected request: ${path}`);
    });
    const { wrapper } = await mountAt("/detail/movie/88", fetchMock);
    const sourceButtons = wrapper.findAll(".mteam-tablist .mteam-tab");
    const doubanButton = sourceButtons.find((button) => button.text() === "豆瓣 ID");
    const keywordButton = sourceButtons.find((button) => button.text() === "原标题");

    await doubanButton.trigger("click");
    await keywordButton.trigger("click");

    keyword.resolveJson({ items: [{ id: "new", name: "新种子" }], page: 1, page_size: 50 });
    await flushPromises();
    expect(wrapper.get(".torrent-name").text()).toBe("新种子");

    douban.resolveJson({ items: [{ id: "old", name: "旧种子" }], page: 1, page_size: 50 });
    await flushPromises();
    expect(wrapper.get(".torrent-name").text()).toBe("新种子");
  });
});

describe("current App subscription polling", () => {
  it("stops polling after leaving the subscription route tree", async () => {
    vi.useFakeTimers();
    let wantedRequests = 0;
    const fetchMock = configOr((path) => {
      if (path === "/api/subscriptions/wanted?limit=100") {
        wantedRequests += 1;
        return Promise.resolve(jsonResponse({ items: [], next_cursor: null }));
      }
      throw new Error(`Unexpected request: ${path}`);
    });
    const { router } = await mountAt("/subscriptions", fetchMock, { flush: false });
    await vi.advanceTimersByTimeAsync(0);
    const initialRequests = wantedRequests;

    await vi.advanceTimersByTimeAsync(5000);
    expect(wantedRequests).toBeGreaterThan(initialRequests);

    await router.push("/");
    await vi.advanceTimersByTimeAsync(0);
    const requestsAfterLeaving = wantedRequests;
    await vi.advanceTimersByTimeAsync(15_000);

    expect(wantedRequests).toBe(requestsAfterLeaving);
  });
});
