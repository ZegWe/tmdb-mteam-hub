import { describe, expect, it } from "vitest";
import {
  createOperationLogFilters,
  operationLogActionLabel,
  operationLogCategoryLabel,
  operationLogFilterKey,
  operationLogMatchedKeywords,
  operationLogMatchStats,
  operationLogRelated,
  operationLogRuleEvaluationSummary,
  operationLogStatusLabel,
  operationLogSummary,
  operationLogTarget,
  operationLogTorrentMatches,
} from "./domain.js";

describe("operation log domain", () => {
  it("normalizes shareable filters and builds a stable summary", () => {
    const filters = createOperationLogFilters({
      category: " SEARCH ",
      status: " FAILED ",
      q: "  电影  ",
    });

    expect(filters).toEqual({ category: "search", status: "failed", q: "电影" });
    expect(operationLogFilterKey(filters)).toBe('["search","failed","电影"]');
    expect(operationLogSummary({ total: 9 }, [{}, {}], filters)).toBe(
      "共 9 条 · 已显示 2 条 · 分类 搜索订阅 · 状态 失败 · 关键词 电影",
    );
  });

  it("keeps labels and target/related formatting compatible with the page", () => {
    expect(operationLogCategoryLabel("qb_push")).toBe("推送 qB");
    expect(operationLogStatusLabel("SUCCESS")).toBe("成功");
    expect(operationLogActionLabel("push_torrent")).toBe("订阅推送 qB");
    expect(
      operationLogTarget({ target_title: "电影", target_type: "subscription", target_id: "7" }),
    ).toBe("电影 · 对象 subscription · ID 7");
    expect(operationLogTarget({})).toBe("无关联对象");
    expect(
      operationLogRelated({
        related: {
          fields: [
            { key: "candidate_count", value: "3" },
            { key: "qb_server", value: "NAS" },
            { key: "empty", value: "" },
          ],
          torrent_matches: [],
        },
      }),
    ).toEqual(["候选 3", "qB NAS"]);
  });

  it("formats torrent matching diagnostics without exposing page logic", () => {
    const match = {
      torrent_id: "torrent-1",
      seeders: 8,
      leechers: 2,
      size: "4 GB",
      matched_keywords: ["1080p", "中字"],
      rule_evaluations: [
        {
          rule_name: "高清",
          matched: false,
          matched_keywords: ["1080p"],
          missing_keywords: ["HDR"],
          excluded_reason: "缺少 HDR",
        },
      ],
    };
    const entry = { related: { fields: [], torrent_matches: [match] } };

    expect(operationLogTorrentMatches(entry)).toEqual([match]);
    expect(operationLogMatchStats(match)).toBe("ID torrent-1 · 做种 8 · 下载 2 · 大小 4 GB");
    expect(operationLogMatchedKeywords(match)).toBe("1080p、中字");
    expect(operationLogRuleEvaluationSummary(match)).toBe(
      "高清 未命中，命中 1080p，缺少 HDR，缺少 HDR",
    );
  });
});
