import { describe, expect, it, vi } from "vitest";
import { createSearchContext } from "./context.js";

describe("search context", () => {
  it("exposes a one-shot detail origin without sharing result objects", () => {
    const store = { dispose: vi.fn() };
    const context = createSearchContext({ store });

    context.markDetailOpened();
    expect(context.consumeDetailOrigin()).toBe(true);
    expect(context.consumeDetailOrigin()).toBe(false);

    context.dispose();
    expect(store.dispose).toHaveBeenCalledOnce();
  });
});
