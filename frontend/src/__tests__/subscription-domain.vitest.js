import { describe, expect, it } from "vitest";
import {
  subscriptionAttentionKey,
  subscriptionCardSubtitle,
  subscriptionCapabilities,
  subscriptionDetailRows,
  subscriptionDisplayStatus,
  subscriptionLifecycleKey,
  subscriptionLifecycleNodes,
  subscriptionPollToast,
  subscriptionSummary,
} from "../features/subscriptions/domain.js";

describe("subscription domain display", () => {
  it("uses explicit lifecycle and attention fields", () => {
    const record = {
      subject_id: "semantic-state",
      lifecycle_state: "searching",
      attention_tags: [],
    };

    expect(subscriptionLifecycleKey(record)).toBe("searching");
    expect(subscriptionDisplayStatus(record)).toEqual({ key: "searching", text: "搜索中" });
    expect(subscriptionAttentionKey(record)).toBe("");
  });

  it("uses summary fields for card subtitles", () => {
    expect(subscriptionCardSubtitle({ release_year: 2026, subject_id: "subject-1" })).toBe("2026");
    expect(subscriptionCardSubtitle({ release_year: null, subject_id: "subject-1" })).toBe(
      "subject-1",
    );
  });

  it("describes scheduling capabilities for inactive, TV, blocked, and ordinary movies", () => {
    const movie = subscriptionCapabilities({
      subject_id: "movie-1",
      active: true,
      media_kind: "movie",
      schedulable: true,
      blocked_reason: null,
      lifecycle_state: "searching",
    });
    expect(movie.badges).toEqual([{ key: "schedulable", text: "可调度", tone: "success" }]);
    expect(movie.explanation).toContain("后台任务调度");
    expect(movie).not.toHaveProperty("actions");

    const inactive = subscriptionCapabilities({
      subject_id: "inactive-1",
      active: false,
      media_kind: "movie",
      schedulable: true,
      lifecycle_state: "searching",
    });
    expect(inactive.badges.map((badge) => badge.text)).toEqual(["已停用", "不可调度"]);
    expect(inactive.explanation).toContain("仅保留历史记录");

    const tvRecord = {
      subject_id: "tv-1",
      active: true,
      media_kind: "tv",
      schedulable: false,
      blocked_reason: "tv_not_supported",
      lifecycle_state: "downloading",
    };
    const tv = subscriptionCapabilities(tvRecord);
    expect(tv.badges.map((badge) => badge.text)).toEqual(["TV 未开放", "不可调度"]);
    expect(tv.explanation).toContain("不会执行搜索、下载或硬链接");

    const blocked = subscriptionCapabilities({
      subject_id: "blocked-1",
      active: true,
      media_kind: "movie",
      schedulable: false,
      blocked_reason: "operator_hold",
      lifecycle_state: "searching",
    });
    expect(blocked.badges.map((badge) => badge.text)).toEqual([
      "自动处理受限：operator hold",
      "不可调度",
    ]);
    expect(blocked.explanation).toContain("后台任务会等待状态恢复");
  });

  it("summarizes lifecycle and attention independently", () => {
    const summary = subscriptionSummary([
      { lifecycle_state: "queued" },
      { lifecycle_state: "meta" },
      { lifecycle_state: "searching", attention_tags: ["waiting_release"] },
      { lifecycle_state: "downloading" },
      { lifecycle_state: "linking", attention_tags: ["failed"] },
      { lifecycle_state: "completed", attention_tags: ["skipped"] },
    ]);

    for (const expected of [
      "总计 6",
      "待处理 1",
      "元数据 1",
      "搜索中 1",
      "下载中 1",
      "硬链接中 1",
      "完成 1",
      "失败 1",
      "跳过 1",
      "等待发布 1",
    ]) {
      expect(summary).toContain(expected);
    }
    expect(summary).not.toMatch(/unprocessed|pushed|downloaded|待链接/);
  });

  it("reads media and observation rows from the nested detail DTO", () => {
    const rows = subscriptionDetailRows({
      subject_id: "nested-1",
      release_year: 2026,
      active: true,
      media_kind: "movie",
      schedulable: true,
      retry_count: 1,
      max_retries: 3,
      updated_at: 300,
      source: {
        date_published: "2026-07-11",
        rating_value: 8.8,
        rating_count: 1200,
        original_title: "Nested Title",
        aka: ["别名"],
        genres: ["剧情"],
        countries: ["中国"],
        languages: ["中文"],
        directors: ["导演"],
        actors: ["演员"],
        duration: "120 分钟",
        synopsis: "嵌套简介",
        douban_date: "2026-07-10",
      },
      observation: {
        created_at: 100,
        first_seen_at: 110,
        last_seen_at: 290,
      },
    });

    expect(rows).toEqual(
      expect.arrayContaining([
        { label: "上映日期", value: "2026-07-11" },
        { label: "评分", value: "8.8（1,200 人）" },
        { label: "原名", value: "Nested Title" },
        { label: "简介", value: "嵌套简介" },
        { label: "首次看到", value: expect.any(String) },
        { label: "最近看到", value: expect.any(String) },
      ]),
    );
  });

  it("formats the latest Poll outcome vocabulary", () => {
    expect(
      subscriptionPollToast({
        inserted: 1,
        updated: 2,
        unchanged: 3,
        reactivated: 4,
        deactivated: 5,
        fetched_items: 15,
        snapshot_complete: true,
      }),
    ).toBe("订阅刷新完成：抓取 15 · 新增 1 · 更新 2 · 未变 3 · 恢复 4 · 停用 5 · 完整快照");
  });

  it("exposes a fixed lifecycle graph and attention on the current node", () => {
    expect(
      subscriptionLifecycleNodes({
        lifecycle_state: "downloading",
        attention_tags: ["waiting_release"],
      }).map(({ key, label, state, attention }) => ({ key, label, state, attention })),
    ).toEqual([
      { key: "queued", label: "入队", state: "done", attention: "" },
      { key: "meta", label: "元数据", state: "done", attention: "" },
      { key: "searching", label: "搜索", state: "done", attention: "" },
      { key: "downloading", label: "下载", state: "current", attention: "waiting_release" },
      { key: "linking", label: "硬链接中", state: "todo", attention: "" },
      { key: "completed", label: "完成", state: "todo", attention: "" },
    ]);

    expect(
      subscriptionLifecycleNodes({
        lifecycle_state: "linking",
        attention_tags: ["failed"],
      }).find((node) => node.key === "linking"),
    ).toEqual({ key: "linking", label: "硬链接中", state: "current", attention: "failed" });
    expect(
      subscriptionDisplayStatus({
        lifecycle_state: "linking",
        attention_tags: ["failed"],
      }),
    ).toEqual({ key: "failed", text: "失败" });
  });
});
