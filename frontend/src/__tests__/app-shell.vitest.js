import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { describe, expect, it, vi } from "vitest";
import App from "../App.vue";
import { createAppRoutes } from "../app/routes.js";

function createTestRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: createAppRoutes(),
  });
}

describe("App shell", () => {
  it("mounts the current shell with navigation and initial configuration loading", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          revision: 1,
          has_tmdb_api_key: false,
          has_mteam_api_key: false,
          has_douban_cookie: false,
          qb_servers: [],
          subscription_categories: [],
          torrent_match_rules: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const router = createTestRouter();
    await router.push("/");
    await router.isReady();

    const wrapper = mount(App, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();

    expect(wrapper.get(".app-shell").exists()).toBe(true);
    expect(wrapper.get(".brand h1").text()).toBe("影视检索");
    expect(wrapper.findAll(".nav-item")).toHaveLength(4);
    expect(router.currentRoute.value.name).toBe("main");
    expect(wrapper.get(".nav-item.is-active").text()).toBe("主功能");
    expect(wrapper.get(".nav-item.is-active").attributes("aria-current")).toBe("page");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/config");
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("Accept")).toBe("application/json");

    wrapper.unmount();
  });

  it("resolves stable navigation ownership for lists, details, and not-found", () => {
    const router = createTestRouter();
    const expectations = [
      ["/", "main"],
      ["/detail/movie/42", "main"],
      ["/subscriptions", "subscriptions"],
      ["/subscriptions/subject-7", "subscriptions"],
      ["/logs", "logs"],
      ["/settings", "settings"],
      ["/missing/path", ""],
    ];

    for (const [path, navPage] of expectations) {
      expect(router.resolve(path).meta.navPage).toBe(navPage);
    }
  });
});
