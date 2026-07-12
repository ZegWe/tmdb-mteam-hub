import { defaultApiClient } from "../client.js";

/** @typedef {"movie" | "tv" | "douban"} MediaDetailType */

/**
 * @typedef {Omit<import("../client.js").ApiRequestOptions, "body" | "method"> & {client?: import("../client.js").ApiClient}} MediaDetailRequestOptions
 */

/**
 * @typedef {import("../contracts.js").DoubanInterestRequestDto} DoubanInterestUpdateDto
 */

/**
 * @param {MediaDetailType} mediaType
 * @param {string | number} id
 * @param {MediaDetailRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").TmdbMediaDetailDto | import("../contracts.js").DoubanSubjectDetailDto>}
 */
export function getMediaDetail(
  mediaType,
  id,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const normalizedType = String(mediaType || "");
  const normalizedId = encodeURIComponent(String(id || ""));
  if (normalizedType === "douban") {
    return client.request(`/api/douban/subject/${normalizedId}`, requestOptions);
  }
  if (normalizedType === "movie" || normalizedType === "tv") {
    return client.request(`/api/tmdb/${normalizedType}/${normalizedId}`, requestOptions);
  }
  throw new TypeError(`Unsupported media type: ${normalizedType}`);
}

/**
 * @param {string | number} tvId
 * @param {string | number} seasonNumber
 * @param {MediaDetailRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").TmdbSeasonDetailDto>}
 */
export function getTvSeasonEpisodes(
  tvId,
  seasonNumber,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  return client.request(
    `/api/tmdb/tv/${encodeURIComponent(String(tvId || ""))}/season/${encodeURIComponent(String(seasonNumber))}`,
    requestOptions,
  );
}

/**
 * @param {string | number} subjectId
 * @param {DoubanInterestUpdateDto} payload
 * @param {MediaDetailRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").DoubanInterestResponseDto>}
 */
export function saveDoubanInterest(
  subjectId,
  payload,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  return client.request(
    `/api/douban/subject/${encodeURIComponent(String(subjectId || ""))}/interest`,
    {
      ...requestOptions,
      method: "POST",
      body: payload,
    },
  );
}

/**
 * @param {{limit?: number} & MediaDetailRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").DoubanTagHistoryResponseDto>}
 */
export function getDoubanTagHistory({
  limit = 80,
  client = defaultApiClient,
  ...requestOptions
} = {}) {
  const params = new URLSearchParams({ limit: String(Math.max(1, Number(limit) || 80)) });
  return client.request(`/api/douban/tags?${params}`, requestOptions);
}

/**
 * @param {Record<string, string>} searchParams
 * @param {MediaDetailRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").MteamSearchResponseDto>}
 */
export function searchMteamTorrents(
  searchParams,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const params = new URLSearchParams(searchParams || {});
  return client.request(`/api/mteam/torrents?${params}`, requestOptions);
}
