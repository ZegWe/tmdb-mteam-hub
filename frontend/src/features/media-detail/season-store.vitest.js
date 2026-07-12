import { describe, expect, it, vi } from "vitest";
import { createTvSeasonStore } from "./season-store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function createStore(loadSeason = vi.fn().mockResolvedValue({ episodes: [] })) {
  return {
    loadSeason,
    store: createTvSeasonStore({ transport: { loadSeason } }),
  };
}

describe("TV season store", () => {
  it("de-duplicates the same season request and caches its return value", async () => {
    const pending = deferred();
    const response = { episodes: [{ episode_number: 1, name: "Pilot" }] };
    const { loadSeason, store } = createStore(vi.fn(() => pending.promise));
    store.initialize({ tvId: "100" });

    const firstRequest = store.load(1);
    await expect(store.load("1")).resolves.toBeNull();
    expect(loadSeason).toHaveBeenCalledOnce();
    expect(loadSeason).toHaveBeenCalledWith("100", 1, {
      signal: expect.any(AbortSignal),
    });

    pending.resolve(response);
    await expect(firstRequest).resolves.toBe(response);

    expect(store.episodes[1]).toEqual(response.episodes);
    expect(store.loading[1]).toBe(false);
    expect(store.errors[1]).toBe("");
    await expect(store.load(1)).resolves.toBeNull();
    expect(loadSeason).toHaveBeenCalledOnce();
  });

  it("allows different seasons to load concurrently without cancelling each other", async () => {
    const first = deferred();
    const second = deferred();
    const signals = new Map();
    const { store } = createStore(
      vi.fn((_tvId, seasonNumber, { signal }) => {
        signals.set(seasonNumber, signal);
        return seasonNumber === 1 ? first.promise : second.promise;
      }),
    );
    store.initialize({ tvId: "200" });

    const firstRequest = store.load(1);
    const secondRequest = store.load(2);

    expect(store.loading).toMatchObject({ 1: true, 2: true });
    expect(signals.get(1).aborted).toBe(false);
    expect(signals.get(2).aborted).toBe(false);

    second.resolve({ episodes: [{ episode_number: 2 }] });
    await secondRequest;
    expect(store.loading[1]).toBe(true);
    expect(store.loading[2]).toBe(false);
    expect(signals.get(1).aborted).toBe(false);

    first.resolve({ episodes: [{ episode_number: 1 }] });
    await firstRequest;
    expect(store.episodes).toEqual({
      1: [{ episode_number: 1 }],
      2: [{ episode_number: 2 }],
    });
  });

  it("isolates errors by season while other requests continue", async () => {
    const second = deferred();
    const { store } = createStore(
      vi.fn((_tvId, seasonNumber) => {
        if (seasonNumber === 1) return Promise.reject(new Error("第一季失败"));
        return second.promise;
      }),
    );
    store.initialize({ tvId: "300" });

    const failedRequest = store.load(1);
    const secondRequest = store.load(2);
    await expect(failedRequest).resolves.toBeNull();

    expect(store.errors[1]).toBe("第一季失败");
    expect(store.loading[1]).toBe(false);
    expect(store.loading[2]).toBe(true);

    const response = { episodes: [{ episode_number: 1 }] };
    second.resolve(response);
    await expect(secondRequest).resolves.toBe(response);
    expect(store.errors[2]).toBe("");
    expect(store.episodes[2]).toEqual(response.episodes);
  });

  it("aborts reset requests and keeps their late completion out of the new context", async () => {
    const older = deferred();
    const newer = deferred();
    let olderSignal;
    const { store } = createStore(
      vi.fn((tvId, _seasonNumber, { signal }) => {
        if (tvId === "old") {
          olderSignal = signal;
          return older.promise;
        }
        return newer.promise;
      }),
    );
    store.initialize({ tvId: "old" });
    const olderRequest = store.load(1);

    store.initialize({ tvId: "new" });
    expect(olderSignal.aborted).toBe(true);
    expect(store.episodes).toEqual({});
    expect(store.loading).toEqual({});
    expect(store.errors).toEqual({});
    const newerRequest = store.load(1);

    older.resolve({ episodes: [{ name: "stale" }] });
    await olderRequest;
    expect(store.loading[1]).toBe(true);
    expect(store.episodes[1]).toBeUndefined();

    newer.resolve({ episodes: [{ name: "fresh" }] });
    await newerRequest;
    expect(store.loading[1]).toBe(false);
    expect(store.episodes[1]).toEqual([{ name: "fresh" }]);
  });

  it("normalizes invalid episode payloads without affecting the response contract", async () => {
    const response = { episodes: null, season_number: 4 };
    const { loadSeason, store } = createStore(vi.fn().mockResolvedValue(response));
    store.initialize({ tvId: "400" });

    await expect(store.load("not-a-season")).resolves.toBeNull();
    await expect(store.load(4)).resolves.toBe(response);

    expect(loadSeason).toHaveBeenCalledOnce();
    expect(store.episodes[4]).toEqual([]);
  });

  it("dispose aborts every season and prevents late or subsequent work", async () => {
    const first = deferred();
    const second = deferred();
    const signals = [];
    const { loadSeason, store } = createStore(
      vi.fn((_tvId, seasonNumber, { signal }) => {
        signals.push(signal);
        return seasonNumber === 1 ? first.promise : second.promise;
      }),
    );
    store.initialize({ tvId: "500" });
    const firstRequest = store.load(1);
    const secondRequest = store.load(2);

    store.dispose();

    expect(signals.every((signal) => signal.aborted)).toBe(true);
    expect(store.episodes).toEqual({});
    expect(store.loading).toEqual({});
    expect(store.errors).toEqual({});
    first.resolve({ episodes: [{ name: "stale-1" }] });
    second.resolve({ episodes: [{ name: "stale-2" }] });
    await Promise.all([firstRequest, secondRequest]);

    expect(store.initialize({ tvId: "new" })).toBeNull();
    await expect(store.load(1)).resolves.toBeNull();
    expect(loadSeason).toHaveBeenCalledTimes(2);
  });
});
