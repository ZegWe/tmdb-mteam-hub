import { describe, expect, it } from "vitest";
import { mteamTorrentWebUrl, torrentStats } from "./torrents.js";

describe("torrent presentation", () => {
  it("distinguishes unknown counts from zero and preserves legacy size text", () => {
    expect(torrentStats({})).toBe("大小 — · 做种 — · 下载中 —");
    expect(torrentStats({ size: "1.25 GB", seeders: 0, leechers: 0 })).toBe(
      "大小 1.25 GB · 做种 0 · 下载中 0",
    );
    expect(torrentStats({ size: "" })).not.toContain("0 B");
  });
  it("uses only the original-site detail URL with an encoded ID", () => {
    expect(mteamTorrentWebUrl("123")).toBe("https://kp.m-team.cc/detail/123");
    expect(mteamTorrentWebUrl("123/?x=1")).toBe("https://kp.m-team.cc/detail/123%2F%3Fx%3D1");
  });
});
