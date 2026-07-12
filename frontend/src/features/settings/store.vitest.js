import { describe, expect, it, vi } from "vitest";
import { createSettingsStore } from "./store.js";

function subscriptionWatcher(overrides = {}) {
  return {
    enabled: false,
    dry_run: true,
    poll_interval_secs: 4321,
    library_limit: 77,
    max_retries: 5,
    search_interval_secs: 654,
    progress_interval_secs: 9,
    link_retry_interval_secs: 876,
    system_retry_interval_secs: 321,
    bootstrap_existing_as_skipped: false,
    ...overrides,
  };
}

function configSnapshot(overrides = {}) {
  return {
    revision: 7,
    has_tmdb_api_key: true,
    has_mteam_api_key: true,
    has_douban_cookie: true,
    qb_servers: [
      {
        id: "nas",
        name: "NAS",
        base_url: "http://127.0.0.1:8080",
        username: "admin",
        insecure_tls: false,
        has_password: true,
      },
    ],
    subscription_categories: [{ name: "电影", wanted_tag: "movie", qb_server_id: "nas" }],
    subscription_watcher: subscriptionWatcher(),
    torrent_match_rules: [],
    restart_required: false,
    ...overrides,
  };
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

function transport(overrides = {}) {
  return {
    load: vi.fn().mockResolvedValue(configSnapshot()),
    save: vi.fn().mockResolvedValue(configSnapshot({ revision: 8 })),
    testQb: vi.fn().mockResolvedValue({ version: "5.0.4" }),
    startQr: vi.fn().mockResolvedValue({ session_id: "qr", image_url: "/qr" }),
    pollQr: vi.fn().mockResolvedValue({ done: false, description: "等待扫码" }),
    authStatus: vi.fn().mockResolvedValue({
      authenticated: true,
      token_configured: false,
      bootstrap_allowed: true,
    }),
    login: vi.fn().mockResolvedValue({
      authenticated: true,
      token_configured: true,
      bootstrap_allowed: false,
    }),
    ...overrides,
  };
}

describe("settings store", () => {
  it("keeps runtime summaries isolated from drafts until save succeeds", async () => {
    const pendingSave = deferred();
    const requests = transport({ save: vi.fn(() => pendingSave.promise) });
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    expect(store.runtimeQbServers.value[0].base_url).toBe("http://127.0.0.1:8080");
    expect(store.runtimeSubscriptionCategories.value[0].wanted_tag).toBe("movie");

    store.form.qb_servers[0].base_url = "http://127.0.0.1:9090";
    store.form.subscription_categories[0].wanted_tag = "new-movie";
    store.form.mteam_api_key = "replacement-key";
    store.form.douban_cookie = "must-not-be-sent";
    store.clearSecrets.douban_cookie = true;

    const saveRequest = store.save();
    expect(requests.save).toHaveBeenCalledWith({
      expected_revision: 7,
      mteam_api_key: "replacement-key",
      clear_douban_cookie: true,
      qb_servers: [
        {
          id: "nas",
          name: "NAS",
          base_url: "http://127.0.0.1:9090",
          username: "admin",
          insecure_tls: false,
        },
      ],
      subscription_categories: [
        {
          name: "电影",
          wanted_tag: "new-movie",
          qb_server_id: "nas",
          qb_category: "",
          qb_save_dir_name: "",
          download_dir: "",
          link_target_dir: "",
        },
      ],
      subscription_watcher: subscriptionWatcher(),
      torrent_match_rules: [],
    });
    expect(store.runtimeQbServers.value[0].base_url).toBe("http://127.0.0.1:8080");
    expect(store.runtimeSubscriptionCategories.value[0].wanted_tag).toBe("movie");

    pendingSave.resolve(
      configSnapshot({
        revision: 8,
        qb_servers: [
          {
            id: "nas",
            name: "NAS",
            base_url: "http://127.0.0.1:9090",
            username: "admin",
            insecure_tls: false,
            has_password: true,
          },
        ],
        subscription_categories: [{ name: "电影", wanted_tag: "new-movie", qb_server_id: "nas" }],
      }),
    );
    await saveRequest;

    expect(store.runtimeQbServers.value[0].base_url).toBe("http://127.0.0.1:9090");
    expect(store.runtimeSubscriptionCategories.value[0].wanted_tag).toBe("new-movie");
    expect(store.revision.value).toBe(8);
    expect(store.form.mteam_api_key).toBe("");
    expect(store.form.douban_cookie).toBe("");
  });

  it("does not save when enabling automation is not confirmed", async () => {
    const requests = transport();
    const confirmEnableAutomation = vi.fn(() => false);
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    expect(store.form.subscription_watcher).toEqual({ enabled: false, dry_run: true });
    expect(store.runtimeSubscriptionWatcher.value).toEqual(subscriptionWatcher());
    store.form.subscription_watcher.enabled = true;
    store.form.subscription_watcher.dry_run = false;
    expect(store.runtimeSubscriptionWatcher.value).toEqual(subscriptionWatcher());

    await expect(store.save({ confirmEnableAutomation })).resolves.toBeNull();

    expect(confirmEnableAutomation).toHaveBeenCalledOnce();
    expect(requests.save).not.toHaveBeenCalled();
  });

  it("uses a replacement management token once to refresh the HttpOnly session", async () => {
    const replacement = "replacement-management-token-123456";
    const requests = transport({
      save: vi.fn().mockResolvedValue(
        configSnapshot({
          revision: 8,
          has_admin_token: true,
        }),
      ),
    });
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    store.form.admin_token = replacement;
    await store.save();

    expect(requests.save.mock.calls[0][0]).toMatchObject({ admin_token: replacement });
    expect(requests.login).toHaveBeenCalledOnce();
    expect(requests.login).toHaveBeenCalledWith(replacement);
    expect(store.form.admin_token).toBe("");
    expect(store.secretPresence.admin_token).toBe(true);
  });

  it("refreshes bootstrap authentication after explicitly clearing the management token", async () => {
    const requests = transport({
      load: vi.fn().mockResolvedValue(configSnapshot({ has_admin_token: true })),
      save: vi.fn().mockResolvedValue(
        configSnapshot({
          revision: 8,
          has_admin_token: false,
        }),
      ),
    });
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    store.clearSecrets.admin_token = true;
    await store.save();

    expect(requests.save.mock.calls[0][0]).toMatchObject({ clear_admin_token: true });
    expect(requests.authStatus).toHaveBeenCalledOnce();
    expect(requests.login).not.toHaveBeenCalled();
  });

  it("does not misreport a persisted token rotation as an unsaved configuration", async () => {
    const requests = transport({
      save: vi.fn().mockResolvedValue(
        configSnapshot({
          revision: 8,
          has_admin_token: true,
        }),
      ),
      login: vi.fn().mockRejectedValue(new Error("session refresh failed")),
    });
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    store.form.admin_token = "replacement-management-token-123456";
    await expect(store.save()).resolves.toMatchObject({ revision: 8, has_admin_token: true });

    expect(store.revision.value).toBe(8);
    expect(store.form.admin_token).toBe("");
    expect(store.saveStatus.message).toContain("已更新");
    expect(store.saveStatus.message).toContain("重新登录失败");
  });

  it("adds explicit confirmation while preserving the complete watcher configuration", async () => {
    const requests = transport({
      save: vi.fn().mockResolvedValue(
        configSnapshot({
          revision: 8,
          subscription_watcher: subscriptionWatcher({ enabled: true, dry_run: false }),
        }),
      ),
    });
    const confirmEnableAutomation = vi.fn(() => true);
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    store.form.subscription_watcher.enabled = true;
    store.form.subscription_watcher.dry_run = false;
    await store.save({ confirmEnableAutomation });

    expect(confirmEnableAutomation).toHaveBeenCalledOnce();
    const payload = requests.save.mock.calls[0][0];
    expect(payload.confirm_enable_automation).toBe(true);
    expect(payload.subscription_watcher).toEqual(
      subscriptionWatcher({ enabled: true, dry_run: false }),
    );
    expect(store.runtimeSubscriptionWatcher.value).toEqual(
      subscriptionWatcher({ enabled: true, dry_run: false }),
    );
  });

  it("saves an already-enabled watcher without confirmation or a confirmation field", async () => {
    const requests = transport({
      load: vi
        .fn()
        .mockResolvedValue(
          configSnapshot({ subscription_watcher: subscriptionWatcher({ enabled: true }) }),
        ),
      save: vi.fn().mockResolvedValue(
        configSnapshot({
          revision: 8,
          subscription_watcher: subscriptionWatcher({ enabled: true, dry_run: false }),
        }),
      ),
    });
    const confirmEnableAutomation = vi.fn(() => true);
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();

    expect(store.form.subscription_watcher.enabled).toBe(true);
    store.form.subscription_watcher.dry_run = false;
    await store.save({ confirmEnableAutomation });

    expect(confirmEnableAutomation).not.toHaveBeenCalled();
    const payload = requests.save.mock.calls[0][0];
    expect(payload).not.toHaveProperty("confirm_enable_automation");
    expect(payload.subscription_watcher).toEqual(
      subscriptionWatcher({ enabled: true, dry_run: false }),
    );
  });

  it("tests only an unchanged saved qB draft and sends its ID only", async () => {
    const requests = transport();
    const store = createSettingsStore({ transport: requests });
    await store.enterPage();
    const server = store.form.qb_servers[0];

    await store.testQbServer(server);
    expect(requests.testQb).toHaveBeenCalledWith({ server_id: "nas" });

    server.base_url = "http://127.0.0.1:9090";
    await store.testQbServer(server);
    expect(requests.testQb).toHaveBeenCalledTimes(1);
    expect(server.testMessage).toBe("请先保存后测试");
  });

  it("clears the QR poll timer and sensitive drafts when the page leaves", async () => {
    const requests = transport();
    const setIntervalFn = vi.fn(() => 41);
    const clearIntervalFn = vi.fn();
    const store = createSettingsStore({ transport: requests, setIntervalFn, clearIntervalFn });
    await store.enterPage();
    store.form.tmdb_api_key = "draft-secret";

    await store.startQrLogin();
    expect(setIntervalFn).toHaveBeenCalledOnce();
    expect(store.qrImage.value).toContain("/qr&t=");

    store.leavePage();
    expect(clearIntervalFn).toHaveBeenCalledWith(41);
    expect(store.qrImage.value).toBe("");
    expect(store.form.tmdb_api_key).toBe("");
    expect(store.pageLoaded.value).toBe(false);
  });
});
