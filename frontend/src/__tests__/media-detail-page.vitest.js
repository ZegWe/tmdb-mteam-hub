import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App.vue";

const EmptyRoute = { template: "" };
let activeWrapper = null;

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  vi.restoreAllMocks();
});

function jsonResponse(value) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function configSnapshot() {
  return {
    revision: 1,
    has_tmdb_api_key: true,
    has_mteam_api_key: true,
    has_douban_cookie: false,
    qb_servers: [
      {
        id: "nas",
        name: "NAS",
        base_url: "http://qb.internal:8080",
        username: "admin",
        has_password: true,
        insecure_tls: false,
      },
    ],
    subscription_categories: [],
    torrent_match_rules: [],
  };
}

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "main", component: EmptyRoute },
      {
        path: "/detail/:mediaType/:id",
        name: "media-detail",
        component: () => import("../pages/MediaDetailPage.vue"),
      },
    ],
  });
}

function detailFetchMock(
  config = configSnapshot(),
  { torrents = [{ id: "torrent-7", name: "测试种子" }], pushResponse = null } = {},
) {
  return vi.fn(async (input) => {
    const path = String(input);
    if (path === "/api/config") return jsonResponse(config);
    if (path === "/api/tmdb/movie/42") {
      return jsonResponse({
        id: 42,
        title: "推送电影",
        original_title: "Push Movie",
      });
    }
    if (path.startsWith("/api/douban/tags?")) return jsonResponse({ tags: [] });
    if (path.startsWith("/api/mteam/torrents?")) {
      return jsonResponse({ items: torrents, page: 1, page_size: 50 });
    }
    if (path === "/api/qb/push-mteam") return pushResponse || jsonResponse({ ok: true });
    throw new Error(`Unexpected request: ${path}`);
  });
}

async function mountDetail(fetchMock = detailFetchMock()) {
  vi.stubGlobal("fetch", fetchMock);
  const router = createTestRouter();
  await router.push("/detail/movie/42");
  await router.isReady();
  activeWrapper = mount(App, {
    attachTo: document.body,
    global: { plugins: [router] },
  });
  await flushPromises();
  return { wrapper: activeWrapper, fetchMock };
}

async function openDialog(wrapper) {
  const trigger = wrapper.get(".torrent-push-trigger");
  await trigger.trigger("click");
  await flushPromises();
  return { dialog: wrapper.get("dialog"), trigger };
}

describe("MediaDetailPage qB dialog", () => {
  it("uses showModal once, handles native cancel/close-button through one close path, and restores focus", async () => {
    const showModal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
    const close = vi.spyOn(HTMLDialogElement.prototype, "close");
    const { wrapper } = await mountDetail();
    const trigger = wrapper.get(".torrent-push-trigger");
    const dialog = wrapper.get("dialog");
    const triggerFocus = vi.spyOn(trigger.element, "focus");
    const initialFocus = vi.spyOn(dialog.get("select").element, "focus");
    const firstOpen = await openDialog(wrapper);

    expect(showModal).toHaveBeenCalledOnce();
    expect(firstOpen.dialog.element.hasAttribute("open")).toBe(true);
    expect(initialFocus).toHaveBeenCalled();
    expect(document.activeElement).toBe(firstOpen.dialog.get("select").element);

    await firstOpen.trigger.trigger("click");
    await flushPromises();
    expect(showModal).toHaveBeenCalledOnce();

    const cancelEvent = new Event("cancel", { bubbles: false, cancelable: true });
    firstOpen.dialog.element.dispatchEvent(cancelEvent);
    await flushPromises();

    expect(cancelEvent.defaultPrevented).toBe(true);
    expect(close).toHaveBeenCalledTimes(1);
    expect(firstOpen.dialog.element.hasAttribute("open")).toBe(false);
    expect(triggerFocus).toHaveBeenCalled();
    expect(document.activeElement).toBe(firstOpen.trigger.element);

    const secondOpen = await openDialog(wrapper);
    expect(showModal).toHaveBeenCalledTimes(2);
    const cancelButton = secondOpen.dialog
      .findAll('button[type="button"]')
      .find((button) => button.text() === "取消");
    expect(cancelButton).toBeTruthy();
    await cancelButton.trigger("click");
    await flushPromises();

    expect(close).toHaveBeenCalledTimes(2);
    expect(secondOpen.dialog.element.hasAttribute("open")).toBe(false);
    expect(document.activeElement).toBe(secondOpen.trigger.element);
  });

  it("submits only server and torrent identifiers plus explicit options", async () => {
    const showModal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
    const close = vi.spyOn(HTMLDialogElement.prototype, "close");
    const fetchMock = detailFetchMock();
    const { wrapper } = await mountDetail(fetchMock);
    const { dialog, trigger } = await openDialog(wrapper);
    const triggerFocus = vi.spyOn(trigger.element, "focus");
    const inputs = dialog.findAll("input");

    expect(showModal).toHaveBeenCalledOnce();
    await inputs[0].setValue("movie");
    await inputs[1].setValue("/downloads/movies");
    await dialog.get("form").trigger("submit");
    await flushPromises();

    const pushCall = fetchMock.mock.calls.find(([input]) => String(input) === "/api/qb/push-mteam");
    expect(pushCall).toBeTruthy();
    expect(JSON.parse(pushCall[1].body)).toEqual({
      server_id: "nas",
      torrent_id: "torrent-7",
      category: "movie",
      savepath: "/downloads/movies",
    });
    expect(close).toHaveBeenCalledOnce();
    expect(dialog.element.hasAttribute("open")).toBe(false);
    expect(triggerFocus).toHaveBeenCalled();
    expect(document.activeElement).toBe(trigger.element);
  });

  it("keeps the original torrent and return-focus trigger while its push is pending", async () => {
    const pendingPush = deferred();
    const close = vi.spyOn(HTMLDialogElement.prototype, "close");
    const fetchMock = detailFetchMock(configSnapshot(), {
      torrents: [
        { id: "torrent-1", name: "First Torrent" },
        { id: "torrent-2", name: "Second Torrent" },
      ],
      pushResponse: pendingPush.promise,
    });
    const { wrapper } = await mountDetail(fetchMock);
    const triggers = wrapper.findAll(".torrent-push-trigger");
    const firstFocus = vi.spyOn(triggers[0].element, "focus");
    const secondFocus = vi.spyOn(triggers[1].element, "focus");

    await triggers[0].trigger("click");
    await flushPromises();
    const dialog = wrapper.get("dialog");
    expect(dialog.get(".hint").text()).toContain("First Torrent");
    await dialog.get("form").trigger("submit");
    await flushPromises();

    await triggers[1].trigger("click");
    await flushPromises();

    expect(dialog.get(".hint").text()).toContain("First Torrent");
    expect(dialog.get(".hint").text()).not.toContain("Second Torrent");
    expect(
      fetchMock.mock.calls.filter(([input]) => String(input) === "/api/qb/push-mteam"),
    ).toHaveLength(1);

    pendingPush.resolve(jsonResponse({ ok: true }));
    await flushPromises();

    expect(close).toHaveBeenCalledOnce();
    expect(firstFocus).toHaveBeenCalled();
    expect(secondFocus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(triggers[0].element);
  });

  it("skips the disabled server select when no qB server is configured", async () => {
    const showModal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
    const { wrapper } = await mountDetail(detailFetchMock({ ...configSnapshot(), qb_servers: [] }));
    const dialog = wrapper.get("dialog");
    const serverSelect = dialog.get("select");
    const categoryInput = dialog.findAll("input")[0];
    const selectFocus = vi.spyOn(serverSelect.element, "focus");
    const inputFocus = vi.spyOn(categoryInput.element, "focus");

    await openDialog(wrapper);

    expect(showModal).toHaveBeenCalledOnce();
    expect(serverSelect.attributes("disabled")).toBeDefined();
    expect(selectFocus).not.toHaveBeenCalled();
    expect(inputFocus).toHaveBeenCalled();
    expect(document.activeElement).toBe(categoryInput.element);
  });

  it("converges an external native close back into the store and cleans up an open dialog on unmount", async () => {
    const showModal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
    const close = vi.spyOn(HTMLDialogElement.prototype, "close");
    const { wrapper } = await mountDetail();
    const firstOpen = await openDialog(wrapper);

    firstOpen.dialog.element.close();
    await flushPromises();
    expect(firstOpen.dialog.element.open).toBe(false);

    await firstOpen.trigger.trigger("click");
    await flushPromises();
    expect(showModal).toHaveBeenCalledTimes(2);
    expect(firstOpen.dialog.element.open).toBe(true);

    close.mockClear();
    wrapper.unmount();
    activeWrapper = null;
    expect(close).toHaveBeenCalledOnce();
    expect(firstOpen.dialog.element.open).toBe(false);
  });
});
