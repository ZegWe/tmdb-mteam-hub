import { nextTick, reactive } from "vue";
import { describe, expect, it, vi } from "vitest";
import { createOperationLogFilters } from "./domain.js";
import {
  createLogsRouteSync,
  operationLogFiltersFromQuery,
  operationLogQueryFromFilters,
} from "./route.js";

function routeStore() {
  const filters = reactive(createOperationLogFilters());
  return {
    filters,
    setFilters: vi.fn((value) => Object.assign(filters, createOperationLogFilters(value))),
    applyFilters: vi.fn((value) => Object.assign(filters, createOperationLogFilters(value))),
    resetFilters: vi.fn(() => Object.assign(filters, createOperationLogFilters())),
    load: vi.fn().mockResolvedValue(null),
  };
}

function routerFor(route) {
  async function navigate(location) {
    route.name = location.name;
    route.query = location.query;
    await nextTick();
  }
  return { push: vi.fn(navigate), replace: vi.fn(navigate) };
}

describe("logs route synchronization", () => {
  it("normalizes deep-link query arrays and omits empty filters", () => {
    expect(
      operationLogFiltersFromQuery({
        category: [" SEARCH ", "ignored"],
        status: " FAILED ",
        q: "  电影  ",
      }),
    ).toEqual({ category: "search", status: "failed", q: "电影" });
    expect(operationLogQueryFromFilters({ category: "search", status: "", q: "电影" })).toEqual({
      category: "search",
      q: "电影",
    });
  });

  it("hydrates from a deep link and reloads once per route transition", async () => {
    const route = reactive({
      name: "logs",
      query: { category: "search", status: "failed", q: "first" },
    });
    const store = routeStore();
    const router = routerFor(route);
    const sync = createLogsRouteSync({ route, router, store });

    expect(store.filters).toEqual({ category: "search", status: "failed", q: "first" });
    expect(store.load).toHaveBeenCalledTimes(1);
    expect(store.load).toHaveBeenLastCalledWith({ page: 1, silent: true });

    store.filters.q = "second";
    await sync.applyFilters();
    await nextTick();

    expect(router.push).toHaveBeenCalledWith({
      name: "logs",
      query: { category: "search", status: "failed", q: "second" },
    });
    expect(store.load).toHaveBeenCalledTimes(2);
    expect(store.load).toHaveBeenLastCalledWith({ page: 1, silent: false });

    route.query = { category: "search", status: "failed", q: "first" };
    await nextTick();
    expect(store.filters.q).toBe("first");
    expect(store.load).toHaveBeenCalledTimes(3);
    expect(store.load).toHaveBeenLastCalledWith({ page: 1, silent: true });
    sync.dispose();
  });

  it("reloads directly for an unchanged query and resets through navigation", async () => {
    const route = reactive({ name: "logs", query: { q: "same" } });
    const store = routeStore();
    const router = routerFor(route);
    const sync = createLogsRouteSync({ route, router, store });
    store.load.mockClear();

    await sync.applyFilters();
    expect(router.push).not.toHaveBeenCalled();
    expect(store.load).toHaveBeenCalledOnce();
    expect(store.load).toHaveBeenCalledWith({ page: 1 });

    await sync.resetFilters();
    await nextTick();
    expect(router.push).toHaveBeenCalledWith({ name: "logs", query: {} });
    expect(store.load).toHaveBeenCalledTimes(2);
    sync.dispose();
  });
});
