import { isValidSubscriptionId } from "../shared/api/endpoints/subscriptions.js";

const DETAIL_MEDIA_TYPES = ["movie", "tv", "douban"];

export function firstQueryValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

export function normalizeDetailRoute(routeLike) {
  const name = String(routeLike?.name || "");
  const params = routeLike?.params || {};
  if (name === "media-detail") {
    const mediaType = String(firstQueryValue(params.mediaType) || "").trim();
    const id = String(firstQueryValue(params.id) || "").trim();
    if (DETAIL_MEDIA_TYPES.includes(mediaType) && id) {
      return { kind: "media", mediaType, id };
    }
    return null;
  }
  if (name === "subscription-detail") {
    const id = firstQueryValue(params.id);
    return isValidSubscriptionId(id) ? { kind: "subscription", id } : null;
  }
  return null;
}

export function detailRouteLocationFromMediaCard(item, fallbackType) {
  const type = item?.source === "douban" ? "douban" : item?.media_type || fallbackType;
  const mediaType = DETAIL_MEDIA_TYPES.includes(type) ? type : fallbackType;
  const rawId = type === "douban" ? (item?.id ?? item?.subject_id) : item?.id;
  const id = String(rawId || "").trim();
  if (!id) return null;
  const query = {};
  const tags = Array.isArray(item?.tags) ? item.tags.join(" ") : item?.tags || "";
  if (mediaType === "douban" && String(tags).trim()) query.doubanTags = String(tags).trim();
  return { name: "media-detail", params: { mediaType, id }, query };
}

export function detailRouteLocationFromSubscriptionRecord(record) {
  const id = record?.subject_id;
  return isValidSubscriptionId(id)
    ? { name: "subscription-detail", params: { id }, query: {} }
    : null;
}

export function detailBackRouteLocation(parsed) {
  return parsed?.kind === "subscription" ? { name: "subscriptions" } : { name: "main" };
}
