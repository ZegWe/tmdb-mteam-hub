import { defaultApiClient } from "../client.js";

/**
 * @typedef {import("../client.js").ApiRequestOptions & {client?: import("../client.js").ApiClient}} SearchRequestOptions
 */

/**
 * @param {unknown} query
 * @param {SearchRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").TmdbSearchResponseDto>}
 */
export function searchTmdb(query, { client = defaultApiClient, ...requestOptions } = {}) {
  const params = new URLSearchParams({ q: String(query || "").trim() });
  return client.request(`/api/search?${params}`, requestOptions);
}

/**
 * @param {unknown} query
 * @param {SearchRequestOptions & {page?: number, pageSize?: number}} [options]
 * @returns {Promise<import("../contracts.js").DoubanSearchResponseDto>}
 */
export function searchDouban(
  query,
  { page = 1, pageSize = 20, client = defaultApiClient, ...requestOptions } = {},
) {
  const params = new URLSearchParams({
    q: String(query || "").trim(),
    page: String(Math.max(1, Number(page) || 1)),
    page_size: String(Math.max(1, Number(pageSize) || 20)),
  });
  return client.request(`/api/douban/search?${params}`, requestOptions);
}
