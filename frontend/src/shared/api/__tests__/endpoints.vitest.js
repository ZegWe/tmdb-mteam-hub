import { describe, expect, it, vi } from "vitest";
import {
  getAuthStatus,
  getSettings,
  getMediaDetail,
  getOperationLogs,
  getWantedSubscriptionDetail,
  getWantedSubscriptions,
  isValidSubscriptionId,
  loginAuthSession,
  MAX_PAGES,
  MAX_RECORDS,
  normalizeWantedSubscriptionDetailResponse,
  normalizeWantedSubscriptionsPage,
  logoutAuthSession,
  pushMteamTorrent,
  searchMteamTorrents,
  searchTmdb,
  SUBSCRIPTION_SUMMARY_FIELDS,
  testQbServer,
  updateSettings,
} from "../endpoints/index.js";

function clientMock() {
  return { request: vi.fn().mockResolvedValue({ ok: true }) };
}

function subscriptionSummary(id = "summary-1", overrides = {}) {
  return {
    subject_id: id,
    revision: 7,
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
    updated_at: 200,
    title: "Summary",
    release_year: 2026,
    poster_url: "https://example.test/poster.jpg",
    category_text: "电影",
    douban_sort_time: 190,
    attention_tags: [],
    ...overrides,
  };
}

function subscriptionDetail(id = "summary-1", overrides = {}) {
  return {
    summary: subscriptionSummary(id),
    source: { synopsis: "Nested detail synopsis" },
    observation: { created_at: 100, first_seen_at: 100, last_seen_at: 200 },
    issues: [],
    skip_reason: null,
    candidates: [],
    tv: null,
    downloads: [],
    links: [],
    ...overrides,
  };
}

describe("API endpoint modules", () => {
  it("builds cookie-backed auth requests without retaining the token", async () => {
    const client = clientMock();
    const token = "test-management-token-123456789";
    const signal = new AbortController().signal;

    await getAuthStatus({ client, signal });
    await loginAuthSession(token, { client, signal });
    await logoutAuthSession({ client, signal });

    expect(client.request).toHaveBeenNthCalledWith(1, "/api/auth/status", {
      signal,
      credentials: "same-origin",
    });
    expect(client.request).toHaveBeenNthCalledWith(2, "/api/auth/login", {
      signal,
      method: "POST",
      body: { token },
      credentials: "same-origin",
    });
    expect(client.request).toHaveBeenNthCalledWith(3, "/api/auth/logout", {
      signal,
      method: "POST",
      credentials: "same-origin",
    });
  });

  it("returns the strict auth status DTO without legacy field adaptation", async () => {
    const status = {
      authenticated: true,
      token_configured: true,
      bootstrap_allowed: false,
    };
    const client = clientMock();
    client.request.mockResolvedValue(status);

    await expect(getAuthStatus({ client })).resolves.toBe(status);
    expect(status).not.toHaveProperty("logged_in");
    expect(status).not.toHaveProperty("bootstrap");
  });

  it("builds search and media detail requests", async () => {
    const client = clientMock();

    await searchTmdb("肖申克的救赎", { client });
    await getMediaDetail("movie", 278, { client });
    await searchMteamTorrents({ source: "imdb", imdb_id: "tt0111161" }, { client });

    expect(client.request).toHaveBeenNthCalledWith(
      1,
      `/api/search?${new URLSearchParams({ q: "肖申克的救赎" })}`,
      {},
    );
    expect(client.request).toHaveBeenNthCalledWith(2, "/api/tmdb/movie/278", {});
    expect(client.request).toHaveBeenNthCalledWith(
      3,
      `/api/mteam/torrents?${new URLSearchParams({ source: "imdb", imdb_id: "tt0111161" })}`,
      {},
    );
  });

  it("builds subscription and log requests", async () => {
    const client = clientMock();
    client.request.mockResolvedValueOnce({ items: [], next_cursor: null });

    await getWantedSubscriptions({ client });
    await getOperationLogs({ page: 2, status: "failed", q: "" }, { client });

    expect(client.request).toHaveBeenNthCalledWith(1, "/api/subscriptions/wanted?limit=100", {});
    expect(client.request).toHaveBeenNthCalledWith(
      2,
      `/api/operation-logs?${new URLSearchParams({ page: "2", status: "failed" })}`,
      {},
    );
  });

  it("builds settings and qB requests from strict DTOs", async () => {
    const client = clientMock();
    const update = { expected_revision: 7, tmdb_api_key: "key" };
    const qbTest = { server_id: "nas" };
    const qbPush = { server_id: "nas", torrent_id: "42", category: "movie" };

    await getSettings({ client });
    await updateSettings(update, { client });
    await testQbServer(qbTest, { client });
    await pushMteamTorrent(qbPush, { client });

    expect(client.request).toHaveBeenNthCalledWith(1, "/api/config", {});
    expect(client.request).toHaveBeenNthCalledWith(2, "/api/config", {
      method: "PUT",
      body: update,
    });
    expect(client.request).toHaveBeenNthCalledWith(3, "/api/qb/test", {
      method: "POST",
      body: qbTest,
    });
    expect(client.request).toHaveBeenNthCalledWith(4, "/api/qb/push-mteam", {
      method: "POST",
      body: qbPush,
    });
  });

  it("normalizes strict summary-page subscription responses at the endpoint boundary", () => {
    const summaryRecord = subscriptionSummary("summary-1", {
      unexpected_detail_field: "must not cross the summary boundary",
    });
    const page = normalizeWantedSubscriptionsPage({
      items: [summaryRecord],
      next_cursor: null,
    });
    expect(page).toEqual({
      next_cursor: null,
      ordered_ids: ["summary-1"],
      records: {
        "summary-1": Object.fromEntries(
          SUBSCRIPTION_SUMMARY_FIELDS.map((field) => [field, summaryRecord[field]]),
        ),
      },
    });
    expect(page).not.toHaveProperty("items");
    expect(page.records["summary-1"]).not.toHaveProperty("unexpected_detail_field");

    const continuation = normalizeWantedSubscriptionsPage({
      items: [subscriptionSummary("summary-2")],
      next_cursor: "opaque-next-page",
    });
    expect(continuation.next_cursor).toBe("opaque-next-page");
  });

  it("aggregates every v5 cursor page with fixed filters and an encoded cursor URL", async () => {
    const client = clientMock();
    const firstCursor = "opaque.next+page";
    client.request
      .mockResolvedValueOnce({
        items: [subscriptionSummary("summary-1")],
        next_cursor: firstCursor,
      })
      .mockResolvedValueOnce({
        items: [subscriptionSummary("summary-2", { revision: 8 })],
        next_cursor: null,
      });

    const filters = {
      active: true,
      media_kind: "movie",
      lifecycle_state: "queued",
      attention_tag: "failed",
    };
    const result = await getWantedSubscriptions({ client, filters });
    const firstParams = new URLSearchParams({
      limit: "100",
      active: "true",
      media_kind: "movie",
      lifecycle_state: "queued",
      attention_tag: "failed",
    });
    const secondParams = new URLSearchParams(firstParams);
    secondParams.set("cursor", firstCursor);

    expect(client.request).toHaveBeenNthCalledWith(
      1,
      `/api/subscriptions/wanted?${firstParams}`,
      {},
    );
    expect(client.request).toHaveBeenNthCalledWith(
      2,
      `/api/subscriptions/wanted?${secondParams}`,
      {},
    );
    expect(result).toEqual({
      next_cursor: null,
      ordered_ids: ["summary-1", "summary-2"],
      records: {
        "summary-1": subscriptionSummary("summary-1"),
        "summary-2": subscriptionSummary("summary-2", { revision: 8 }),
      },
    });
  });

  it("preserves raw numeric ID order across cursor pages without inferring it from object keys", async () => {
    const client = clientMock();
    client.request
      .mockResolvedValueOnce({
        items: [
          subscriptionSummary("100", { douban_sort_time: null, updated_at: 100 }),
          subscriptionSummary("2", { douban_sort_time: null, updated_at: 100 }),
        ],
        next_cursor: "page-2",
      })
      .mockResolvedValueOnce({
        items: [subscriptionSummary("9", { douban_sort_time: null, updated_at: 100 })],
        next_cursor: null,
      });

    const result = await getWantedSubscriptions({ client });

    expect(result.ordered_ids).toEqual(["100", "2", "9"]);
    expect(result.ordered_ids.map((id) => result.records[id].subject_id)).toEqual([
      "100",
      "2",
      "9",
    ]);
    expect(Object.keys(result.records)).toEqual(["2", "9", "100"]);
  });

  it("aborts between cursor pages and never resolves a partial snapshot", async () => {
    const preAbortedClient = clientMock();
    const preAbortedController = new AbortController();
    preAbortedController.abort();
    await expect(
      getWantedSubscriptions({ client: preAbortedClient, signal: preAbortedController.signal }),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(preAbortedClient.request).not.toHaveBeenCalled();

    const client = clientMock();
    const controller = new AbortController();
    const committed = { records: { kept: { subject_id: "kept" } } };
    client.request
      .mockResolvedValueOnce({
        items: [subscriptionSummary("first-page")],
        next_cursor: "cursor-2",
      })
      .mockImplementationOnce(async () => {
        controller.abort();
        return {
          items: [subscriptionSummary("second-page")],
          next_cursor: null,
        };
      });

    const request = getWantedSubscriptions({ client, signal: controller.signal }).then((state) => {
      committed.records = state.records;
    });

    await expect(request).rejects.toMatchObject({ name: "AbortError" });
    expect(client.request).toHaveBeenCalledTimes(2);
    expect(committed.records).toEqual({ kept: { subject_id: "kept" } });
  });

  it("rejects repeated cursors and duplicate IDs across pages", async () => {
    const repeatedCursorClient = clientMock();
    repeatedCursorClient.request
      .mockResolvedValueOnce({ items: [], next_cursor: "same-cursor" })
      .mockResolvedValueOnce({ items: [], next_cursor: "same-cursor" });
    await expect(getWantedSubscriptions({ client: repeatedCursorClient })).rejects.toThrow(
      "repeated next_cursor",
    );
    expect(repeatedCursorClient.request).toHaveBeenCalledTimes(2);

    const duplicateIdClient = clientMock();
    duplicateIdClient.request
      .mockResolvedValueOnce({
        items: [subscriptionSummary("duplicate")],
        next_cursor: "page-2",
      })
      .mockResolvedValueOnce({
        items: [subscriptionSummary("duplicate", { revision: 8 })],
        next_cursor: null,
      });
    await expect(getWantedSubscriptions({ client: duplicateIdClient })).rejects.toThrow(
      "duplicate record ID across pages: duplicate",
    );
  });

  it("enforces page and aggregate record limits", async () => {
    const pageLimitClient = clientMock();
    pageLimitClient.request.mockImplementation(async () => ({
      items: [],
      next_cursor: `cursor-${pageLimitClient.request.mock.calls.length}`,
    }));
    await expect(getWantedSubscriptions({ client: pageLimitClient })).rejects.toThrow(
      `page limit exceeded: ${MAX_PAGES}`,
    );
    expect(pageLimitClient.request).toHaveBeenCalledTimes(MAX_PAGES);

    const recordLimitClient = clientMock();
    recordLimitClient.request.mockResolvedValueOnce({
      items: Array.from({ length: MAX_RECORDS + 1 }, (_, index) =>
        subscriptionSummary(`record-${index}`),
      ),
      next_cursor: null,
    });
    await expect(getWantedSubscriptions({ client: recordLimitClient })).rejects.toThrow(
      `record limit exceeded: ${MAX_RECORDS}`,
    );
    expect(recordLimitClient.request).toHaveBeenCalledTimes(1);
  });

  it("normalizes the strict nested subscription detail DTO", async () => {
    const client = clientMock();
    const response = subscriptionDetail("主题-1", {
      summary: subscriptionSummary("主题-1", {
        unexpected_summary_field: "must not escape the summary whitelist",
      }),
      candidates: [{ torrent_id: "torrent-1" }],
    });
    client.request.mockResolvedValueOnce(response);

    const detail = await getWantedSubscriptionDetail("主题-1", { client });

    expect(client.request).toHaveBeenCalledWith(
      `/api/subscriptions/wanted/${encodeURIComponent("主题-1")}`,
      {},
    );
    expect(detail.summary.subject_id).toBe("主题-1");
    expect(detail.summary.revision).toBe(7);
    expect(detail.summary).not.toHaveProperty("unexpected_summary_field");
    expect(detail.source).toEqual({ synopsis: "Nested detail synopsis" });
    expect(detail.candidates).toEqual([{ torrent_id: "torrent-1" }]);
    expect(detail).not.toHaveProperty("subject_id");
    expect(detail).not.toHaveProperty("revision");
    expect(detail).not.toHaveProperty("title");
  });

  it("fails closed for unknown detail shapes, missing revisions, and path/body ID mismatch", async () => {
    for (const response of [
      null,
      {},
      { ...subscriptionDetail("subject-1"), subject_id: "subject-1" },
      subscriptionDetail("subject-1", { source: [] }),
      subscriptionDetail("subject-1", { issues: [null] }),
      subscriptionDetail("subject-1", {
        summary: subscriptionSummary("subject-1", { revision: undefined }),
      }),
    ]) {
      expect(() => normalizeWantedSubscriptionDetailResponse(response, "subject-1")).toThrow(
        "Invalid subscription detail response",
      );
    }

    const client = clientMock();
    client.request.mockResolvedValueOnce(subscriptionDetail("different-id"));
    await expect(getWantedSubscriptionDetail("subject-1", { client })).rejects.toThrow(
      "summary ID does not match the requested path ID",
    );
    expect(client.request).toHaveBeenCalledTimes(1);
  });

  it("rejects an invalid original detail ID before issuing a request", async () => {
    for (const id of [" subject-1", "subject-1 ", "subject/1", "界".repeat(86)]) {
      const client = clientMock();
      await expect(getWantedSubscriptionDetail(id, { client })).rejects.toThrow(
        "subscription id is invalid",
      );
      expect(client.request).not.toHaveBeenCalled();
    }
  });

  it.each([".", "..", "\ud800", "\udc00", "prefix\ud800", "\udc00suffix"])(
    "rejects the unsafe path ID %j before detail can request it",
    async (id) => {
      const client = clientMock();
      await expect(getWantedSubscriptionDetail(id, { client })).rejects.toThrow(
        "subscription id is invalid",
      );
      expect(client.request).not.toHaveBeenCalled();
    },
  );

  it("validates subscription IDs exactly using the backend UTF-8 byte contract", () => {
    expect(isValidSubscriptionId("subject-1")).toBe(true);
    expect(isValidSubscriptionId("a".repeat(256))).toBe(true);
    expect(isValidSubscriptionId("界".repeat(85))).toBe(true);
    expect(isValidSubscriptionId("😀".repeat(64))).toBe(true);

    for (const invalid of [
      null,
      42,
      "",
      " subject",
      "subject ",
      "subject\n",
      "subject\0id",
      "subject\u0085id",
      "subject/id",
      "subject\\id",
      ".",
      "..",
      "\ud800",
      "\udc00",
      "prefix\ud800",
      "\udc00suffix",
      "a".repeat(257),
      "界".repeat(86),
      "😀".repeat(65),
    ]) {
      expect(isValidSubscriptionId(invalid), `accepted invalid ID ${JSON.stringify(invalid)}`).toBe(
        false,
      );
    }
  });

  it("fails closed for unknown or malformed subscription list responses", async () => {
    const malformed = [
      null,
      {},
      { records: {}, items: [], next_cursor: null },
      { records: [] },
      { records: {}, next_cursor: undefined },
      { records: {}, next_cursor: "opaque-next-page" },
      {
        records: {
          "map-key": { subject_id: "different-id", title: "mismatched alias" },
        },
      },
      { records: { "map-key": { subject_id: null } } },
      { records: { " map-key": { title: "trimmed aliases are forbidden" } } },
      { items: "not-an-array", next_cursor: null },
      { items: [], next_cursor: "" },
      { items: [], next_cursor: null, unknown: true },
      { items: [null], next_cursor: null },
      { items: [[]], next_cursor: null },
      { items: [{}], next_cursor: null },
      { items: [subscriptionSummary("subject/id")], next_cursor: null },
      { items: [{ subject_id: "missing-revision" }], next_cursor: null },
      {
        items: [subscriptionSummary("missing-revision", { revision: undefined })],
        next_cursor: null,
      },
      { items: [subscriptionSummary("null-revision", { revision: null })], next_cursor: null },
      { items: [subscriptionSummary("zero-revision", { revision: 0 })], next_cursor: null },
      { items: [subscriptionSummary("negative-revision", { revision: -1 })], next_cursor: null },
      { items: [subscriptionSummary("fraction-revision", { revision: 1.5 })], next_cursor: null },
      {
        items: [subscriptionSummary("unsafe-revision", { revision: Number.MAX_SAFE_INTEGER + 1 })],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("missing-title", { title: undefined })],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("invalid-media-kind", { media_kind: "podcast" })],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("invalid-lifecycle", { lifecycle_state: "paused" })],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("invalid-execution", { execution_state: "finished" })],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("invalid-attention", { attention_tags: ["unknown"] })],
        next_cursor: null,
      },
      {
        items: [
          subscriptionSummary("duplicate-attention", { attention_tags: ["failed", "failed"] }),
        ],
        next_cursor: null,
      },
      {
        items: [subscriptionSummary("duplicate"), subscriptionSummary("duplicate")],
        next_cursor: null,
      },
    ];

    for (const response of malformed) {
      expect(() => normalizeWantedSubscriptionsPage(response)).toThrow(
        "Invalid subscription list response",
      );
    }

    for (const invalidItem of [null, []]) {
      const client = clientMock();
      client.request.mockResolvedValueOnce({ items: [invalidItem], next_cursor: null });
      await expect(getWantedSubscriptions({ client })).rejects.toThrow(
        "Invalid subscription list response",
      );
    }
  });
});
