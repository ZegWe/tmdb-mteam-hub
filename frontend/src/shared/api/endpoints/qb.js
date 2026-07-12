import { defaultApiClient } from "../client.js";

/** @typedef {import("../contracts.js").QbTestRequestDto} QbTestRequestDto */
/** @typedef {import("../contracts.js").QbTestResponseDto} QbTestResponseDto */
/** @typedef {import("../contracts.js").QbPushMteamRequestDto} QbPushMteamRequestDto */
/** @typedef {import("../contracts.js").QbPushMteamResponseDto} QbPushMteamResponseDto */

/**
 * @typedef {Omit<import("../client.js").ApiRequestOptions, "body" | "method"> & {client?: import("../client.js").ApiClient}} QbRequestOptions
 */

/**
 * @param {QbTestRequestDto} payload
 * @param {QbRequestOptions} [options]
 * @returns {Promise<QbTestResponseDto>}
 */
export function testQbServer(payload, { client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/qb/test", {
    ...requestOptions,
    method: "POST",
    body: payload,
  });
}

/**
 * @param {QbPushMteamRequestDto} payload
 * @param {QbRequestOptions} [options]
 * @returns {Promise<QbPushMteamResponseDto>}
 */
export function pushMteamTorrent(payload, { client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/qb/push-mteam", {
    ...requestOptions,
    method: "POST",
    body: payload,
  });
}
