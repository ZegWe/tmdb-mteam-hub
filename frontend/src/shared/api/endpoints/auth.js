import { defaultApiClient } from "../client.js";

/** @typedef {import("../contracts.js").AuthStatusDto} AuthStatusDto */

/**
 * @typedef {Omit<import("../client.js").ApiRequestOptions, "body" | "method" | "credentials"> & {client?: import("../client.js").ApiClient}} AuthRequestOptions
 */

/**
 * @param {AuthRequestOptions} [options]
 * @returns {Promise<AuthStatusDto>}
 */
export function getAuthStatus({ client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/auth/status", {
    ...requestOptions,
    credentials: "same-origin",
  });
}

/**
 * @param {string} token
 * @param {AuthRequestOptions} [options]
 * @returns {Promise<AuthStatusDto>}
 */
export function loginAuthSession(token, { client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/auth/login", {
    ...requestOptions,
    method: "POST",
    body: { token },
    credentials: "same-origin",
  });
}

/**
 * @param {AuthRequestOptions} [options]
 * @returns {Promise<AuthStatusDto>}
 */
export function logoutAuthSession({ client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/auth/logout", {
    ...requestOptions,
    method: "POST",
    credentials: "same-origin",
  });
}
