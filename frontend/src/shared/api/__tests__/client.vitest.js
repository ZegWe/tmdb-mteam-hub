import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, createApiClient } from "../client.js";
import { AUTH_SESSION_CHANGED_EVENT } from "../auth-session.js";

afterEach(() => {
  vi.restoreAllMocks();
});

function jsonResponse(value, init = {}) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

function abortablePendingFetch() {
  return vi.fn(
    (_url, { signal }) =>
      new Promise((_resolve, reject) => {
        const rejectFromAbort = () => reject(signal.reason);
        if (signal.aborted) rejectFromAbort();
        else signal.addEventListener("abort", rejectFromAbort, { once: true });
      }),
  );
}

describe("API client", () => {
  it("returns parsed JSON and leaves GET requests without a JSON content type", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    await expect(client.request("/api/health")).resolves.toEqual({ ok: true });

    const [url, init] = fetchImpl.mock.calls[0];
    const headers = new Headers(init.headers);
    expect(url).toBe("/api/health");
    expect(init.method).toBe("GET");
    expect(init.body).toBeUndefined();
    expect(headers.get("Accept")).toBe("application/json");
    expect(headers.has("Content-Type")).toBe(false);
  });

  it("serializes object bodies as JSON", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({ saved: true }));
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    await client.request("/api/items", { method: "POST", body: { title: "测试" } });

    const init = fetchImpl.mock.calls[0][1];
    expect(new Headers(init.headers).get("Content-Type")).toBe("application/json");
    expect(init.body).toBe(JSON.stringify({ title: "测试" }));
  });

  it("returns null for a successful empty response", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    await expect(client.request("/api/empty")).resolves.toBeNull();
  });

  it("reports invalid JSON with a stable error shape", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response("not-json", { status: 200 }));
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    const error = await client.request("/api/broken").catch((caught) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      name: "ApiError",
      code: "invalid_json",
      status: 200,
      path: "/api/broken",
    });
    expect(error.cause).toBeInstanceOf(SyntaxError);
  });

  it("preserves server error codes and details for HTTP failures", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      jsonResponse(
        {
          error: {
            code: "validation_failed",
            message: "配置无效",
            details: { field: "base_url" },
          },
        },
        { status: 422, statusText: "Unprocessable Entity" },
      ),
    );
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    await expect(client.request("/api/config")).rejects.toMatchObject({
      name: "ApiError",
      message: "配置无效",
      code: "validation_failed",
      status: 422,
      path: "/api/config",
      details: { field: "base_url" },
    });
  });

  it("notifies the authentication gate when a protected request returns 401", async () => {
    const listener = vi.fn();
    globalThis.addEventListener(AUTH_SESSION_CHANGED_EVENT, listener, { once: true });
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(jsonResponse({ error: { message: "unauthorized" } }, { status: 401 }));
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    await expect(client.request("/api/config")).rejects.toMatchObject({ status: 401 });

    expect(listener).toHaveBeenCalledOnce();
    expect(listener.mock.calls[0][0].detail).toEqual({
      authenticated: false,
      token_configured: false,
      bootstrap_allowed: false,
    });
  });

  it("wraps network failures and preserves the original cause", async () => {
    const cause = new TypeError("fetch failed");
    const fetchImpl = vi.fn().mockRejectedValue(cause);
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });

    const error = await client.request("/api/offline").catch((caught) => caught);
    expect(error).toMatchObject({
      name: "ApiError",
      code: "network_error",
      status: 0,
      path: "/api/offline",
    });
    expect(error.cause).toBe(cause);
  });

  it("aborts requests that exceed their timeout", async () => {
    vi.useFakeTimers();
    const fetchImpl = abortablePendingFetch();
    const client = createApiClient({ fetchImpl, timeoutMs: 25 });
    const request = client.request("/api/slow");
    const assertion = expect(request).rejects.toMatchObject({
      name: "ApiError",
      code: "request_timeout",
      path: "/api/slow",
    });

    await vi.advanceTimersByTimeAsync(25);
    await assertion;
    expect(fetchImpl.mock.calls[0][1].signal.aborted).toBe(true);
  });

  it("honors caller-provided abort signals", async () => {
    const fetchImpl = abortablePendingFetch();
    const client = createApiClient({ fetchImpl, timeoutMs: 0 });
    const controller = new AbortController();
    const request = client.request("/api/cancelled", { signal: controller.signal });

    controller.abort(new DOMException("Caller cancelled", "AbortError"));

    const error = await request.catch((caught) => caught);
    expect(error).toMatchObject({
      name: "ApiError",
      code: "request_aborted",
      path: "/api/cancelled",
    });
    expect(error.cause).toBe(controller.signal.reason);
  });
});
