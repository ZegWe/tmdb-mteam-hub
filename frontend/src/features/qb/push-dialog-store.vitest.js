import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { createQbPushDialogStore } from "./push-dialog-store.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function settingsStore({
  servers = [{ id: "nas", name: "NAS", base_url: "http://127.0.0.1:8080" }],
  ensureRuntimeLoaded = vi.fn().mockResolvedValue(undefined),
} = {}) {
  return { runtimeQbServers: ref(servers), ensureRuntimeLoaded };
}

function transport(push = vi.fn().mockResolvedValue({ ok: true })) {
  return { push };
}

describe("qB push dialog store", () => {
  it("submits an ID-only payload and closes after success", async () => {
    const settings = settingsStore();
    const api = transport();
    const store = createQbPushDialogStore({ settingsStore: settings, transport: api });

    await store.openForTorrent({ id: "42", name: "Torrent" });
    store.form.category = "movie";
    store.form.savepath = "/downloads/movies";
    const result = await store.submit();

    expect(settings.ensureRuntimeLoaded).toHaveBeenCalledOnce();
    expect(api.push).toHaveBeenCalledWith(
      {
        server_id: "nas",
        torrent_id: "42",
        category: "movie",
        savepath: "/downloads/movies",
      },
      { signal: expect.any(AbortSignal) },
    );
    expect(result).toEqual({
      response: { ok: true },
      server: { id: "nas", name: "NAS", base_url: "http://127.0.0.1:8080" },
    });
    expect(store.open.value).toBe(false);
    expect(store.loading.value).toBe(false);
  });

  it("reports a stable error when no server is available", async () => {
    const api = transport();
    const store = createQbPushDialogStore({
      settingsStore: settingsStore({ servers: [] }),
      transport: api,
    });
    await store.openForTorrent({ id: "42" });

    await expect(store.submit()).rejects.toThrow("请先在 API 设置中配置 qB 服务器");

    expect(api.push).not.toHaveBeenCalled();
    expect(store.open.value).toBe(true);
    expect(store.loading.value).toBe(false);
  });

  it("reports a stable error when the selected server has no ID", async () => {
    const api = transport();
    const store = createQbPushDialogStore({
      settingsStore: settingsStore({ servers: [{ id: "  ", name: "Invalid" }] }),
      transport: api,
    });
    await store.openForTorrent({ id: "42" });

    await expect(store.submit()).rejects.toThrow("所选 qB 服务器无效");

    expect(api.push).not.toHaveBeenCalled();
    expect(store.open.value).toBe(true);
    expect(store.loading.value).toBe(false);
  });

  it("keeps the dialog open and restores loading after a current push failure", async () => {
    const api = transport(vi.fn().mockRejectedValue(new Error("qB push failed")));
    const store = createQbPushDialogStore({ settingsStore: settingsStore(), transport: api });
    await store.openForTorrent({ id: "42" });

    await expect(store.submit()).rejects.toThrow("qB push failed");

    expect(store.open.value).toBe(true);
    expect(store.loading.value).toBe(false);
  });

  it("coalesces rapid duplicate submissions while the first push is pending", async () => {
    const pending = deferred();
    const api = transport(vi.fn(() => pending.promise));
    const store = createQbPushDialogStore({ settingsStore: settingsStore(), transport: api });
    await store.openForTorrent({ id: "42" });

    const firstRequest = store.submit();
    await expect(store.submit()).resolves.toBeNull();
    expect(api.push).toHaveBeenCalledOnce();
    expect(store.loading.value).toBe(true);

    pending.resolve({ ok: true });
    await expect(firstRequest).resolves.toMatchObject({ response: { ok: true } });
    expect(store.open.value).toBe(false);
    expect(store.loading.value).toBe(false);
  });

  it("does not replace the active torrent while its push is pending", async () => {
    const pending = deferred();
    const settings = settingsStore();
    const api = transport(vi.fn(() => pending.promise));
    const store = createQbPushDialogStore({ settingsStore: settings, transport: api });
    await store.openForTorrent({ id: "42", name: "First Torrent" });
    const submitRequest = store.submit();

    await expect(
      store.openForTorrent({ id: "43", name: "Replacement Torrent" }),
    ).resolves.toBeNull();

    expect(settings.ensureRuntimeLoaded).toHaveBeenCalledOnce();
    expect(store.form.torrentId).toBe("42");
    expect(store.form.title).toBe("First Torrent");
    expect(store.open.value).toBe(true);
    expect(api.push).toHaveBeenCalledOnce();

    pending.resolve({ ok: true });
    await submitRequest;
    expect(store.open.value).toBe(false);
  });

  it("does not open when runtime settings loading fails", async () => {
    const ensureRuntimeLoaded = vi.fn().mockRejectedValue(new Error("settings unavailable"));
    const store = createQbPushDialogStore({
      settingsStore: settingsStore({ ensureRuntimeLoaded }),
      transport: transport(),
    });

    await expect(store.openForTorrent({ id: "42" })).rejects.toThrow("settings unavailable");

    expect(store.open.value).toBe(false);
    expect(store.form.torrentId).toBe("");
  });

  it("ignores a pending open after dispose", async () => {
    const pending = deferred();
    const ensureRuntimeLoaded = vi.fn(() => pending.promise);
    const store = createQbPushDialogStore({
      settingsStore: settingsStore({ ensureRuntimeLoaded }),
      transport: transport(),
    });
    const openRequest = store.openForTorrent({ id: "42", name: "Late Torrent" });

    store.dispose();
    pending.resolve();

    await expect(openRequest).resolves.toBeNull();
    expect(store.open.value).toBe(false);
    expect(store.form.torrentId).toBe("");
    await expect(store.openForTorrent({ id: "43" })).resolves.toBeNull();
    expect(ensureRuntimeLoaded).toHaveBeenCalledOnce();
  });

  it("aborts a pending submit on dispose and never returns its late success", async () => {
    const pending = deferred();
    let signal;
    const api = transport(
      vi.fn((_payload, options) => {
        signal = options.signal;
        return pending.promise;
      }),
    );
    const store = createQbPushDialogStore({ settingsStore: settingsStore(), transport: api });
    await store.openForTorrent({ id: "42" });
    const submitRequest = store.submit();

    store.dispose();

    expect(signal.aborted).toBe(true);
    expect(store.open.value).toBe(false);
    expect(store.loading.value).toBe(false);
    pending.resolve({ ok: true });
    await expect(submitRequest).resolves.toBeNull();
    await expect(store.submit()).resolves.toBeNull();
    expect(api.push).toHaveBeenCalledOnce();
  });
});
