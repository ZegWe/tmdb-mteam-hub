import { describe, expect, it, vi } from "vitest";
import { createMediaDetailStore } from "../features/media-detail/store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function createTransport(overrides = {}) {
  return {
    loadDetail: vi.fn().mockResolvedValue({}),
    loadSeason: vi.fn().mockResolvedValue({ episodes: [] }),
    loadInterest: vi.fn().mockResolvedValue({}),
    saveInterest: vi.fn().mockResolvedValue({}),
    searchTorrents: vi.fn().mockResolvedValue({ items: [], page: 1, page_size: 50 }),
    loadTags: vi.fn().mockResolvedValue({ tags: [] }),
    ...overrides,
  };
}

describe("media detail store", () => {
  it("publishes primary detail while the initial M-Team request is still pending", async () => {
    const torrents = deferred();
    const transport = createTransport({
      loadDetail: vi.fn().mockResolvedValue({
        id: 101,
        title: "已加载详情",
        original_title: "Pending Torrents",
      }),
      searchTorrents: vi.fn(() => torrents.promise),
    });
    const store = createMediaDetailStore({ transport });

    await store.load({ mediaType: "movie", id: 101 });

    expect(store.primary.loading).toBe(false);
    expect(store.primary.data?.title).toBe("已加载详情");
    expect(store.mteam.activeSource).toBe("keyword");
    expect(store.mteam.loading).toBe(true);

    torrents.resolve({ items: [], page: 1, page_size: 50 });
    await vi.waitFor(() => expect(store.mteam.loading).toBe(false));
    store.dispose();
  });

  it("publishes primary detail before starting non-blocking Douban interest work", async () => {
    const interest = deferred();
    const tags = deferred();
    const callOrder = [];
    let store;
    const transport = createTransport({
      loadDetail: vi.fn().mockResolvedValue({
        id: 102,
        title: "主详情先完成",
        original_title: "Primary First",
        douban_id: "10200",
      }),
      loadTags: vi.fn(() => {
        expect(store.primary.loading).toBe(false);
        callOrder.push("tags");
        return tags.promise;
      }),
      loadInterest: vi.fn(() => {
        expect(store.primary.loading).toBe(false);
        callOrder.push("interest");
        return interest.promise;
      }),
      searchTorrents: vi.fn().mockImplementation(() => {
        callOrder.push("mteam");
        return Promise.resolve({ items: [], page: 1, page_size: 50 });
      }),
    });
    store = createMediaDetailStore({ transport });

    await store.load({ mediaType: "movie", id: 102 });

    expect(store.primary.loading).toBe(false);
    expect(store.primary.data?.title).toBe("主详情先完成");
    expect(store.interest.loading).toBe(true);
    expect(store.interest.tagHistoryLoading).toBe(true);
    expect(callOrder).toEqual(["tags", "interest", "mteam"]);

    tags.resolve({ tags: [] });
    interest.resolve({});
    await vi.waitFor(() => {
      expect(store.interest.loading).toBe(false);
      expect(store.interest.tagHistoryLoading).toBe(false);
    });
    store.dispose();
  });

  it("keeps the newest primary route when an older request resolves last", async () => {
    const older = deferred();
    const newer = deferred();
    const transport = createTransport({
      loadDetail: vi.fn((mediaType, id) => (String(id) === "1" ? older.promise : newer.promise)),
    });
    const store = createMediaDetailStore({ transport });

    const olderLoad = store.load({ mediaType: "movie", id: 1 });
    const newerLoad = store.load({ mediaType: "movie", id: 2 });
    newer.resolve({
      id: 2,
      title: "新详情",
      original_title: "Newest Detail",
      douban_id: "200",
    });
    await newerLoad;

    expect(store.primary.data?.title).toBe("新详情");
    expect(transport.loadTags).toHaveBeenCalledOnce();
    expect(transport.loadInterest).toHaveBeenCalledOnce();
    expect(transport.loadInterest).toHaveBeenCalledWith("200", {
      signal: expect.any(AbortSignal),
    });
    expect(transport.searchTorrents).toHaveBeenCalledOnce();

    older.resolve({
      id: 1,
      title: "旧详情",
      original_title: "Stale Detail",
      douban_id: "100",
    });
    await olderLoad;

    expect(store.primary.data?.title).toBe("新详情");
    expect(store.primary.numericId).toBe("2");
    expect(transport.loadTags).toHaveBeenCalledOnce();
    expect(transport.loadInterest).toHaveBeenCalledOnce();
    expect(transport.searchTorrents).toHaveBeenCalledOnce();
    store.dispose();
  });

  it("preserves the composed season model and load return contract", async () => {
    const season = { episodes: [{ episode_number: 1, name: "第一集" }] };
    const transport = createTransport({
      loadDetail: vi.fn().mockResolvedValue({
        id: 77,
        title: "剧集",
        original_title: "Series",
      }),
      loadSeason: vi.fn().mockResolvedValue(season),
    });
    const store = createMediaDetailStore({ transport });
    await store.load({ mediaType: "tv", id: 77 });

    await expect(store.loadSeason("1")).resolves.toBe(season);

    expect(transport.loadSeason).toHaveBeenCalledWith("77", 1, {
      signal: expect.any(AbortSignal),
    });
    expect(store.model.value.seasonEpisodes[1]).toEqual(season.episodes);
    expect(store.model.value.seasonLoading[1]).toBe(false);
    expect(store.model.value.seasonErrors[1]).toBe("");
    await expect(store.loadSeason(1)).resolves.toBeNull();
    expect(transport.loadSeason).toHaveBeenCalledOnce();
    store.dispose();
  });

  it("keeps the active M-Team source when an older tab request resolves last", async () => {
    const douban = deferred();
    const keyword = deferred();
    const transport = createTransport({
      loadDetail: vi.fn().mockResolvedValue({
        id: 88,
        title: "多源电影",
        original_title: "Original Title",
        imdb_id: "tt0000088",
        douban_id: "8800",
      }),
      searchTorrents: vi.fn(({ source }) => {
        if (source === "imdb") return Promise.resolve({ items: [], page: 1, page_size: 50 });
        if (source === "douban") return douban.promise;
        if (source === "keyword") return keyword.promise;
        throw new Error(`Unexpected source: ${source}`);
      }),
    });
    const store = createMediaDetailStore({ transport });
    await store.load({ mediaType: "movie", id: 88 });
    await vi.waitFor(() => expect(store.mteam.loading).toBe(false));

    const olderRequest = store.selectTorrentSource("douban");
    const newerRequest = store.selectTorrentSource("keyword");
    keyword.resolve({ items: [{ id: "new", name: "新种子" }], page: 1, page_size: 50 });
    await newerRequest;

    expect(store.mteam.activeSource).toBe("keyword");
    expect(store.mteam.rows).toEqual([{ id: "new", name: "新种子" }]);

    douban.resolve({ items: [{ id: "old", name: "旧种子" }], page: 1, page_size: 50 });
    await olderRequest;

    expect(store.mteam.activeSource).toBe("keyword");
    expect(store.mteam.rows).toEqual([{ id: "new", name: "新种子" }]);
    store.dispose();
  });

  it("isolates optional panel failures from a successful primary detail", async () => {
    const transport = createTransport({
      loadDetail: vi.fn().mockResolvedValue({
        id: 9,
        title: "主详情可用",
        original_title: "Optional Failures",
        douban_id: "900",
      }),
      loadSeason: vi.fn().mockRejectedValue(new Error("分集失败")),
      loadInterest: vi.fn().mockRejectedValue(new Error("豆瓣状态失败")),
      loadTags: vi.fn().mockRejectedValue(new Error("标签失败")),
      searchTorrents: vi.fn().mockRejectedValue(new Error("M-Team 失败")),
    });
    const store = createMediaDetailStore({ transport });

    await store.load({ mediaType: "tv", id: 9 });
    await store.loadSeason(1);
    await vi.waitFor(() => {
      expect(store.interest.error).toBe("豆瓣状态失败");
      expect(store.mteam.error).toBe("M-Team 失败");
    });

    expect(store.primary.loading).toBe(false);
    expect(store.primary.error).toBe("");
    expect(store.primary.data?.title).toBe("主详情可用");
    expect(store.model.value.seasonErrors[1]).toBe("分集失败");
    expect(store.model.value.interest.tagHistoryError).toBe("标签失败");
    store.dispose();
  });
});
