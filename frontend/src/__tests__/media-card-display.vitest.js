import { describe, expect, it } from "vitest";
import { cardKey, cardSubtitle } from "../features/search/domain.js";

describe("media card display", () => {
  it("keeps Douban subtitles compact", () => {
    expect(
      cardSubtitle({
        source: "douban",
        abstract_text: "美国 / 剧情 犯罪 / 弗兰克·德拉邦特",
        abstract_2: "1994",
        rating: { value: 9.7 },
      }),
    ).toBe("1994 · ★ 9.7");
  });

  it("keeps the TMDB date and rating", () => {
    expect(
      cardSubtitle({ media_type: "movie", release_date: "1994-09-10", vote_average: 9.3 }),
    ).toBe("1994-09-10 · ★ 9.3");
  });

  it("builds stable keys from the owning source and best available identity", () => {
    expect(cardKey({ source: "douban", subject_id: "1292052" }, "movie")).toBe("douban-1292052");
    expect(cardKey({ media_type: "tv", id: 42 }, "movie")).toBe("tv-42");
    expect(cardKey({ title: "无 ID 条目" }, "movie")).toBe("movie-无 ID 条目");
  });
});
