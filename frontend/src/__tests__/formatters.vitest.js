import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatPercent,
  formatSize,
  formatUnixSeconds,
  joinDetailList,
  joinKeywordList,
  joinNames,
  mergeDoubanTagText,
  normalizeDoubanTags,
  normalizedStatus,
  splitKeywordList,
} from "../shared/lib/formatters.js";

describe("shared formatters", () => {
  it("normalizes display lists and statuses", () => {
    expect(joinNames([{ name: "电影" }, "剧集", null])).toBe("电影 · 剧集");
    expect(joinDetailList(["剧情", "犯罪", ""])).toBe("剧情 · 犯罪");
    expect(normalizedStatus(" Downloading ")).toBe("downloading");
  });

  it("normalizes Douban tags and keyword lists", () => {
    expect(normalizeDoubanTags("  电影   家庭  ")).toBe("电影 家庭");
    expect(mergeDoubanTagText("电影", "家庭")).toBe("电影 家庭");
    expect(mergeDoubanTagText("电影 家庭", "家庭")).toBe("电影 家庭");
    expect(splitKeywordList("2160p, REMUX，HDR\nAtmos")).toEqual([
      "2160p",
      "REMUX",
      "HDR",
      "Atmos",
    ]);
    expect(joinKeywordList(["2160p", "REMUX"])).toBe("2160p, REMUX");
  });

  it("formats percentages, sizes, and timestamps", () => {
    expect(formatPercent(0.426)).toBe("43%");
    expect(formatPercent(null)).toBe("0%");
    expect(formatSize(1024)).toBe("1.00 KB");
    expect(formatBytes(0)).toBe("");
    expect(formatBytes(1024)).toBe("1.00 KB");
    expect(formatUnixSeconds(0)).toBe("");
    expect(formatUnixSeconds(1)).not.toBe("");
  });
});
