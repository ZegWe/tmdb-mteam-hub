import { defaultApiClient } from "../client.js";

/**
 * @typedef {{category?: string, status?: string, q?: string, page?: number, page_size?: number}} OperationLogFilters
 * @typedef {import("../client.js").ApiRequestOptions & {client?: import("../client.js").ApiClient}} OperationLogRequestOptions
 */

/**
 * @param {OperationLogFilters} [filters]
 * @param {OperationLogRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").OperationLogPageDto>}
 */
export function getOperationLogs(
  filters = {},
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value != null && String(value).trim() !== "") params.set(key, String(value));
  }
  const query = params.toString();
  return client.request(`/api/operation-logs${query ? `?${query}` : ""}`, requestOptions);
}
