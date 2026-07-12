import { describe, expect, it } from "vitest";
import {
  DEFAULT_SUBSCRIPTION_WATCHER,
  createQbServerDrafts,
  createSettingsForm,
  isSavedQbServerDraft,
  qbServerFingerprints,
  settingsFormFromSnapshot,
  settingsUpdatePayload,
  subscriptionWatcherRuntimeBanner,
} from "./form-model.js";

describe("settings form model", () => {
  it("uses safe watcher defaults and maps only its editable switches into the draft", () => {
    expect(createSettingsForm().subscription_watcher).toEqual({
      enabled: false,
      dry_run: true,
    });

    const model = settingsFormFromSnapshot({
      subscription_watcher: {
        enabled: true,
        dry_run: false,
        poll_interval_secs: 4321,
        library_limit: 77,
        max_retries: 5,
        search_interval_secs: 654,
        progress_interval_secs: 9,
        link_retry_interval_secs: 876,
        system_retry_interval_secs: 321,
        bootstrap_existing_as_skipped: false,
      },
    });

    expect(model.form.subscription_watcher).toEqual({
      enabled: true,
      dry_run: false,
    });
  });

  it("describes unloaded, disabled, dry-run, and live watcher runtime modes", () => {
    expect(subscriptionWatcherRuntimeBanner()).toEqual({
      key: "loading",
      tone: "info",
      title: "订阅自动化状态读取中",
      message: "正在读取后端运行配置；加载完成前不会推断自动化模式。",
    });
    expect(
      subscriptionWatcherRuntimeBanner({
        runtimeLoaded: true,
        watcher: { enabled: false, dry_run: false },
      }),
    ).toMatchObject({
      key: "disabled",
      title: "订阅自动化已停用",
    });
    const dryRun = subscriptionWatcherRuntimeBanner({
      runtimeLoaded: true,
      watcher: { enabled: true, dry_run: true },
    });
    expect(dryRun).toMatchObject({ key: "dry_run", title: "订阅自动化：试运行" });
    expect(dryRun.message).toContain("不会推送 qB 或创建硬链接");

    const live = subscriptionWatcherRuntimeBanner({
      runtimeLoaded: true,
      watcher: { enabled: true, dry_run: false },
    });
    expect(live).toMatchObject({ key: "live", title: "订阅自动化：实时执行" });
    expect(live.message).toContain("真实副作用");
    expect(live.message).toContain("推送 qB");
    expect(live.message).toContain("创建硬链接");
  });

  it("normalizes redacted snapshots without copying response secrets into drafts", () => {
    const model = settingsFormFromSnapshot({
      revision: 7,
      has_tmdb_api_key: true,
      qb_servers: [
        {
          id: "nas",
          name: "NAS",
          base_url: "http://127.0.0.1:8080",
          username: "admin",
          password: "response-secret-must-be-dropped",
          has_password: true,
        },
      ],
      subscription_categories: [{ name: "电影", wanted_tag: "movie" }],
      torrent_match_rules: [{ title_keywords: ["REMUX", "BluRay"] }],
    });

    expect(model.revision).toBe(7);
    expect(model.secretPresence.tmdb_api_key).toBe(true);
    expect(model.form.tmdb_api_key).toBe("");
    expect(model.form.qb_servers[0]).toMatchObject({
      id: "nas",
      password: "",
      has_password: true,
      clear_password: false,
    });
    expect(model.form.subscription_categories[0].qb_server_id).toBe("nas");
    expect(model.form.torrent_match_rules[0].title_keywords_text).toBe("REMUX, BluRay");
    expect(JSON.stringify(model)).not.toContain("response-secret-must-be-dropped");
  });

  it("builds one update DTO with Keep, Set, Clear and normalized nested forms", () => {
    const form = createSettingsForm();
    form.tmdb_api_key = "";
    form.mteam_api_key = "replacement-mteam-key";
    form.douban_cookie = "must-not-be-sent";
    form.qb_servers = [
      {
        id: " nas ",
        name: " NAS ",
        base_url: " http://127.0.0.1:8080 ",
        username: " admin ",
        password: "",
        has_password: true,
      },
      { id: "empty", base_url: "" },
    ];
    form.subscription_categories = [
      { name: " 电影 ", wanted_tag: " movie ", qb_server_id: " nas " },
      {},
    ];
    form.torrent_match_rules = [
      { name: " REMUX ", priority: "5", mode: "any", title_keywords_text: "REMUX，BluRay" },
      {},
    ];

    expect(
      settingsUpdatePayload({
        form,
        expectedRevision: 7,
        clearSecrets: { douban_cookie: true },
      }),
    ).toEqual({
      expected_revision: 7,
      mteam_api_key: "replacement-mteam-key",
      clear_douban_cookie: true,
      qb_servers: [
        {
          id: "nas",
          name: "NAS",
          base_url: "http://127.0.0.1:8080",
          username: "admin",
          insecure_tls: false,
        },
      ],
      subscription_categories: [
        {
          name: "电影",
          wanted_tag: "movie",
          qb_server_id: "nas",
          qb_category: "",
          qb_save_dir_name: "",
          download_dir: "",
          link_target_dir: "",
        },
      ],
      subscription_watcher: { ...DEFAULT_SUBSCRIPTION_WATCHER },
      torrent_match_rules: [
        {
          name: "REMUX",
          priority: 5,
          mode: "any",
          title_keywords: ["REMUX", "BluRay"],
          resolution_keywords: [],
          source_keywords: [],
        },
      ],
    });
  });

  it("tracks qB draft identity by its safe patch fingerprint", () => {
    const [draft] = createQbServerDrafts([
      {
        id: "nas",
        base_url: "http://127.0.0.1:8080",
        has_password: true,
      },
    ]);
    const fingerprints = qbServerFingerprints([draft]);

    expect(isSavedQbServerDraft(draft, fingerprints)).toBe(true);
    draft.password = "replacement";
    expect(isSavedQbServerDraft(draft, fingerprints)).toBe(false);
    draft.password = "";
    draft.base_url = "http://127.0.0.1:9090";
    expect(isSavedQbServerDraft(draft, fingerprints)).toBe(false);
  });
});
