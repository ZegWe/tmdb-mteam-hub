import { describe, expect, it, vi } from "vitest";
import { createSubscriptionContext } from "./context.js";

function storeMock() {
  return {
    start: vi.fn().mockResolvedValue(null),
    stop: vi.fn(),
    dispose: vi.fn(),
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe("subscription route context", () => {
  it("keeps one polling lifecycle across list and detail route handoff", async () => {
    const scheduled = [];
    const store = storeMock();
    const context = createSubscriptionContext({
      store,
      scheduleMicrotask: (callback) => scheduled.push(callback),
    });

    await context.enterRoute();
    context.leaveRoute();
    await context.enterRoute();
    scheduled.splice(0).forEach((callback) => callback());

    expect(store.start).toHaveBeenCalledOnce();
    expect(store.stop).not.toHaveBeenCalled();
    expect(context.activeRouteCount.value).toBe(1);

    context.leaveRoute();
    scheduled.splice(0).forEach((callback) => callback());
    expect(store.stop).toHaveBeenCalledOnce();
  });

  it("disposes the shared store with the app shell", () => {
    const store = storeMock();
    const context = createSubscriptionContext({ store });

    context.dispose();

    expect(store.dispose).toHaveBeenCalledOnce();
    expect(context.activeRouteCount.value).toBe(0);
  });

  it("shares the pending initial refresh across a same-tree route handoff", async () => {
    const initial = deferred();
    const store = storeMock();
    store.start.mockReturnValue(initial.promise);
    const context = createSubscriptionContext({ store });

    const listEntry = context.enterRoute();
    const detailEntry = context.enterRoute();

    expect(store.start).toHaveBeenCalledOnce();
    const latestState = { next_cursor: null, ordered_ids: [], records: {} };
    initial.resolve(latestState);
    await expect(Promise.all([listEntry, detailEntry])).resolves.toEqual([
      latestState,
      latestState,
    ]);
  });
});
