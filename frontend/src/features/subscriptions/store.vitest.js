import { describe, expect, it, vi } from "vitest";
import { normalizeWantedSubscriptionsPage } from "../../shared/api/endpoints/subscriptions.js";
import { createSubscriptionStore } from "./store.js";

function summaryRecord(id, overrides = {}) {
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
    title: id,
    release_year: null,
    poster_url: "",
    category_text: null,
    douban_sort_time: null,
    attention_tags: [],
    ...overrides,
  };
}

function subscriptionSummaryState(items = []) {
  return normalizeWantedSubscriptionsPage({
    items: items.map((item) => summaryRecord(item.subject_id, item)),
    next_cursor: null,
  });
}

function nestedDetail(id, revision, overrides = {}) {
  return {
    summary: summaryRecord(id, {
      revision,
      updated_at: revision * 100,
      title: `详情 ${id}`,
    }),
    source: { synopsis: `简介 ${revision}` },
    observation: { created_at: 100, first_seen_at: 100, last_seen_at: revision * 100 },
    issues: [],
    skip_reason: null,
    candidates: [],
    tv: null,
    downloads: [],
    links: [],
    ...overrides,
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function fakeDocument(initiallyHidden = false) {
  const target = new EventTarget();
  let hidden = initiallyHidden;
  Object.defineProperties(target, {
    hidden: { get: () => hidden },
    visibilityState: { get: () => (hidden ? "hidden" : "visible") },
  });
  target.setHidden = (value) => {
    hidden = value;
    target.dispatchEvent(new Event("visibilitychange"));
  };
  return target;
}

function transport(overrides = {}) {
  return {
    load: vi.fn().mockResolvedValue(subscriptionSummaryState()),
    loadDetail: vi.fn().mockRejectedValue(new Error("unexpected detail request")),
    poll: vi.fn().mockResolvedValue({
      inserted: 0,
      updated: 0,
      unchanged: 0,
      reactivated: 0,
      deactivated: 0,
      fetched_items: 0,
      snapshot_complete: true,
    }),
    ...overrides,
  };
}

describe("subscription store polling lifecycle", () => {
  it("starts immediately, repeats on the interval, and stops with the route tree", async () => {
    vi.useFakeTimers();
    const api = transport();
    const store = createSubscriptionStore({
      transport: api,
      pollIntervalMs: 1000,
      documentRef: fakeDocument(),
    });

    await store.start();
    expect(api.load).toHaveBeenCalledTimes(1);
    expect(store.pollingActive.value).toBe(true);

    await vi.advanceTimersByTimeAsync(1000);
    expect(api.load).toHaveBeenCalledTimes(2);

    store.stop();
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.load).toHaveBeenCalledTimes(2);
    expect(store.pollingActive.value).toBe(false);
  });

  it("pauses while hidden and refreshes as soon as the document becomes visible", async () => {
    vi.useFakeTimers();
    const documentRef = fakeDocument(true);
    const api = transport();
    const store = createSubscriptionStore({
      transport: api,
      pollIntervalMs: 1000,
      documentRef,
    });

    await store.start();
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.load).not.toHaveBeenCalled();

    documentRef.setHidden(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(api.load).toHaveBeenCalledTimes(1);

    documentRef.setHidden(true);
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.load).toHaveBeenCalledTimes(1);
  });

  it("recovers after a background error and schedules the next refresh", async () => {
    vi.useFakeTimers();
    const backgroundError = vi.fn();
    const api = transport({
      load: vi
        .fn()
        .mockRejectedValueOnce(new Error("temporary outage"))
        .mockResolvedValueOnce(subscriptionSummaryState([summaryRecord("recovered")])),
    });
    const store = createSubscriptionStore({
      transport: api,
      pollIntervalMs: 1000,
      documentRef: fakeDocument(),
      onBackgroundError: backgroundError,
    });

    await store.start();
    expect(backgroundError).toHaveBeenCalledTimes(1);
    expect(store.lastError.value).toBe("temporary outage");

    await vi.advanceTimersByTimeAsync(1000);
    expect(api.load).toHaveBeenCalledTimes(2);
    expect(store.records.value.map((item) => item.subject_id)).toEqual(["recovered"]);
    expect(store.lastError.value).toBe("");
  });

  it("coalesces callers and never overlaps a slow refresh", async () => {
    vi.useFakeTimers();
    const first = deferred();
    const api = transport({
      load: vi
        .fn()
        .mockReturnValueOnce(first.promise)
        .mockResolvedValue(subscriptionSummaryState([summaryRecord("second")])),
    });
    const store = createSubscriptionStore({
      transport: api,
      pollIntervalMs: 1000,
      documentRef: fakeDocument(),
    });

    const start = store.start();
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.load).toHaveBeenCalledTimes(1);

    const joinedA = store.refresh();
    const joinedB = store.refresh();
    expect(joinedA).toBe(joinedB);
    expect(api.load).toHaveBeenCalledTimes(1);

    first.resolve(subscriptionSummaryState([summaryRecord("first")]));
    await Promise.all([start, joinedA]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.load).toHaveBeenCalledTimes(2);
  });
});

describe("subscription store latest list and detail state", () => {
  it("owns manual polling and exposes records, summary, and selected ID", async () => {
    const api = transport({
      poll: vi.fn().mockResolvedValue({
        inserted: 1,
        updated: 2,
        unchanged: 3,
        reactivated: 4,
        deactivated: 5,
        fetched_items: 15,
        snapshot_complete: true,
      }),
      load: vi
        .fn()
        .mockResolvedValue(
          subscriptionSummaryState([
            summaryRecord("selected", { title: "选中订阅", lifecycle_state: "searching" }),
          ]),
        ),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    store.setSelectedId("selected");
    const result = await store.poll();

    expect(api.poll).toHaveBeenCalledTimes(1);
    expect(api.load).toHaveBeenCalledTimes(1);
    expect(result.outcome).toEqual({
      inserted: 1,
      updated: 2,
      unchanged: 3,
      reactivated: 4,
      deactivated: 5,
      fetched_items: 15,
      snapshot_complete: true,
    });
    expect(store.selected.value?.title).toBe("选中订阅");
    expect(store.summary.value).toContain("总计 1");
  });

  it("patches newer summaries without erasing detail and refreshes stale detail", async () => {
    const summaryV2 = summaryRecord("subject-1", {
      revision: 2,
      updated_at: 200,
      title: "摘要 v2",
    });
    const summaryV1 = summaryRecord("subject-1", {
      revision: 1,
      updated_at: 100,
      title: "过期摘要",
    });
    const summaryV3 = summaryRecord("subject-1", {
      revision: 3,
      updated_at: 300,
      title: "摘要 v3",
      lifecycle_state: "searching",
    });
    const detailV2 = nestedDetail("subject-1", 2, {
      summary: summaryV2,
      source: { synopsis: "详情 v2" },
      downloads: [{ id: "download-2", progress: 0.5, files: [] }],
    });
    const detailV3 = nestedDetail("subject-1", 3, {
      summary: summaryV3,
      source: { synopsis: "详情 v3" },
      downloads: [{ id: "download-3", progress: 0.75, files: [] }],
    });
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([summaryV2]))
        .mockResolvedValueOnce(subscriptionSummaryState([summaryV1]))
        .mockResolvedValueOnce(subscriptionSummaryState([summaryV3])),
      loadDetail: vi.fn().mockResolvedValueOnce(detailV2).mockResolvedValueOnce(detailV3),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    await store.loadDetail("subject-1");
    expect(store.hasFreshDetail("subject-1")).toBe(true);

    await store.refresh();
    expect(store.getById("subject-1")).toMatchObject({
      revision: 2,
      title: "摘要 v2",
      source: { synopsis: "详情 v2" },
    });
    expect(store.hasFreshDetail("subject-1")).toBe(true);

    await store.refresh();
    expect(store.getById("subject-1")).toMatchObject({
      revision: 3,
      title: "摘要 v3",
      source: { synopsis: "详情 v2" },
      downloads: [{ id: "download-2", progress: 0.5, files: [] }],
    });
    expect(store.hasFreshDetail("subject-1")).toBe(false);

    await store.loadDetail("subject-1");
    expect(store.getById("subject-1")).toMatchObject({
      revision: 3,
      source: { synopsis: "详情 v3" },
      downloads: [{ id: "download-3", progress: 0.75, files: [] }],
    });
    expect(store.hasFreshDetail("subject-1")).toBe(true);
  });

  it("does not claim that a summary-only entity has fresh detail", async () => {
    const api = transport({
      load: vi.fn().mockResolvedValue(subscriptionSummaryState([summaryRecord("summary-only")])),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();

    expect(store.getById("summary-only")?.title).toBe("summary-only");
    expect(store.hasFreshDetail("summary-only")).toBe(false);
  });

  it("loads a nested detail DTO into the entity cache and reuses a fresh revision", async () => {
    const summary = summaryRecord("subject-1", {
      revision: 7,
      title: "摘要标题",
      updated_at: 700,
    });
    const detailRequest = deferred();
    const api = transport({
      load: vi.fn().mockResolvedValue(subscriptionSummaryState([summary])),
      loadDetail: vi.fn().mockReturnValue(detailRequest.promise),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    const loading = store.loadDetail("subject-1");
    expect(store.isDetailLoading("subject-1")).toBe(true);
    detailRequest.resolve(
      nestedDetail("subject-1", 7, {
        summary: { ...summary, title: "详情标题" },
        source: { synopsis: "嵌套简介", date_published: "2026-07-11" },
        candidates: [{ torrent_id: "torrent-1", title: "Candidate" }],
        downloads: [{ id: "download-1", progress: 0.5, files: [] }],
        links: [{ id: "link-1", state: "planned", checked_at: 700, files: [] }],
      }),
    );
    await loading;

    expect(store.getById("subject-1")).toMatchObject({
      subject_id: "subject-1",
      revision: 7,
      title: "详情标题",
      source: { synopsis: "嵌套简介", date_published: "2026-07-11" },
      candidates: [{ torrent_id: "torrent-1", title: "Candidate" }],
      downloads: [{ id: "download-1", progress: 0.5, files: [] }],
      links: [{ id: "link-1", state: "planned", checked_at: 700, files: [] }],
    });
    expect(store.getById("subject-1")).not.toHaveProperty("summary");
    expect(store.hasFreshDetail("subject-1")).toBe(true);
    expect(store.isDetailLoading("subject-1")).toBe(false);
    expect(store.detailError("subject-1")).toBe("");

    await store.loadDetail("subject-1");
    expect(api.loadDetail).toHaveBeenCalledTimes(1);
  });

  it("invalidates detail on a newer summary and retries a stale detail response", async () => {
    const summaryV1 = summaryRecord("subject-1", { revision: 1, title: "摘要 v1" });
    const summaryV2 = summaryRecord("subject-1", {
      revision: 2,
      title: "摘要 v2",
      updated_at: 200,
      lifecycle_state: "searching",
    });
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([summaryV1]))
        .mockResolvedValueOnce(subscriptionSummaryState([summaryV2])),
      loadDetail: vi
        .fn()
        .mockResolvedValueOnce(nestedDetail("subject-1", 1, { summary: summaryV1 }))
        .mockResolvedValueOnce(
          nestedDetail("subject-1", 1, {
            summary: summaryV1,
            source: { synopsis: "迟到的旧详情" },
          }),
        )
        .mockResolvedValueOnce(
          nestedDetail("subject-1", 2, {
            summary: summaryV2,
            source: { synopsis: "详情 v2" },
          }),
        ),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    await store.loadDetail("subject-1");
    await store.refresh();
    expect(store.hasFreshDetail("subject-1")).toBe(false);

    await store.loadDetail("subject-1");

    expect(api.loadDetail).toHaveBeenCalledTimes(3);
    expect(store.getById("subject-1")).toMatchObject({
      revision: 2,
      title: "摘要 v2",
      lifecycle_state: "searching",
      source: { synopsis: "详情 v2" },
    });
    expect(store.hasFreshDetail("subject-1")).toBe(true);
  });

  it("cancels an obsolete detail request and keeps cancellation out of visible errors", async () => {
    const request = deferred();
    const api = transport({
      load: vi.fn().mockResolvedValue(subscriptionSummaryState([summaryRecord("subject-1")])),
      loadDetail: vi.fn().mockImplementation((_id, { signal }) => {
        signal.addEventListener(
          "abort",
          () => {
            const error = new Error("aborted");
            error.name = "AbortError";
            request.reject(error);
          },
          { once: true },
        );
        return request.promise;
      }),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    const detail = store.loadDetail("subject-1");
    store.cancelDetailLoad("subject-1");

    await expect(detail).rejects.toMatchObject({ name: "AbortError" });
    expect(store.isDetailLoading("subject-1")).toBe(false);
    expect(store.detailError("subject-1")).toBe("");
    expect(store.hasFreshDetail("subject-1")).toBe(false);
  });

  it("rejects invalid detail IDs before transport and records non-abort detail failures", async () => {
    const api = transport({
      loadDetail: vi.fn().mockRejectedValue(new Error("detail unavailable")),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await expect(store.loadDetail(" subject-1")).rejects.toThrow("subscription id is invalid");
    expect(api.loadDetail).not.toHaveBeenCalled();

    await expect(store.loadDetail("subject-1")).rejects.toThrow("detail unavailable");
    expect(store.detailError("subject-1")).toBe("detail unavailable");
  });

  it("uses the server-provided summary order verbatim", async () => {
    const summaries = ["100", "2", "9"].map((id) =>
      summaryRecord(id, { douban_sort_time: null, updated_at: 100 }),
    );
    const api = transport({
      load: vi.fn().mockResolvedValue(subscriptionSummaryState(summaries)),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();

    expect(store.records.value.map((item) => item.subject_id)).toEqual(["100", "2", "9"]);
  });

  it("rejects ordered IDs that are missing, duplicated, or unknown", async () => {
    const records = {
      first: summaryRecord("first"),
      second: summaryRecord("second"),
    };
    const invalidOrders = [
      { ordered_ids: ["first"], message: "must match records exactly" },
      { ordered_ids: ["first", "first", "second"], message: "duplicate ID" },
      { ordered_ids: ["first", "unknown"], message: "unknown ID" },
    ];

    for (const { ordered_ids, message } of invalidOrders) {
      const api = transport({
        load: vi.fn().mockResolvedValue({ next_cursor: null, ordered_ids, records }),
      });
      const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

      await expect(store.refresh()).rejects.toThrow(message);
      expect(store.state.value).toBeNull();
    }
  });

  it("preserves newer detail merges across a late complete snapshot", async () => {
    const listRequest = deferred();
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([summaryRecord("100"), summaryRecord("2")]))
        .mockReturnValueOnce(listRequest.promise),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    const refresh = store.refresh();
    await vi.waitFor(() => expect(api.load).toHaveBeenCalledTimes(2));
    store.mergeDetail(nestedDetail("100", 2), "100");
    store.mergeDetail(nestedDetail("2", 2), "2");
    listRequest.resolve(subscriptionSummaryState([summaryRecord("9")]));
    await refresh;

    expect(store.state.value.ordered_ids).toEqual(["9", "100", "2"]);
    expect(store.records.value.map((item) => item.subject_id)).toEqual(["9", "100", "2"]);
  });

  it("keeps the existing cache when transport violates the aggregated latest contract", async () => {
    const kept = summaryRecord("kept", { revision: 2, updated_at: 200 });
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([kept]))
        .mockResolvedValueOnce({
          next_cursor: "opaque-next-page",
          ordered_ids: ["replacement"],
          records: { replacement: summaryRecord("replacement") },
        })
        .mockResolvedValueOnce({
          next_cursor: null,
          ordered_ids: ["replacement"],
          records: { replacement: summaryRecord("replacement") },
          unexpected: true,
        })
        .mockResolvedValueOnce({ items: [], next_cursor: null }),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await expect(store.refresh()).rejects.toThrow("invalid list state");
      expect(store.records.value.map((item) => item.subject_id)).toEqual(["kept"]);
    }
  });

  it("rejects summaries without a legal revision before changing cache metadata", async () => {
    const kept = summaryRecord("kept", { revision: 2, updated_at: 200 });
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([kept]))
        .mockResolvedValueOnce({
          next_cursor: null,
          ordered_ids: ["kept"],
          records: {
            kept: { ...summaryRecord("kept"), revision: undefined, title: "invalid summary" },
          },
        }),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    await expect(store.refresh()).rejects.toThrow("summary revision");

    expect(store.getById("kept")?.title).toBe("kept");
  });

  it("rejects unsafe or aliased cache IDs without changing the existing entity", async () => {
    const api = transport({
      load: vi
        .fn()
        .mockResolvedValueOnce(subscriptionSummaryState([summaryRecord("kept")]))
        .mockResolvedValueOnce({
          next_cursor: null,
          ordered_ids: ["kept"],
          records: { kept: summaryRecord("different-id", { revision: 2 }) },
        })
        .mockResolvedValueOnce({
          next_cursor: null,
          ordered_ids: [" kept"],
          records: { " kept": summaryRecord(" kept", { revision: 2 }) },
        }),
    });
    const store = createSubscriptionStore({ transport: api, documentRef: fakeDocument() });

    await store.refresh();
    expect(() => store.setSelectedId(" kept")).toThrow("subscription id is invalid");
    expect(store.getById(" kept")).toBeNull();
    await expect(store.refresh()).rejects.toThrow("does not match its cache key");
    await expect(store.refresh()).rejects.toThrow("invalid record");

    expect(store.records.value.map((item) => item.subject_id)).toEqual(["kept"]);
  });
});
