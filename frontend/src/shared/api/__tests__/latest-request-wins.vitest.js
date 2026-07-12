import { describe, expect, it } from "vitest";
import { createLatestRequestWins, StaleRequestError } from "../latest-request-wins.js";

describe("latest request wins", () => {
  it("aborts the prior signal and rejects a late prior result as stale", async () => {
    const latest = createLatestRequestWins();
    let resolveOlder;
    let olderSignal;
    const older = latest.run(({ signal }) => {
      olderSignal = signal;
      return new Promise((resolve) => {
        resolveOlder = resolve;
      });
    });
    const staleAssertion = expect(older).rejects.toBeInstanceOf(StaleRequestError);

    const newer = latest.run(() => Promise.resolve("newer"));

    expect(olderSignal.aborted).toBe(true);
    await expect(newer).resolves.toBe("newer");
    resolveOlder("older");
    await staleAssertion;
  });

  it("passes through a caller abort for the current request", async () => {
    const latest = createLatestRequestWins();
    const caller = new AbortController();
    const request = latest.run(
      ({ signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(signal.reason), { once: true });
        }),
      { signal: caller.signal },
    );

    caller.abort(new DOMException("Caller cancelled", "AbortError"));

    await expect(request).rejects.toMatchObject({ name: "AbortError" });
  });
});
