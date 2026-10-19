import { describe, expect, it } from "vitest";
import { itemImageUrl, posterUrl, searchImageUrl } from "./images.js";

describe("shared media images", () => {
  it("loads search posters directly while subscription posters keep their server URL", () => {
    const original = "https://qnmob3-sign.doubanio.com/p.jpg?sa_cv=abc&sa_ct=123&imageView2/0/q/80";
    const legacy = `/api/douban/image?url=${encodeURIComponent(original)}`;
    expect(searchImageUrl({ poster_url: legacy })).toBe(original);
    expect(searchImageUrl({ poster_url: original })).toBe(original);
    const local = "/api/subscriptions/posters/123";
    expect(itemImageUrl({ poster_url: local })).toBe(local);
    expect(searchImageUrl({ poster_url: "/api/douban/image?url=invalid" })).toBe("");
  });
  it("builds TMDB poster URLs without changing empty values", () => {
    expect(posterUrl("/poster.jpg")).toBe("https://image.tmdb.org/t/p/w342/poster.jpg");
    expect(posterUrl("")).toBe("");
  });

  it("prefers explicit poster and cover URLs before a TMDB poster path", () => {
    expect(
      itemImageUrl({
        poster_url: "https://example.test/poster.jpg",
        cover_url: "https://example.test/cover.jpg",
        poster_path: "/tmdb.jpg",
      }),
    ).toBe("https://example.test/poster.jpg");
    expect(
      itemImageUrl({ cover_url: "https://example.test/cover.jpg", poster_path: "/tmdb.jpg" }),
    ).toBe("https://example.test/cover.jpg");
    expect(itemImageUrl({ poster_path: "/tmdb.jpg" })).toBe(
      "https://image.tmdb.org/t/p/w342/tmdb.jpg",
    );
    expect(itemImageUrl(null)).toBe("");
  });
});
