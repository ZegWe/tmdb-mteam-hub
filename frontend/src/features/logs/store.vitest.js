import { describe, expect, it, vi } from "vitest";
import { createLogsStore } from "./store.js";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function transport(overrides = {}) {
  return {
    load: vi.fn().mockResolvedValue({
      items: [],
      page: 1,
      page_size: 30,
      total: 0,
      has_more: false,
    }),
    ...overrides,
  };
}

describe("logs store", () => {
  it("owns filters, pagination, append behavior, and summary state", async () => {
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce({
          items: [{ id: 1, summary: "第一页" }],
          page: 1,
          page_size: 30,
          total: 2,
          has_more: true,
        })
        .mockResolvedValueOnce({
          items: [{ id: 2, summary: "第二页" }],
          page: 2,
          page_size: 30,
          total: 2,
          has_more: false,
        }),
    });
    const store = createLogsStore({ transport: api });
    store.applyFilters({ category: "search", status: "success", q: "电影" });

    await store.load({ silent: true });
    store.filters.q = "尚未提交";
    await store.loadMore();

    expect(api.load).toHaveBeenNthCalledWith(
      1,
      {
        page: 1,
        page_size: 30,
        category: "search",
        status: "success",
        q: "电影",
      },
      { signal: expect.any(AbortSignal) },
    );
    expect(api.load).toHaveBeenNthCalledWith(
      2,
      {
        page: 2,
        page_size: 30,
        category: "search",
        status: "success",
        q: "电影",
      },
      { signal: expect.any(AbortSignal) },
    );
    expect(store.entries.value.map((entry) => entry.id)).toEqual([1, 2]);
    expect(store.page).toMatchObject({ page: 2, total: 2, has_more: false });
    expect(store.summary.value).toContain("已显示 2 条");
    expect(store.summary.value).toContain("关键词 电影");
  });

  it("keeps only the latest filter response", async () => {
    const older = deferred();
    const newer = deferred();
    const api = transport({
      load: vi.fn().mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise),
    });
    const store = createLogsStore({ transport: api });

    store.applyFilters({ q: "older" });
    const olderRequest = store.load({ silent: true });
    store.applyFilters({ q: "newer" });
    const newerRequest = store.load({ silent: true });
    newer.resolve({
      items: [{ id: 2, summary: "新日志" }],
      page: 1,
      page_size: 30,
      total: 1,
      has_more: false,
    });
    await newerRequest;
    older.resolve({
      items: [{ id: 1, summary: "旧日志" }],
      page: 1,
      page_size: 30,
      total: 1,
      has_more: false,
    });
    await olderRequest;

    expect(store.entries.value.map((entry) => entry.summary)).toEqual(["新日志"]);
    expect(store.loading.value).toBe(false);
  });

  it("owns loading/error/toast state and clears it on a later success", async () => {
    const clearTimeoutFn = vi.fn();
    const setTimeoutFn = vi.fn(() => 7);
    const api = transport({
      load: vi
        .fn()
        .mockRejectedValueOnce(new Error("network down"))
        .mockResolvedValueOnce({ items: [], page: 1, page_size: 30, total: 0 }),
    });
    const store = createLogsStore({ transport: api, clearTimeoutFn, setTimeoutFn });

    await store.load();
    expect(store.lastError.value).toBe("加载日志失败：network down");
    expect(store.toast).toMatchObject({ message: "加载日志失败：network down", kind: "err" });
    expect(store.loading.value).toBe(false);

    await store.load();
    expect(store.lastError.value).toBe("");
    expect(store.toast).toMatchObject({ message: "日志已加载", kind: "ok" });
    expect(setTimeoutFn).toHaveBeenCalledTimes(2);
    store.dispose();
    expect(clearTimeoutFn).toHaveBeenCalled();
  });

  it("aborts the active request when the page store is disposed", async () => {
    let signal;
    const api = transport({
      load: vi.fn((_filters, options) => {
        signal = options.signal;
        return new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(signal.reason), { once: true });
        });
      }),
    });
    const store = createLogsStore({ transport: api });
    const request = store.load({ silent: true });

    store.dispose();
    await request;

    expect(signal.aborted).toBe(true);
    expect(store.loading.value).toBe(false);
    expect(store.lastError.value).toBe("");
  });
});
