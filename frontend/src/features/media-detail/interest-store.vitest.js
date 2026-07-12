import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { createDoubanInterestStore } from "./interest-store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function categories() {
  return ref([
    { name: "电影", wanted_tag: "movie" },
    { name: "剧集", wanted_tag: "tv" },
  ]);
}

function createTransport(overrides = {}) {
  return {
    loadInterest: vi.fn().mockResolvedValue({}),
    saveInterest: vi.fn().mockResolvedValue({ ok: true }),
    loadTags: vi.fn().mockResolvedValue({ tags: [] }),
    ...overrides,
  };
}

function createStore(overrides = {}) {
  const transport = createTransport(overrides);
  return {
    store: createDoubanInterestStore({
      subscriptionCategories: categories(),
      transport,
    }),
    transport,
  };
}

describe("Douban interest store", () => {
  it("initializes route state and derives the existing read-only model", () => {
    const { store } = createStore();

    store.initialize({
      doubanId: "1295644",
      doubanTags: " movie   classic ",
      data: { user_interest: "wish", user_rating: 4, tags: ["ignored"] },
    });

    expect(store.state.mark).toEqual({
      interest: "wish",
      rating: "4",
      tags: "movie classic",
      category: "movie",
    });
    expect(store.state.status).toBe("已想看");
    expect(store.model.value).toMatchObject({
      ratingLabel: "4 星",
      saveDisabled: false,
      categoryLabel: "电影 · movie",
      categories: [
        { name: "电影", wanted_tag: "movie" },
        { name: "剧集", wanted_tag: "tv" },
      ],
    });
  });

  it("owns mark updates and de-duplicates applied tag suggestions", () => {
    const { store } = createStore();
    store.initialize({ doubanId: "1", data: {} });
    store.state.status = "旧状态";
    store.state.error = "旧错误";

    store.setInterest("collect");
    store.updateRating(5);
    store.updateTags("classic movie");
    store.applyTagSuggestion(" movie ");
    store.applyTagSuggestion("director-cut");

    expect(store.state.status).toBe("");
    expect(store.state.error).toBe("");
    expect(store.state.mark).toEqual({
      interest: "collect",
      rating: "5",
      tags: "classic movie director-cut",
      category: "",
    });

    store.setInterest("wish");
    store.applyTagSuggestion("tv");
    expect(store.state.mark.category).toBe("tv");
  });

  it("cancels the old hydrate and ignores its response after a route change", async () => {
    const older = deferred();
    const newer = deferred();
    let olderSignal;
    const { store } = createStore({
      loadInterest: vi.fn((doubanId, { signal }) => {
        if (doubanId === "old") {
          olderSignal = signal;
          return older.promise;
        }
        if (doubanId === "new") return newer.promise;
        throw new Error(`Unexpected Douban ID: ${doubanId}`);
      }),
    });
    store.initialize({ doubanId: "old", data: { user_interest: "wish" } });
    const olderRequest = store.hydrate();

    store.initialize({ doubanId: "new", data: {} });
    expect(olderSignal.aborted).toBe(true);
    const newerRequest = store.hydrate();
    newer.resolve({ user_interest: "collect", user_rating: 5 });
    await newerRequest;

    older.resolve({ user_interest: "wish", user_rating: 1 });
    await olderRequest;

    expect(store.state.loading).toBe(false);
    expect(store.state.mark.interest).toBe("collect");
    expect(store.state.mark.rating).toBe("5");
    expect(store.state.status).toBe("已看过");
  });

  it("contains hydrate failures within the optional panel", async () => {
    const { store } = createStore({
      loadInterest: vi.fn().mockRejectedValue(new Error("豆瓣状态失败")),
    });
    store.initialize({ doubanId: "1", data: {} });

    await expect(store.hydrate()).resolves.toBeNull();

    expect(store.state.loading).toBe(false);
    expect(store.state.error).toBe("豆瓣状态失败");
    expect(store.state.status).toBe("");
  });

  it("de-duplicates tag-history requests and preserves the cache across route resets", async () => {
    const tags = deferred();
    const { store, transport } = createStore({
      loadTags: vi.fn(() => tags.promise),
    });
    store.initialize({ doubanId: "1", data: {} });

    const firstRequest = store.loadTagHistory();
    const duplicateRequest = store.loadTagHistory();
    expect(transport.loadTags).toHaveBeenCalledOnce();
    expect(store.state.tagHistoryLoading).toBe(true);

    tags.resolve({ tags: ["movie", "", "classic"] });
    await Promise.all([firstRequest, duplicateRequest]);
    store.reset();

    expect(store.state.tagHistory).toEqual(["movie", "classic"]);
    await expect(store.loadTagHistory()).resolves.toEqual(["movie", "classic"]);
    expect(transport.loadTags).toHaveBeenCalledOnce();
  });

  it("isolates tag-history refresh failures and keeps existing rows", async () => {
    const { store } = createStore({
      loadTags: vi.fn().mockRejectedValue(new Error("标签失败")),
    });
    store.state.tagHistory = ["cached"];

    await expect(store.loadTagHistory(true)).resolves.toEqual(["cached"]);

    expect(store.state.tagHistoryLoading).toBe(false);
    expect(store.state.tagHistory).toEqual(["cached"]);
    expect(store.state.tagHistoryError).toBe("标签失败");
  });

  it("normalizes save payloads and moves configured tags to the history front", async () => {
    const { store, transport } = createStore();
    store.initialize({ doubanId: "42", data: {} });
    store.state.tagHistory = ["classic", "movie"];
    store.setInterest("collect");
    store.updateRating("4");
    store.updateTags(" movie   unconfigured ");

    await store.save();

    expect(transport.saveInterest).toHaveBeenCalledWith("42", {
      interest: "collect",
      rating: 4,
      tags: "movie unconfigured",
    });
    expect(store.state.status).toBe("已标记看过");
    expect(store.state.tagHistory).toEqual(["movie", "classic"]);
    expect(store.state.saving).toBe(false);
  });

  it("keeps validation errors in the current interest panel", async () => {
    const { store, transport } = createStore();
    store.initialize({ doubanId: "42", data: {} });
    store.setInterest("wish");

    await expect(store.save()).rejects.toThrow("请选择订阅分类");

    expect(transport.saveInterest).not.toHaveBeenCalled();
    expect(store.state.error).toBe("请选择订阅分类");
    expect(store.state.status).toBe("请选择订阅分类");
    expect(store.state.saving).toBe(false);
  });

  it("does not let an older route save overwrite a newer save", async () => {
    const older = deferred();
    const newer = deferred();
    const { store } = createStore({
      saveInterest: vi.fn((doubanId) => (doubanId === "old" ? older.promise : newer.promise)),
    });
    store.initialize({ doubanId: "old", doubanTags: "movie", data: {} });
    store.setInterest("wish");
    const olderSave = store.save();

    store.initialize({ doubanId: "new", doubanTags: "tv", data: {} });
    store.setInterest("wish");
    const newerSave = store.save();
    older.resolve({ ok: true });
    await olderSave;

    expect(store.state.saving).toBe(true);
    expect(store.state.status).toBe("保存中…");
    expect(store.state.tagHistory).toEqual([]);

    newer.resolve({ ok: true });
    await newerSave;

    expect(store.state.saving).toBe(false);
    expect(store.state.status).toBe("已标记想看");
    expect(store.state.tagHistory).toEqual(["tv"]);
  });

  it("dispose cancels hydrate work, preserves tag history and blocks later network work", async () => {
    const pending = deferred();
    let signal;
    const { store, transport } = createStore({
      loadInterest: vi.fn((_doubanId, options) => {
        signal = options.signal;
        return pending.promise;
      }),
    });
    store.state.tagHistory = ["cached"];
    store.initialize({ doubanId: "1", data: {} });
    const request = store.hydrate();

    store.dispose();

    expect(signal.aborted).toBe(true);
    expect(store.state).toMatchObject({
      loading: false,
      saving: false,
      error: "",
      status: "",
      mark: { interest: "", rating: "", tags: "", category: "" },
      tagHistory: ["cached"],
    });
    pending.resolve({ user_interest: "collect", user_rating: 5 });
    await request;

    expect(store.initialize({ doubanId: "2", data: {} })).toBeNull();
    await expect(store.hydrate()).resolves.toBeNull();
    await expect(store.save()).resolves.toBeNull();
    expect(transport.loadInterest).toHaveBeenCalledOnce();
  });
});
