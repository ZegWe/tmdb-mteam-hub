import { defaultApiClient } from "../../shared/api/client.js";
import { testQbServer as requestTestQbServer } from "../../shared/api/endpoints/qb.js";
import {
  getSettings as requestSettings,
  updateSettings as requestSettingsUpdate,
} from "../../shared/api/endpoints/settings.js";

export function getSettings(options) {
  return requestSettings(options);
}

export function updateSettings(payload, options) {
  return requestSettingsUpdate(payload, options);
}

export function testQbServer(payload, options) {
  return requestTestQbServer(payload, options);
}

/**
 * @param {import("../../shared/api/client.js").ApiRequestOptions & {client?: import("../../shared/api/client.js").ApiClient}} [options]
 * @returns {Promise<import("../../shared/api/contracts.js").DoubanQrStartResponseDto>}
 */
export function startDoubanQrSession({ client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/douban/qr/start", {
    ...requestOptions,
    method: "POST",
    body: {},
  });
}

/**
 * @param {unknown} sessionId
 * @param {import("../../shared/api/client.js").ApiRequestOptions & {client?: import("../../shared/api/client.js").ApiClient}} [options]
 * @returns {Promise<import("../../shared/api/contracts.js").DoubanQrPollResponseDto>}
 */
export function pollDoubanQrSession(
  sessionId,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const query = new URLSearchParams({ session_id: String(sessionId ?? "") });
  return client.request(`/api/douban/qr/poll?${query}`, requestOptions);
}
