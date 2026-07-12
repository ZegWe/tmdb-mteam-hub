import { defaultApiClient } from "../client.js";

/** @typedef {import("../contracts.js").ConfigResponseDto} SettingsSnapshotDto */
/** @typedef {import("../contracts.js").ConfigUpdateDto} SettingsUpdateDto */
/** @typedef {import("../contracts.js").RedactedQbServerDto} RedactedQbServerDto */
/** @typedef {import("../contracts.js").SubscriptionCategoryDto} SubscriptionCategoryDto */
/** @typedef {import("../contracts.js").SubscriptionWatcherDto} SubscriptionWatcherDto */
/** @typedef {import("../contracts.js").TorrentMatchRuleDto} TorrentMatchRuleDto */
/** @typedef {import("../contracts.js").QbServerPatchDto} QbServerPatchDto */

/**
 * @typedef {Omit<import("../client.js").ApiRequestOptions, "body" | "method"> & {client?: import("../client.js").ApiClient}} SettingsRequestOptions
 */

/**
 * @param {SettingsRequestOptions} [options]
 * @returns {Promise<SettingsSnapshotDto>}
 */
export function getSettings({ client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/config", requestOptions);
}

/**
 * @param {SettingsUpdateDto} payload
 * @param {SettingsRequestOptions} [options]
 * @returns {Promise<SettingsSnapshotDto>}
 */
export function updateSettings(payload, { client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/config", {
    ...requestOptions,
    method: "PUT",
    body: payload,
  });
}
