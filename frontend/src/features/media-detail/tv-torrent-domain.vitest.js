import { describe, expect, it } from "vitest";
import { classifyTvTorrentTitle } from "./domain.js";

describe("TV torrent title coverage", () => {
  it.each([
    ["Show.S02E03.1080p", "episode", 3, 3, "S02E03 单集"],
    ["Show.S02E03-E06.2160p", "partial_pack", 3, 6, "S02E03-E06 部分合集"],
    ["Show E07-E09 WEB-DL", "partial_pack", 7, 9, "S02E07-E09 部分合集"],
    ["剧名 第3-6集", "partial_pack", 3, 6, "S02E03-E06 部分合集"],
    ["剧名 第3集-第6集", "partial_pack", 3, 6, "S02E03-E06 部分合集"],
    ["Show EP03-EP06", "partial_pack", 3, 6, "S02E03-E06 部分合集"],
    ["Show [03-06]", "partial_pack", 3, 6, "S02E03-E06 部分合集"],
    ["Show.S02.Complete.BluRay", "season_pack", 1, 10, "S02 整季合集"],
  ])("recognizes %s", (title, kind, start, end, label) => {
    expect(classifyTvTorrentTitle(title, 2, 10)).toMatchObject({
      kind,
      episodeStart: start,
      episodeEnd: end,
      compatible: true,
      label,
    });
  });

  it("rejects another season, out-of-range episodes, and ambiguous titles", () => {
    expect(classifyTvTorrentTitle("Show.S01E03", 2, 10).compatible).toBe(false);
    expect(classifyTvTorrentTitle("Show.S02E11", 2, 10).compatible).toBe(false);
    expect(classifyTvTorrentTitle("Show.2026.1080p", 2, 10)).toMatchObject({
      kind: "unknown",
      compatible: false,
    });
  });
});
