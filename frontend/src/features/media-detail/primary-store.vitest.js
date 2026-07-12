import { describe, expect, it, vi } from "vitest";
import { createMediaDetailPrimaryStore } from "./primary-store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("media detail primary store", () => {
  it("loads TMDB detail and publishes the derived presentation model", async () => {
    const detail = {
      id: 77,
      title: "示例剧集",
      original_title: "Example Series",
      first_air_date: "2026-07-11",
      overview: "剧情简介",
      poster_url: "https://image.tmdb.org/t/p/w500/poster.jpg",
      number_of_seasons: 2,
      imdb_id: "tt0000077",
      douban_id: "7700",
      seasons: [{ season_number: 2 }, { season_number: 0 }, { season_number: 1 }],
    };
    const transport = { loadDetail: vi.fn().mockResolvedValue(detail) };
    const store = createMediaDetailPrimaryStore({ transport });

    await expect(store.load({ mediaType: "tv", id: 77 })).resolves.toBe(detail);

    expect(transport.loadDetail).toHaveBeenCalledWith("tv", 77, {
      signal: expect.any(AbortSignal),
    });
    expect(store.state).toMatchObject({
      loading: false,
      error: "",
      mediaType: "tv",
      numericId: "77",
      doubanId: "7700",
    });
    expect(store.model.value).toMatchObject({
      title: "示例剧集",
      date: "2026-07-11",
      overview: "剧情简介",
      poster: "https://image.tmdb.org/t/p/w500/poster.jpg",
      seasons: [{ season_number: 0 }, { season_number: 1 }, { season_number: 2 }],
      externalLinks: [
        { href: "https://www.themoviedb.org/tv/77", label: "TMDB · 77" },
        { href: "https://www.imdb.com/title/tt0000077/", label: "IMDb · tt0000077" },
        { href: "https://movie.douban.com/subject/7700/", label: "豆瓣 · 7700" },
      ],
    });
    expect(store.model.value.metaRows).toContainEqual({ label: "季数", value: "2 季" });
    expect(store.pageTitle.value).toBe("示例剧集");
    expect(store.pageSubtitle.value).toBe("剧集资料、分集与 M-Team 种子");
    store.dispose();
  });

  it("derives Douban identity, fallback fields, and external links without a TMDB link", async () => {
    const detail = {
      subject_id: 1295644,
      title: "这个杀手不太冷",
      date_published: "1994-09-14",
      summary: "里昂与玛蒂尔达的故事",
      image: "https://img.example/poster.jpg",
      imdb_id: "0110413",
      rating: { value: 9.4, count: 2488092 },
    };
    const transport = { loadDetail: vi.fn().mockResolvedValue(detail) };
    const store = createMediaDetailPrimaryStore({ transport });

    await store.load({ mediaType: "douban", id: "1295644" });

    expect(store.state.doubanId).toBe("1295644");
    expect(store.model.value).toMatchObject({
      title: "这个杀手不太冷",
      date: "1994-09-14",
      overview: "里昂与玛蒂尔达的故事",
      poster: "https://img.example/poster.jpg",
      externalLinks: [
        { href: "https://www.imdb.com/title/tt0110413/", label: "IMDb · 0110413" },
        {
          href: "https://movie.douban.com/subject/1295644/",
          label: "豆瓣 · 1295644",
        },
      ],
    });
    expect(store.model.value.externalLinks).not.toContainEqual(
      expect.objectContaining({ label: expect.stringContaining("TMDB") }),
    );
    expect(store.model.value.metaRows).toContainEqual({
      label: "评分",
      value: expect.stringContaining("9.4 / 10"),
    });
    expect(store.pageSubtitle.value).toBe("豆瓣资料、标记与 M-Team 种子");
    store.dispose();
  });

  it("keeps the newest route when an older primary request resolves last", async () => {
    const older = deferred();
    const newer = deferred();
    let olderSignal;
    const transport = {
      loadDetail: vi.fn((mediaType, id, { signal }) => {
        if (String(id) === "1") {
          olderSignal = signal;
          return older.promise;
        }
        return newer.promise;
      }),
    };
    const store = createMediaDetailPrimaryStore({ transport });

    const olderLoad = store.load({ mediaType: "movie", id: 1 });
    const newerLoad = store.load({ mediaType: "movie", id: 2 });
    expect(olderSignal.aborted).toBe(true);
    newer.resolve({ id: 2, title: "新详情" });
    await expect(newerLoad).resolves.toMatchObject({ id: 2 });

    older.resolve({ id: 1, title: "旧详情" });
    await expect(olderLoad).resolves.toBeNull();

    expect(store.state.data?.title).toBe("新详情");
    expect(store.state.numericId).toBe("2");
    expect(store.pageTitle.value).toBe("新详情");
    store.dispose();
  });

  it("owns error/reset state and ignores a pending result after disposal", async () => {
    const pending = deferred();
    let pendingSignal;
    const transport = {
      loadDetail: vi
        .fn()
        .mockRejectedValueOnce(new Error("主详情失败"))
        .mockImplementationOnce((_mediaType, _id, { signal }) => {
          pendingSignal = signal;
          return pending.promise;
        }),
    };
    const store = createMediaDetailPrimaryStore({ transport });

    await expect(store.load({ mediaType: "movie", id: 3 })).rejects.toThrow("主详情失败");
    expect(store.state).toMatchObject({ loading: false, error: "主详情失败", data: null });

    const load = store.load({ mediaType: "movie", id: 4 });
    expect(store.state).toMatchObject({ loading: true, error: "", numericId: "4" });
    store.dispose();
    expect(pendingSignal.aborted).toBe(true);
    expect(store.state).toMatchObject({
      loading: false,
      error: "",
      mediaType: "",
      numericId: "",
      doubanId: "",
      data: null,
    });

    pending.resolve({ id: 4, title: "迟到详情" });
    await expect(load).resolves.toBeNull();
    expect(store.state.data).toBeNull();
  });
});
