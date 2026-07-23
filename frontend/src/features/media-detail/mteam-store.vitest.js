import { describe, expect, it, vi } from "vitest";
import { createMteamTorrentStore } from "./mteam-store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function mediaContext(overrides = {}) {
  return {
    mediaType: "movie",
    doubanId: "1295644",
    data: {
      title: "这个杀手不太冷",
      original_title: "Léon",
      imdb_id: "tt0110413",
    },
    ...overrides,
  };
}

function createStore(
  searchTorrents = vi.fn().mockResolvedValue({ items: [], page: 1, page_size: 50 }),
) {
  return {
    searchTorrents,
    store: createMteamTorrentStore({ transport: { searchTorrents } }),
  };
}

describe("M-Team torrent store", () => {
  it("searches the selected TV season and annotates safe push coverage", async () => {
    const searchTorrents = vi.fn().mockResolvedValue({
      items: [
        { id: "episode", name: "Series.S02E03.1080p" },
        { id: "range", name: "Series.S02E04-E06.1080p" },
        { id: "pack", name: "Series.S02.Complete.1080p" },
        { id: "unknown", name: "Series.1080p" },
      ],
    });
    const { store } = createStore(searchTorrents);

    await store.initialize(
      mediaContext({
        mediaType: "tv",
        doubanId: "",
        data: {
          original_title: "Series",
          seasons: [
            { season_number: 0, episode_count: 2 },
            { season_number: 1, episode_count: 8 },
            { season_number: 2, episode_count: 10 },
            { season_number: 3, episode_count: 0 },
          ],
        },
      }),
    );

    expect(store.state.seasonNumber).toBe(2);
    expect(store.state.sources[0]).toEqual({
      source: "tv_season",
      label: "第 2 季",
      params: { source: "keyword", keyword: "Series S02" },
    });
    expect(searchTorrents).toHaveBeenCalledWith(
      { source: "keyword", keyword: "Series S02" },
      { signal: expect.any(AbortSignal) },
    );
    expect(store.state.rows.map((row) => [row.tv_match.kind, row.tv_match.compatible])).toEqual([
      ["episode", true],
      ["partial_pack", true],
      ["season_pack", true],
      ["unknown", false],
    ]);

    await store.selectSeason(1);
    expect(searchTorrents).toHaveBeenLastCalledWith(
      { source: "keyword", keyword: "Series S01" },
      { signal: expect.any(AbortSignal) },
    );
  });

  it("constructs sources in IMDb, Douban, keyword order from explicit media context", async () => {
    const row = { id: "imdb-result", name: "IMDb result" };
    const { searchTorrents, store } = createStore(
      vi.fn().mockResolvedValue({ items: [row], page: 1, page_size: 50 }),
    );

    await store.initialize(mediaContext());

    expect(store.state.sources).toEqual([
      { source: "imdb", label: "IMDb", params: { imdb_id: "tt0110413" } },
      { source: "douban", label: "豆瓣 ID", params: { douban_id: "1295644" } },
      { source: "keyword", label: "原标题", params: { keyword: "Léon" } },
    ]);
    expect(store.state.activeSource).toBe("imdb");
    expect(store.state.rows).toEqual([row]);
    expect(searchTorrents).toHaveBeenCalledWith(
      { source: "imdb", imdb_id: "tt0110413" },
      { signal: expect.any(AbortSignal) },
    );
  });

  it.each([
    ["stable items", { items: [{ id: "stable" }], page: 1, page_size: 50 }, [{ id: "stable" }]],
    ["unknown response", { data: [{ id: "ignored" }] }, []],
  ])("accepts %s response rows", async (_label, response, expectedRows) => {
    const { store } = createStore(vi.fn().mockResolvedValue(response));

    await store.initialize(mediaContext());

    expect(store.state.rows).toEqual(expectedRows);
  });

  it("serves normalized rows from the per-context cache", async () => {
    const imdbRows = [{ id: "imdb" }];
    const keywordRows = [{ id: "keyword" }];
    const { searchTorrents, store } = createStore(
      vi
        .fn()
        .mockResolvedValueOnce({ items: imdbRows, page: 1, page_size: 50 })
        .mockResolvedValueOnce({ items: keywordRows, page: 1, page_size: 50 }),
    );
    await store.initialize(mediaContext({ doubanId: "" }));

    await store.select("keyword");
    await store.select("imdb");

    expect(searchTorrents).toHaveBeenCalledTimes(2);
    expect(store.state.activeSource).toBe("imdb");
    expect(store.state.rows).toEqual(imdbRows);
  });

  it("keeps the latest source when an older request resolves last", async () => {
    const douban = deferred();
    const keyword = deferred();
    const { store } = createStore(
      vi.fn(({ source }) => {
        if (source === "imdb") return Promise.resolve({ items: [], page: 1, page_size: 50 });
        if (source === "douban") return douban.promise;
        if (source === "keyword") return keyword.promise;
        throw new Error(`Unexpected source: ${source}`);
      }),
    );
    await store.initialize(mediaContext());

    const olderRequest = store.select("douban");
    const latestRequest = store.select("keyword");
    keyword.resolve({ items: [{ id: "latest" }], page: 1, page_size: 50 });
    await latestRequest;

    douban.resolve({ items: [{ id: "stale" }], page: 1, page_size: 50 });
    await olderRequest;

    expect(store.state.activeSource).toBe("keyword");
    expect(store.state.rows).toEqual([{ id: "latest" }]);
    expect(store.state.error).toBe("");
  });

  it("contains transport failures within the optional panel", async () => {
    const { store } = createStore(vi.fn().mockRejectedValue(new Error("M-Team unavailable")));

    await expect(store.initialize(mediaContext())).resolves.toBeNull();

    expect(store.state.loading).toBe(false);
    expect(store.state.rows).toEqual([]);
    expect(store.state.error).toBe("M-Team unavailable");
  });

  it("reset cancels pending work, clears cache and remains reusable", async () => {
    const pending = deferred();
    const searchTorrents = vi
      .fn()
      .mockImplementationOnce((_params, { signal }) => {
        expect(signal.aborted).toBe(false);
        return pending.promise;
      })
      .mockResolvedValueOnce({ items: [{ id: "fresh" }], page: 1, page_size: 50 });
    const { store } = createStore(searchTorrents);
    const request = store.initialize(mediaContext());

    store.reset();
    expect(store.state).toMatchObject({
      sources: [],
      activeSource: "",
      rows: [],
      loading: false,
      error: "",
    });

    pending.resolve({ items: [{ id: "stale" }], page: 1, page_size: 50 });
    await request;
    await store.initialize(mediaContext({ doubanId: "", data: { original_title: "Fresh" } }));

    expect(searchTorrents).toHaveBeenCalledTimes(2);
    expect(store.state.rows).toEqual([{ id: "fresh" }]);
  });

  it("dispose cancels pending work, clears state and prevents later initialization", async () => {
    const pending = deferred();
    let signal;
    const searchTorrents = vi.fn((_params, options) => {
      signal = options.signal;
      return pending.promise;
    });
    const { store } = createStore(searchTorrents);
    const request = store.initialize(mediaContext());

    store.dispose();

    expect(signal.aborted).toBe(true);
    expect(store.state).toMatchObject({
      sources: [],
      activeSource: "",
      rows: [],
      loading: false,
      error: "",
    });
    pending.resolve({ items: [{ id: "stale" }], page: 1, page_size: 50 });
    await request;
    expect(store.initialize(mediaContext())).toBeNull();
    expect(searchTorrents).toHaveBeenCalledOnce();
  });
});
