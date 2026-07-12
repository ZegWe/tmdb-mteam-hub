import { describe, expect, it } from "vitest";
import { qbPushPayload } from "./domain.js";

describe("qB push domain", () => {
  it("normalizes the ID-only push contract and excludes incidental server data", () => {
    const payload = qbPushPayload({
      serverId: " nas ",
      torrentId: " 42 ",
      category: " movie ",
      savepath: " /downloads/movies ",
      server: { password: "SECRET_MUST_NOT_BE_SENT" },
    });

    expect(payload).toEqual({
      server_id: "nas",
      torrent_id: "42",
      category: "movie",
      savepath: "/downloads/movies",
    });
    expect(payload).not.toHaveProperty("server");
    expect(JSON.stringify(payload)).not.toContain("SECRET_MUST_NOT_BE_SENT");
  });

  it("omits blank optional fields", () => {
    expect(
      qbPushPayload({
        serverId: "nas",
        torrentId: "42",
        category: "  ",
        savepath: null,
      }),
    ).toEqual({ server_id: "nas", torrent_id: "42" });
  });
});
