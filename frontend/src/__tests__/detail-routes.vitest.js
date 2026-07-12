import { describe, expect, it } from "vitest";
import {
  detailBackRouteLocation,
  detailRouteLocationFromMediaCard,
  detailRouteLocationFromSubscriptionRecord,
  normalizeDetailRoute,
} from "../app/detail-routes.js";

describe("detail route domain", () => {
  it("normalizes supported media and subscription routes", () => {
    expect(
      normalizeDetailRoute({
        name: "media-detail",
        params: { mediaType: "movie", id: "123" },
      }),
    ).toEqual({ kind: "media", mediaType: "movie", id: "123" });
    expect(
      normalizeDetailRoute({
        name: "media-detail",
        params: { mediaType: ["tv"], id: [456] },
      }),
    ).toEqual({ kind: "media", mediaType: "tv", id: "456" });
    expect(
      normalizeDetailRoute({ name: "subscription-detail", params: { id: "douban-7" } }),
    ).toEqual({ kind: "subscription", id: "douban-7" });
  });

  it("validates subscription route IDs without trimming or coercing the original value", () => {
    for (const id of [
      " douban-7",
      "douban-7 ",
      "douban/7",
      "douban\\7",
      ".",
      "..",
      "\ud800",
      "\udc00",
      "a".repeat(257),
      88,
    ]) {
      expect(normalizeDetailRoute({ name: "subscription-detail", params: { id } })).toBeNull();
    }
  });

  it("rejects incomplete or unsupported route state", () => {
    expect(normalizeDetailRoute({ name: "media-detail", params: { mediaType: "movie" } })).toBe(
      null,
    );
    expect(
      normalizeDetailRoute({
        name: "media-detail",
        params: { mediaType: "bad", id: "123" },
      }),
    ).toBe(null);
    expect(normalizeDetailRoute({ name: "main", params: {} })).toBe(null);
  });

  it("builds stable locations from cards and subscriptions", () => {
    expect(
      detailRouteLocationFromMediaCard(
        { id: "subject-9", subject_id: "fallback", source: "douban", tags: "tag" },
        "movie",
      ),
    ).toEqual({
      name: "media-detail",
      params: { mediaType: "douban", id: "subject-9" },
      query: { doubanTags: "tag" },
    });
    expect(detailRouteLocationFromMediaCard({ id: 42, media_type: "tv" }, "movie")).toEqual({
      name: "media-detail",
      params: { mediaType: "tv", id: "42" },
      query: {},
    });
    expect(detailRouteLocationFromSubscriptionRecord({ subject_id: "88" })).toEqual({
      name: "subscription-detail",
      params: { id: "88" },
      query: {},
    });
    expect(detailRouteLocationFromSubscriptionRecord({ subject_id: " 88" })).toBeNull();
    expect(detailRouteLocationFromSubscriptionRecord({ subject_id: "." })).toBeNull();
    expect(detailRouteLocationFromSubscriptionRecord({ subject_id: "\ud800" })).toBeNull();
    expect(detailRouteLocationFromSubscriptionRecord({ subject_id: 88 })).toBeNull();
  });

  it("returns to the owning list route", () => {
    expect(detailBackRouteLocation({ kind: "media", mediaType: "movie", id: "1" })).toEqual({
      name: "main",
    });
    expect(detailBackRouteLocation({ kind: "subscription", id: "1" })).toEqual({
      name: "subscriptions",
    });
  });
});
