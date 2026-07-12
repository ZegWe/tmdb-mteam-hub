import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";

const EmptyRoute = { template: "" };
let activeWrapper = null;

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
});

function configSnapshot(overrides = {}) {
  return {
    revision: 1,
    has_tmdb_api_key: false,
    has_mteam_api_key: false,
    has_douban_cookie: false,
    qb_servers: [],
    subscription_categories: [],
    subscription_watcher: { enabled: false, dry_run: true },
    torrent_match_rules: [],
    ...overrides,
  };
}

function subscriptionRecord(overrides = {}) {
  return subscriptionSummaryDto({
    subject_id: "subject-7",
    title: "深链订阅",
    revision: 1,
    lifecycle_state: "queued",
    poster_url: "https://example.test/poster.jpg",
    updated_at: 100,
    ...overrides,
  });
}

function subscriptionState(record = subscriptionRecord()) {
  const records = Array.isArray(record) ? record : [record];
  return {
    items: records,
    next_cursor: null,
  };
}

function subscriptionSummaryDto(overrides = {}) {
  return {
    subject_id: "subject-7",
    revision: 7,
    active: true,
    inactive_at: null,
    last_seen_snapshot_id: "snapshot-7",
    media_kind: "movie",
    schedulable: true,
    blocked_reason: null,
    lifecycle_state: "downloading",
    execution_state: "idle",
    next_attempt_at: null,
    retry_count: 0,
    max_retries: 3,
    retry_blocked: false,
    force_eligible_once: false,
    updated_at: 700,
    title: "嵌套详情订阅",
    release_year: 2026,
    poster_url: "https://example.test/nested.jpg",
    category_text: "电影",
    douban_sort_time: 690,
    attention_tags: [],
    ...overrides,
  };
}

function nestedSubscriptionDetail(id = "subject-7", overrides = {}) {
  return {
    summary: subscriptionSummaryDto({ subject_id: id }),
    source: {
      cover_url: "https://example.test/nested.jpg",
      original_title: "Nested Original",
      aka: ["嵌套别名"],
      languages: ["中文"],
      countries: ["中国"],
      genres: ["剧情"],
      directors: ["测试导演"],
      actors: ["测试演员"],
      date_published: "2026-07-11",
      duration: "120 分钟",
      synopsis: "来自 nested DTO 的简介",
      rating_value: 8.8,
      rating_count: 1200,
      tags: ["测试"],
      douban_date: "2026-07-10",
      douban_return_order: 0,
    },
    observation: { created_at: 100, first_seen_at: 110, last_seen_at: 690 },
    issues: [
      {
        owner: "subscription",
        artifact_id: null,
        lane: null,
        season_number: null,
        episode_number: null,
        operation: "match_candidates",
        error_type: "no_match",
        message: "候选需要人工确认",
        occurred_at: 680,
      },
    ],
    skip_reason: null,
    candidates: [
      {
        torrent_id: "torrent-1",
        title: "Nested.Release.2160p",
        subtitle: "杜比视界",
        source: "mteam",
        selected: true,
        excluded_reason: null,
      },
    ],
    tv: null,
    downloads: [
      {
        id: "download-1",
        torrent_id: "torrent-1",
        torrent_title: "Nested.Release.2160p",
        qb_server_id: "home",
        qb_server_name: "家庭 qB",
        qb_category: "电影",
        qb_save_dir_name: "downloads",
        qb_hash: "hash-1",
        qb_name: "Nested.Release",
        qb_state: "downloading",
        state: "downloading",
        progress: 0.5,
        total_size: 4096,
        files: [{ name: "nested.mkv", size: 4096, progress: 0.5 }],
        pushed_at: 600,
        checked_at: 690,
        completed_at: null,
      },
    ],
    links: [
      {
        id: "link-1",
        download_artifact_id: "download-1",
        state: "planned",
        source_path: "/downloads/Nested.Release",
        target_dir: "/library/电影",
        checked_at: 690,
        completed_at: null,
        files: [],
      },
    ],
    ...overrides,
  };
}

function jsonResponse(value) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function jsonError(status, value) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
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

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "main", component: EmptyRoute },
      { path: "/detail/:mediaType/:id", name: "media-detail", component: EmptyRoute },
      {
        path: "/subscriptions",
        name: "subscriptions",
        component: () => import("../pages/SubscriptionsPage.vue"),
      },
      {
        path: "/subscriptions/:id",
        name: "subscription-detail",
        component: () => import("../pages/SubscriptionDetailPage.vue"),
      },
      { path: "/logs", name: "logs", component: EmptyRoute },
      { path: "/settings", name: "settings", component: EmptyRoute },
    ],
  });
}

async function mountAt(path, fetchMock) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push(path);
  await router.isReady();
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return { wrapper: activeWrapper, router };
}

async function mountAtInjectedSubscriptionParam(id, fetchMock) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push({ name: "subscription-detail", params: { id: "safe-placeholder" } });
  await router.isReady();
  router.currentRoute.value = {
    ...router.currentRoute.value,
    params: { ...router.currentRoute.value.params, id },
  };
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return { wrapper: activeWrapper, router };
}

describe("subscription lazy route pages", () => {
  it("shares one loaded store across list, detail, and browser Back", async () => {
    let wantedRequests = 0;
    let detailRequests = 0;
    const summary = subscriptionRecord();
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        wantedRequests += 1;
        return jsonResponse(subscriptionState(summary));
      }
      if (path === "/api/subscriptions/wanted/subject-7" && method === "GET") {
        detailRequests += 1;
        return jsonResponse(nestedSubscriptionDetail("subject-7", { summary }));
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper, router } = await mountAt("/subscriptions", fetchMock);

    expect(wrapper.get("#page-subscriptions").exists()).toBe(true);
    expect(wrapper.get(".subscription-card .title").text()).toBe("深链订阅");
    expect(wrapper.findAll(".subscriptions-top button")).toHaveLength(1);
    expect(wrapper.get(".subscriptions-top button").text()).toBe("刷新");
    expect(wrapper.get(".subscription-card img").attributes("src")).toBe(
      "https://example.test/poster.jpg",
    );
    expect(wrapper.get(".subscription-card .subscription-status").text()).toBe("待处理");
    expect(wrapper.get(".subscription-watcher-banner").attributes("data-watcher-mode")).toBe(
      "disabled",
    );
    expect(wrapper.find(".subscription-card button").exists()).toBe(false);
    expect(wrapper.find(".subscription-card-progress").exists()).toBe(false);
    expect(wantedRequests).toBe(1);

    await wrapper.get(".subscription-card").trigger("click");
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("subscription-detail"));
    await flushPromises();
    expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
      "深链订阅",
    );
    expect(wrapper.get(".subscription-state-graph").exists()).toBe(true);
    expect(wrapper.get(".subscription-detail-download-progress").text()).toContain("50%");
    expect(wrapper.get(".subscription-watcher-banner").attributes("data-watcher-mode")).toBe(
      "disabled",
    );
    expect(wantedRequests).toBe(1);
    expect(detailRequests).toBe(1);

    router.back();
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe("subscriptions"));
    await flushPromises();
    expect(wrapper.get(".subscription-card .title").text()).toBe("深链订阅");
    expect(wantedRequests).toBe(1);
    expect(detailRequests).toBe(1);
  });

  it("renders the latest summary-page response through the list page", async () => {
    const summary = subscriptionSummaryDto({
      subject_id: "summary-9",
      title: "分页摘要订阅",
      revision: 9,
      release_year: 2026,
    });
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return jsonResponse({ items: [summary], next_cursor: null });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper } = await mountAt("/subscriptions", fetchMock);

    expect(wrapper.get(".subscription-card .title").text()).toBe("分页摘要订阅");
    expect(wrapper.get(".subscription-card .subtle").text()).toContain("2026");
  });

  it("loads the nested detail DTO by ID and reuses the revision-aware cache after navigation", async () => {
    let listRequests = 0;
    let detailRequests = 0;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        listRequests += 1;
        return jsonResponse({ items: [subscriptionSummaryDto()], next_cursor: null });
      }
      if (path === "/api/subscriptions/wanted/subject-7" && method === "GET") {
        detailRequests += 1;
        return jsonResponse(nestedSubscriptionDetail());
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper, router } = await mountAt("/subscriptions/subject-7", fetchMock);

    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
        "嵌套详情订阅",
      ),
    );
    expect(wrapper.text()).toContain("来自 nested DTO 的简介");
    expect(wrapper.text()).toContain("候选需要人工确认");
    expect(wrapper.text()).toContain("Nested.Release.2160p");
    expect(wrapper.text()).toContain("qB家庭 qB");
    expect(wrapper.text()).toContain("链接状态计划链接");
    expect(wrapper.get("[aria-label='下载进度 50%']").exists()).toBe(true);
    expect(listRequests).toBe(1);
    expect(detailRequests).toBe(1);

    await router.push({ name: "subscriptions" });
    await flushPromises();
    await router.push({ name: "subscription-detail", params: { id: "subject-7" } });
    await flushPromises();
    await vi.waitFor(() => expect(wrapper.find(".subscription-detail").exists()).toBe(true));

    expect(detailRequests).toBe(1);
  });

  it("reloads nested detail when polling advances the selected summary revision", async () => {
    vi.useFakeTimers();
    const summaryV1 = subscriptionSummaryDto({
      revision: 1,
      updated_at: 100,
      title: "详情 v1",
    });
    const summaryV2 = subscriptionSummaryDto({
      revision: 2,
      updated_at: 200,
      title: "详情 v2",
      lifecycle_state: "searching",
    });
    let listRequests = 0;
    let detailRequests = 0;
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        listRequests += 1;
        return jsonResponse({
          items: [listRequests === 1 ? summaryV1 : summaryV2],
          next_cursor: null,
        });
      }
      if (path === "/api/subscriptions/wanted/subject-7" && method === "GET") {
        detailRequests += 1;
        const summary = detailRequests === 1 ? summaryV1 : summaryV2;
        return jsonResponse(
          nestedSubscriptionDetail("subject-7", {
            summary,
            source: {
              ...nestedSubscriptionDetail("subject-7").source,
              synopsis: `轮询后的详情 v${summary.revision}`,
            },
          }),
        );
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper } = await mountAt("/subscriptions/subject-7", fetchMock);

    expect(wrapper.text()).toContain("轮询后的详情 v1");
    expect(detailRequests).toBe(1);

    await vi.advanceTimersByTimeAsync(5000);
    await flushPromises();

    expect(wrapper.text()).toContain("轮询后的详情 v2");
    expect(wrapper.text()).not.toContain("轮询后的详情 v1");
    expect(listRequests).toBe(2);
    expect(detailRequests).toBe(2);
  });

  it("shows a stable missing-record state when a valid deep link returns typed 404", async () => {
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return jsonResponse({ items: [], next_cursor: null });
      }
      if (path === "/api/subscriptions/wanted/missing-id" && method === "GET") {
        return jsonError(404, {
          code: "subscription_not_found",
          message: "subscription not found",
        });
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper } = await mountAt("/subscriptions/missing-id", fetchMock);

    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail").text()).toContain(
        "未找到订阅记录：missing-id",
      ),
    );
    expect(wrapper.find(".subscription-detail").exists()).toBe(false);
  });

  it("cancels a stale A detail request so it cannot replace the current B route", async () => {
    const detailA = deferred();
    const summaryA = subscriptionSummaryDto({ subject_id: "subject-a", title: "摘要 A" });
    const summaryB = subscriptionSummaryDto({
      subject_id: "subject-b",
      title: "当前 B",
      revision: 8,
      updated_at: 800,
    });
    let detailASignal = null;
    const fetchMock = vi.fn((input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return Promise.resolve(jsonResponse(configSnapshot()));
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return Promise.resolve(jsonResponse({ items: [summaryA, summaryB], next_cursor: null }));
      }
      if (path === "/api/subscriptions/wanted/subject-a" && method === "GET") {
        detailASignal = init.signal;
        return detailA.promise;
      }
      if (path === "/api/subscriptions/wanted/subject-b" && method === "GET") {
        return Promise.resolve(
          jsonResponse(
            nestedSubscriptionDetail("subject-b", {
              summary: summaryB,
              source: {
                ...nestedSubscriptionDetail("subject-b").source,
                synopsis: "当前 B 的详情",
              },
            }),
          ),
        );
      }
      return Promise.reject(new Error(`Unexpected request: ${method} ${path}`));
    });
    const { wrapper, router } = await mountAt("/subscriptions/subject-a", fetchMock);

    await vi.waitFor(() => expect(detailASignal).not.toBeNull());
    await router.push({ name: "subscription-detail", params: { id: "subject-b" } });
    await flushPromises();
    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
        "当前 B",
      ),
    );
    expect(detailASignal.aborted).toBe(true);

    detailA.resolve(jsonResponse(nestedSubscriptionDetail("subject-a", { summary: summaryA })));
    await flushPromises();

    expect(router.currentRoute.value.params.id).toBe("subject-b");
    expect(wrapper.text()).toContain("当前 B 的详情");
    expect(wrapper.text()).not.toContain("摘要 A");
  });

  it.each([" subject-7", "subject-7 ", "subject/7", "subject\\7", ".", "..", "a".repeat(257)])(
    "rejects the original invalid subscription deep link %j without subscription requests",
    async (id) => {
      const fetchMock = vi.fn(async (input) => {
        const path = String(input);
        if (path === "/api/config") return jsonResponse(configSnapshot());
        throw new Error(`Unexpected request: ${path}`);
      });
      const { wrapper } = await mountAt({ name: "subscription-detail", params: { id } }, fetchMock);

      expect(wrapper.get("#page-subscription-detail").text()).toContain("订阅 ID 无效");
      expect(
        fetchMock.mock.calls.filter(([input]) =>
          String(input).startsWith("/api/subscriptions/wanted"),
        ),
      ).toHaveLength(0);
    },
  );

  it.each(["\ud800", "\udc00", "prefix\ud800", "\udc00suffix"])(
    "rejects the injected unpaired-surrogate deep link %j without subscription requests",
    async (id) => {
      const fetchMock = vi.fn(async (input) => {
        const path = String(input);
        if (path === "/api/config") return jsonResponse(configSnapshot());
        throw new Error(`Unexpected request: ${path}`);
      });
      const { wrapper } = await mountAtInjectedSubscriptionParam(id, fetchMock);

      expect(wrapper.get("#page-subscription-detail").text()).toContain("订阅 ID 无效");
      expect(
        fetchMock.mock.calls.filter(([input]) =>
          String(input).startsWith("/api/subscriptions/wanted"),
        ),
      ).toHaveLength(0);
    },
  );

  it("enters the subscription route normally after an invalid ID becomes valid", async () => {
    let wantedRequests = 0;
    const summary = subscriptionRecord();
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        wantedRequests += 1;
        return jsonResponse(subscriptionState(summary));
      }
      if (path === "/api/subscriptions/wanted/subject-7" && method === "GET") {
        return jsonResponse(nestedSubscriptionDetail("subject-7", { summary }));
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper, router } = await mountAt(
      { name: "subscription-detail", params: { id: " subject-7" } },
      fetchMock,
    );

    expect(wrapper.get("#page-subscription-detail").text()).toContain("订阅 ID 无效");
    expect(wantedRequests).toBe(0);

    await router.push({ name: "subscription-detail", params: { id: "subject-7" } });
    await flushPromises();
    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
        "深链订阅",
      ),
    );
    expect(wantedRequests).toBe(1);
  });

  it("does not let a deferred mounted A request refresh or replace the current B detail", async () => {
    const firstWantedResponse = deferred();
    const currentB = subscriptionRecord({ subject_id: "subject-b", title: "当前 B" });
    let wantedRequests = 0;
    const fetchMock = vi.fn((input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return Promise.resolve(jsonResponse(configSnapshot()));
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        wantedRequests += 1;
        if (wantedRequests === 1) return firstWantedResponse.promise;
        return Promise.resolve(jsonResponse(subscriptionState(currentB)));
      }
      if (path === "/api/subscriptions/wanted/subject-b" && method === "GET") {
        return Promise.resolve(
          jsonResponse(nestedSubscriptionDetail("subject-b", { summary: currentB })),
        );
      }
      return Promise.reject(new Error(`Unexpected request: ${method} ${path}`));
    });
    const { wrapper, router } = await mountAt(
      { name: "subscription-detail", params: { id: "subject-a" } },
      fetchMock,
    );

    await vi.waitFor(() => expect(wantedRequests).toBe(1));
    await router.push({ name: "subscription-detail", params: { id: "subject-b" } });
    await flushPromises();

    firstWantedResponse.resolve(jsonResponse(subscriptionState(currentB)));
    await flushPromises();
    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
        "当前 B",
      ),
    );
    await flushPromises();

    expect(wantedRequests).toBe(1);
    expect(router.currentRoute.value.params.id).toBe("subject-b");
    expect(wrapper.get("#page-subscription-detail").text()).not.toContain("subject-a");
  });

  it("does not let an unmounted deferred A request refresh or clear a remounted B detail", async () => {
    const firstWantedResponse = deferred();
    const currentB = subscriptionRecord({ subject_id: "subject-b", title: "卸载后的 B" });
    let wantedRequests = 0;
    const fetchMock = vi.fn((input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return Promise.resolve(jsonResponse(configSnapshot()));
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        wantedRequests += 1;
        if (wantedRequests === 1) return firstWantedResponse.promise;
        return Promise.resolve(jsonResponse(subscriptionState(currentB)));
      }
      if (path === "/api/subscriptions/wanted/subject-b" && method === "GET") {
        return Promise.resolve(
          jsonResponse(nestedSubscriptionDetail("subject-b", { summary: currentB })),
        );
      }
      return Promise.reject(new Error(`Unexpected request: ${method} ${path}`));
    });
    const { wrapper, router } = await mountAt(
      { name: "subscription-detail", params: { id: "subject-a" } },
      fetchMock,
    );

    await vi.waitFor(() => expect(wantedRequests).toBe(1));
    await router.push({ name: "main" });
    await flushPromises();
    expect(wrapper.find("#page-subscription-detail").exists()).toBe(false);

    await router.push({ name: "subscription-detail", params: { id: "subject-b" } });
    await flushPromises();
    firstWantedResponse.resolve(jsonResponse(subscriptionState(currentB)));
    await flushPromises();
    await vi.waitFor(() =>
      expect(wrapper.get("#page-subscription-detail .subscription-detail h3").text()).toBe(
        "卸载后的 B",
      ),
    );
    await flushPromises();

    expect(wantedRequests).toBe(1);
    expect(router.currentRoute.value.params.id).toBe("subject-b");
    expect(wrapper.get("#page-subscription-detail").text()).not.toContain("subject-a");
  });

  it.each([
    {
      label: "disabled",
      watcher: { enabled: false, dry_run: false },
      mode: "disabled",
      expected: ["订阅自动化已停用", "后台 watcher 不会自动搜索"],
    },
    {
      label: "dry-run",
      watcher: { enabled: true, dry_run: true },
      mode: "dry_run",
      expected: ["订阅自动化：试运行", "不会推送 qB 或创建硬链接"],
    },
    {
      label: "live",
      watcher: { enabled: true, dry_run: false },
      mode: "live",
      expected: ["订阅自动化：实时执行", "真实副作用", "推送 qB", "创建硬链接"],
    },
  ])(
    "renders the $label watcher mode from runtime settings",
    async ({ watcher, mode, expected }) => {
      const fetchMock = vi.fn(async (input, init = {}) => {
        const path = String(input);
        const method = String(init.method || "GET").toUpperCase();
        if (path === "/api/config") {
          return jsonResponse(configSnapshot({ subscription_watcher: watcher }));
        }
        if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
          return jsonResponse(subscriptionState());
        }
        throw new Error(`Unexpected request: ${method} ${path}`);
      });
      const { wrapper } = await mountAt("/subscriptions", fetchMock);
      const banner = wrapper.get(".subscription-watcher-banner");

      expect(banner.attributes("data-watcher-mode")).toBe(mode);
      for (const text of expected) expect(banner.text()).toContain(text);
    },
  );

  it("keeps watcher mode unknown until the runtime snapshot is loaded", async () => {
    let resolveConfig;
    const configResponse = new Promise((resolve) => {
      resolveConfig = resolve;
    });
    const fetchMock = vi.fn((input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return configResponse;
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return Promise.resolve(jsonResponse(subscriptionState()));
      }
      return Promise.reject(new Error(`Unexpected request: ${method} ${path}`));
    });
    const { wrapper } = await mountAt("/subscriptions", fetchMock);
    const banner = () => wrapper.get(".subscription-watcher-banner");

    expect(banner().attributes("data-watcher-mode")).toBe("loading");
    expect(banner().text()).toContain("加载完成前不会推断自动化模式");

    resolveConfig(
      jsonResponse(configSnapshot({ subscription_watcher: { enabled: true, dry_run: true } })),
    );
    await flushPromises();
    await vi.waitFor(() => expect(banner().attributes("data-watcher-mode")).toBe("dry_run"));
  });

  it("does not expose retired item actions from the detail page", async () => {
    const summary = subscriptionRecord();
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return jsonResponse(subscriptionState(summary));
      }
      if (path === "/api/subscriptions/wanted/subject-7" && method === "GET") {
        return jsonResponse(nestedSubscriptionDetail("subject-7", { summary }));
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper } = await mountAt("/subscriptions/subject-7", fetchMock);

    expect(wrapper.text()).not.toContain("重试当前节点");
    expect(wrapper.text()).not.toContain("重跑任务");
    expect(wrapper.find(".subscription-detail .row-actions").exists()).toBe(false);
    expect(fetchMock.mock.calls.map(([input]) => String(input))).toEqual([
      "/api/config",
      "/api/subscriptions/wanted?limit=100",
      "/api/subscriptions/wanted/subject-7",
    ]);
  });

  it("shows inactive, TV, and ordinary movie scheduling capabilities without item actions", async () => {
    const records = [
      subscriptionRecord({
        subject_id: "movie-1",
        title: "普通电影",
        lifecycle_state: "searching",
        updated_at: 300,
      }),
      subscriptionRecord({
        subject_id: "inactive-1",
        title: "历史订阅",
        active: false,
        schedulable: false,
        blocked_reason: "subscription_inactive",
        updated_at: 200,
      }),
      subscriptionRecord({
        subject_id: "tv-1",
        title: "未开放剧集",
        media_kind: "tv",
        schedulable: false,
        blocked_reason: "tv_not_supported",
        lifecycle_state: "downloading",
        updated_at: 100,
      }),
    ];
    const fetchMock = vi.fn(async (input, init = {}) => {
      const path = String(input);
      const method = String(init.method || "GET").toUpperCase();
      if (path === "/api/config") return jsonResponse(configSnapshot());
      if (path === "/api/subscriptions/wanted?limit=100" && method === "GET") {
        return jsonResponse(subscriptionState(records));
      }
      const detailMatch = path.match(/^\/api\/subscriptions\/wanted\/([^/]+)$/);
      if (detailMatch && method === "GET") {
        const id = decodeURIComponent(detailMatch[1]);
        const summary = records.find((record) => record.subject_id === id);
        if (summary) return jsonResponse(nestedSubscriptionDetail(id, { summary }));
      }
      throw new Error(`Unexpected request: ${method} ${path}`);
    });
    const { wrapper, router } = await mountAt("/subscriptions", fetchMock);
    const card = (title) =>
      wrapper.findAll(".subscription-card").find((node) => node.text().includes(title));

    expect(card("普通电影").text()).toContain("可调度");
    expect(card("历史订阅").text()).toContain("已停用");
    expect(card("历史订阅").text()).toContain("不可调度");
    expect(card("未开放剧集").text()).toContain("TV 未开放");
    expect(card("未开放剧集").text()).toContain("不可调度");

    await router.push({ name: "subscription-detail", params: { id: "inactive-1" } });
    await flushPromises();
    await vi.waitFor(() => expect(wrapper.get(".subscription-detail h3").text()).toBe("历史订阅"));
    expect(wrapper.get("#subscription-capability-note").text()).toContain("仅保留历史记录");
    expect(wrapper.find(".subscription-detail .row-actions").exists()).toBe(false);

    await router.push({ name: "subscription-detail", params: { id: "tv-1" } });
    await flushPromises();
    await vi.waitFor(() =>
      expect(wrapper.get(".subscription-detail h3").text()).toBe("未开放剧集"),
    );
    expect(wrapper.get("#subscription-capability-note").text()).toContain(
      "不会执行搜索、下载或硬链接",
    );
    expect(wrapper.find(".subscription-detail .row-actions").exists()).toBe(false);

    await router.push({ name: "subscription-detail", params: { id: "movie-1" } });
    await flushPromises();
    await vi.waitFor(() => expect(wrapper.get(".subscription-detail h3").text()).toBe("普通电影"));
    expect(wrapper.get("#subscription-capability-note").text()).toContain("后台任务调度");
    expect(wrapper.find(".subscription-detail .row-actions").exists()).toBe(false);
  });
});
