import { notifyAuthenticationRequired } from "./auth-session.js";

const DEFAULT_TIMEOUT_MS = 15_000;

/**
 * @typedef {object} ApiErrorOptions
 * @property {string=} code
 * @property {number=} status
 * @property {string=} path
 * @property {unknown=} details
 * @property {unknown=} cause
 */

/**
 * @typedef {Omit<RequestInit, "body"> & {body?: unknown, timeoutMs?: number}} ApiRequestOptions
 */

/**
 * @typedef {object} ApiClient
 * @property {<T = unknown>(path: string, options?: ApiRequestOptions) => Promise<T>} request
 */

export class ApiError extends Error {
  /** @param {string} message @param {ApiErrorOptions} [options] */
  constructor(message, { code = "api_error", status = 0, path = "", details = null, cause } = {}) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "ApiError";
    this.code = code;
    this.status = status;
    this.path = path;
    this.details = details;
  }
}

/** @param {string} baseUrl @param {string} path */
function resolveUrl(baseUrl, path) {
  const target = String(path || "");
  if (!baseUrl || /^[a-z][a-z\d+.-]*:/i.test(target)) return target;
  return `${String(baseUrl).replace(/\/$/, "")}/${target.replace(/^\//, "")}`;
}

/** @param {unknown} body @returns {body is BodyInit} */
function isNativeBody(body) {
  return (
    typeof body === "string" ||
    (typeof FormData !== "undefined" && body instanceof FormData) ||
    (typeof Blob !== "undefined" && body instanceof Blob) ||
    (typeof URLSearchParams !== "undefined" && body instanceof URLSearchParams) ||
    (typeof ArrayBuffer !== "undefined" && body instanceof ArrayBuffer) ||
    (typeof ArrayBuffer !== "undefined" && ArrayBuffer.isView(body))
  );
}

/** @param {unknown} body @param {Headers} headers @returns {BodyInit | undefined} */
function prepareBody(body, headers) {
  if (body == null) return undefined;
  if (isNativeBody(body)) return body;
  if (!headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  return JSON.stringify(body);
}

/** @param {unknown} value @returns {value is Record<string, unknown>} */
function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** @param {unknown} payload @param {Response} response */
function serverErrorFields(payload, response) {
  const root = isRecord(payload) ? payload : {};
  const nested = isRecord(root.error) ? root.error : null;
  const message =
    (typeof nested?.message === "string" ? nested.message : "") ||
    (typeof root.error === "string" ? root.error : "") ||
    (typeof root.message === "string" ? root.message : "") ||
    response.statusText ||
    "请求失败";
  const code =
    (typeof nested?.code === "string" ? nested.code : "") ||
    (typeof root.code === "string" ? root.code : "") ||
    "http_error";
  const details = nested?.details ?? root.details ?? payload ?? null;
  return { message, code, details };
}

/** @param {string} path @param {unknown} cause */
function abortedError(path, cause) {
  return new ApiError("请求已取消", {
    code: "request_aborted",
    path,
    cause,
  });
}

/** @param {string} path @param {number} timeoutMs @param {unknown} cause */
function timeoutError(path, timeoutMs, cause) {
  return new ApiError(`请求超时（${timeoutMs}ms）`, {
    code: "request_timeout",
    path,
    cause,
  });
}

/**
 * @param {{baseUrl?: string, timeoutMs?: number, fetchImpl?: typeof fetch}} [options]
 * @returns {ApiClient}
 */
export function createApiClient({
  baseUrl = "",
  timeoutMs: defaultTimeoutMs = DEFAULT_TIMEOUT_MS,
  fetchImpl = (...args) => globalThis.fetch(...args),
} = {}) {
  if (typeof fetchImpl !== "function") throw new TypeError("fetchImpl must be a function");

  /**
   * @template T
   * @param {string} path
   * @param {ApiRequestOptions} [options]
   * @returns {Promise<T>}
   */
  async function request(path, options = {}) {
    const url = resolveUrl(baseUrl, path);
    const headers = new Headers(options.headers || {});
    if (!headers.has("Accept")) headers.set("Accept", "application/json");
    const body = prepareBody(options.body, headers);
    const method = String(options.method || "GET").toUpperCase();
    const timeoutMs = Number(options.timeoutMs ?? defaultTimeoutMs);
    const callerSignal = options.signal;
    const controller = new AbortController();
    let timedOut = false;
    let callerAborted = false;
    let timeoutId = 0;

    const abortFromCaller = () => {
      callerAborted = true;
      controller.abort(callerSignal?.reason);
    };

    if (callerSignal?.aborted) {
      abortFromCaller();
    } else if (callerSignal) {
      callerSignal.addEventListener("abort", abortFromCaller, { once: true });
    }

    if (!controller.signal.aborted && Number.isFinite(timeoutMs) && timeoutMs > 0) {
      timeoutId = setTimeout(() => {
        timedOut = true;
        controller.abort(new DOMException("Request timed out", "TimeoutError"));
      }, timeoutMs);
    }

    try {
      /** @type {Response} */
      let response;
      try {
        const { timeoutMs: _requestTimeout, ...fetchOptions } = options;
        response = await fetchImpl(url, {
          ...fetchOptions,
          method,
          headers,
          body,
          signal: controller.signal,
        });
      } catch (error) {
        if (timedOut) throw timeoutError(path, timeoutMs, controller.signal.reason || error);
        if (callerAborted || callerSignal?.aborted) {
          throw abortedError(path, callerSignal?.reason || error);
        }
        if (controller.signal.aborted) throw abortedError(path, controller.signal.reason || error);
        throw new ApiError(`请求未收到服务端响应：${path}`, {
          code: "network_error",
          path,
          cause: error,
        });
      }

      let text;
      try {
        text = await response.text();
      } catch (error) {
        throw new ApiError("读取服务端响应失败", {
          code: "response_read_error",
          status: response.status,
          path,
          cause: error,
        });
      }

      let payload = null;
      if (text.trim()) {
        try {
          payload = JSON.parse(text);
        } catch (error) {
          if (response.ok) {
            throw new ApiError("服务端返回了无效 JSON", {
              code: "invalid_json",
              status: response.status,
              path,
              cause: error,
            });
          }
        }
      }

      if (!response.ok) {
        if (response.status === 401) notifyAuthenticationRequired();
        const fields = serverErrorFields(payload, response);
        throw new ApiError(fields.message, {
          code: fields.code,
          status: response.status,
          path,
          details: fields.details,
        });
      }

      return /** @type {T} */ (payload);
    } finally {
      if (timeoutId) clearTimeout(timeoutId);
      callerSignal?.removeEventListener("abort", abortFromCaller);
    }
  }

  return Object.freeze({ request });
}

export const defaultApiClient = createApiClient();
