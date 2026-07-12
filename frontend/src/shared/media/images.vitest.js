import { describe, expect, it } from "vitest";
import { itemImageUrl, posterUrl } from "./images.js";

describe("shared media images", () => {
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
