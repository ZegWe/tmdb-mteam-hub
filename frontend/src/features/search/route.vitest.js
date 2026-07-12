import { nextTick, reactive, ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { createSearchRouteSync, searchQueryFromState, searchStateFromQuery } from "./route.js";

function routeStore() {
  const source = ref("tmdb");
  const query = ref("");
  let resultKey = "";
  const key = (state) => JSON.stringify([state.source, state.query, state.page]);
  return {
    source,
    query,
    hydrateRouteState: vi.fn((state) => {
      source.value = state.source;
      query.value = state.query;
    }),
    hasResultsFor: vi.fn((state) => resultKey === key(state)),
    search: vi.fn(async (page) => {
      resultKey = key({ source: source.value, query: query.value, page });
      return null;
    }),
  };
}

function routerFor(route) {
  async function navigate(location) {
    route.name = location.name;
    route.query = location.query;
    await nextTick();
  }
  return { push: vi.fn(navigate) };
}

describe("search route synchronization", () => {
  it("normalizes shareable source/query/page state and omits defaults", () => {
    expect(searchStateFromQuery({ source: ["douban", "ignored"], q: " 电影 ", page: "2" })).toEqual(
      { source: "douban", query: "电影", page: 2 },
    );
    expect(searchStateFromQuery({ source: "unknown", q: ["test"], page: "999" })).toEqual({
      source: "tmdb",
      query: "test",
      page: 1,
    });
    expect(searchQueryFromState({ source: "tmdb", query: "电影", page: 7 })).toEqual({
      q: "电影",
    });
    expect(searchQueryFromState({ source: "douban", query: "电影", page: 2 })).toEqual({
      source: "douban",
      q: "电影",
      page: "2",
    });
  });

  it("hydrates a deep link and does not reload an already cached result after remount", async () => {
    const route = reactive({
      name: "main",
      query: { source: "douban", q: "电影", page: "2" },
    });
    const store = routeStore();
    const router = routerFor(route);
    const first = createSearchRouteSync({ route, router, store });
    await nextTick();

    expect(store.hydrateRouteState).toHaveBeenCalledWith({
      source: "douban",
      query: "电影",
      page: 2,
    });
    expect(store.search).toHaveBeenCalledOnce();
    expect(store.search).toHaveBeenCalledWith(2);
    first.dispose();

    store.search.mockClear();
    const second = createSearchRouteSync({ route, router, store });
    await nextTick();
    expect(store.search).not.toHaveBeenCalled();
    second.dispose();
  });

  it("publishes submitted searches and source/page changes through route query state", async () => {
    const route = reactive({ name: "main", query: {} });
    const store = routeStore();
    const router = routerFor(route);
    const sync = createSearchRouteSync({ route, router, store });
    await nextTick();

    store.query.value = "电影";
    await sync.submit();
    expect(router.push).toHaveBeenLastCalledWith({ name: "main", query: { q: "电影" } });

    await sync.selectSource("douban");
    expect(router.push).toHaveBeenLastCalledWith({
      name: "main",
      query: { source: "douban", q: "电影" },
    });

    await sync.submit(3);
    expect(router.push).toHaveBeenLastCalledWith({
      name: "main",
      query: { source: "douban", q: "电影", page: "3" },
    });
    sync.dispose();
  });
});
