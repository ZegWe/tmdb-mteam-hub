import { describe, expect, it, vi } from "vitest";
import { createSearchStore } from "./store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function transport(overrides = {}) {
  return {
    searchTmdb: vi.fn().mockResolvedValue({ movies: [], tv: [] }),
    searchDouban: vi.fn().mockResolvedValue({ items: [], page: 1, page_size: 20, has_more: false }),
    ...overrides,
  };
}

describe("search store", () => {
  it("rejects an empty query without making a request", async () => {
    const api = transport();
    const store = createSearchStore({ transport: api });

    await expect(store.search()).rejects.toThrow("请输入搜索词");

    expect(api.searchTmdb).not.toHaveBeenCalled();
    expect(api.searchDouban).not.toHaveBeenCalled();
    expect(store.loading.value).toBe(false);
  });

  it("keeps only the latest TMDB response", async () => {
    const older = deferred();
    const newer = deferred();
    const api = transport({
      searchTmdb: vi.fn().mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise),
    });
    const store = createSearchStore({ transport: api });

    store.query.value = "older";
    const olderRequest = store.search();
    store.query.value = "newer";
    const newerRequest = store.search();
    newer.resolve({ movies: [{ id: 2, title: "新搜索" }], tv: [] });
    await newerRequest;
    older.resolve({ movies: [{ id: 1, title: "旧搜索" }], tv: [] });
    await olderRequest;

    expect(store.movies.value.map((item) => item.title)).toEqual(["新搜索"]);
    expect(store.loading.value).toBe(false);
  });

  it("owns Douban pagination and resets it when the source changes", async () => {
    const api = transport({
      searchDouban: vi.fn().mockResolvedValue({
        items: [{ id: "douban-21", title: "第二页" }],
        page: 2,
        page_size: 20,
        has_more: true,
      }),
    });
    const store = createSearchStore({ transport: api });
    store.setSource("douban");
    store.query.value = "电影";

    await store.loadDoubanPage(2);

    expect(api.searchDouban).toHaveBeenCalledWith("电影", {
      page: 2,
      pageSize: 20,
      signal: expect.any(AbortSignal),
    });
    expect(store.doubanPage).toMatchObject({ page: 2, page_size: 20, has_more: true });
    expect(store.doubanPagerText.value).toBe("第 2 页 · 21-21");

    store.setSource("tmdb");
    expect(store.doubanPage.page).toBe(1);
    expect(store.doubanPage.has_more).toBe(false);
  });
});
