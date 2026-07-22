import { defaultApiClient } from "../client.js";
import {
  SUBSCRIPTION_ATTENTION_TAGS as ATTENTION_TAG_VALUES,
  SUBSCRIPTION_EXECUTION_STATES as EXECUTION_STATE_VALUES,
  SUBSCRIPTION_LIFECYCLE_STATES as LIFECYCLE_STATE_VALUES,
  SUBSCRIPTION_MEDIA_KINDS as MEDIA_KIND_VALUES,
  SUBSCRIPTION_SUMMARY_FIELDS,
} from "../contracts.js";

export { SUBSCRIPTION_SUMMARY_FIELDS } from "../contracts.js";

/**
 * @typedef {import("../client.js").ApiRequestOptions & {client?: import("../client.js").ApiClient}} SubscriptionRequestOptions
 * @typedef {{active?: boolean, media_kind?: import("../contracts.js").SubscriptionMediaKind, lifecycle_state?: import("../contracts.js").SubscriptionLifecycleState, attention_tag?: import("../contracts.js").SubscriptionAttentionTag}} SubscriptionListFilters
 * @typedef {(message: string) => TypeError} InvalidResponseFactory
 */

const SUBSCRIPTION_LIST_PAGE_SIZE = 100;
export const MAX_PAGES = 100;
export const MAX_RECORDS = 10_000;

const SUBSCRIPTION_LIST_FILTER_FIELDS = Object.freeze([
  "active",
  "media_kind",
  "lifecycle_state",
  "attention_tag",
]);
const SUBSCRIPTION_MEDIA_KINDS = new Set(MEDIA_KIND_VALUES);
const SUBSCRIPTION_LIFECYCLE_STATES = new Set(LIFECYCLE_STATE_VALUES);
const SUBSCRIPTION_EXECUTION_STATES = new Set(EXECUTION_STATE_VALUES);
const SUBSCRIPTION_ATTENTION_TAGS = new Set(ATTENTION_TAG_VALUES);
const SUBSCRIPTION_DETAIL_FIELDS = Object.freeze([
  "summary",
  "source",
  "observation",
  "issues",
  "skip_reason",
  "candidates",
  "tv",
  "downloads",
  "links",
]);

const subscriptionIdEncoder = new TextEncoder();

/** @param {unknown} value @returns {value is Record<string, unknown>} */
function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** @param {string} message */
function responseError(message) {
  return new TypeError(`Invalid subscription list response: ${message}`);
}

/** @param {string} message */
function detailResponseError(message) {
  return new TypeError(`Invalid subscription detail response: ${message}`);
}

/** @param {string} value */
function containsControlCharacter(value) {
  for (const character of value) {
    const codePoint = character.codePointAt(0) ?? 0;
    if (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)) return true;
  }
  return false;
}

/** @param {string} value */
function containsUnpairedSurrogate(value) {
  for (let index = 0; index < value.length; index += 1) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
      const nextCodeUnit = value.charCodeAt(index + 1);
      if (!(nextCodeUnit >= 0xdc00 && nextCodeUnit <= 0xdfff)) return true;
      index += 1;
    } else if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
      return true;
    }
  }
  return false;
}

/** @param {unknown} value @returns {value is string} */
export function isValidSubscriptionId(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value !== "." &&
    value !== ".." &&
    value.trim() === value &&
    !containsUnpairedSurrogate(value) &&
    subscriptionIdEncoder.encode(value).byteLength <= 256 &&
    !containsControlCharacter(value) &&
    !value.includes("/") &&
    !value.includes("\\")
  );
}

/** @param {unknown} value @param {InvalidResponseFactory} [invalidResponse] */
function validatedSubscriptionId(value, invalidResponse = responseError) {
  if (!isValidSubscriptionId(value)) throw invalidResponse("record ID is invalid");
  return value;
}

/** @param {unknown} value */
function requestedSubscriptionId(value) {
  if (!isValidSubscriptionId(value)) throw new TypeError("subscription id is invalid");
  return value;
}

/** @param {unknown} value */
function isValidRevision(value) {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

/** @param {unknown} value @param {number} [max] */
function isUnsignedSafeInteger(value, max = Number.MAX_SAFE_INTEGER) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= max;
}

/** @param {unknown} value @param {number} [max] */
function isNullableUnsignedSafeInteger(value, max = Number.MAX_SAFE_INTEGER) {
  return value === null || isUnsignedSafeInteger(value, max);
}

/** @param {unknown} value */
function isNullableString(value) {
  return value === null || typeof value === "string";
}

/**
 * @param {unknown} record
 * @param {InvalidResponseFactory} invalidResponse
 * @returns {import("../contracts.js").SubscriptionSummaryDto}
 */
function normalizeSummaryRecord(record, invalidResponse) {
  if (!isRecord(record)) throw invalidResponse("every summary item must be an object");
  if (!Object.hasOwn(record, "subject_id")) {
    throw invalidResponse("summary item ID is required");
  }
  const id = validatedSubscriptionId(record.subject_id, invalidResponse);
  if (!Object.hasOwn(record, "revision") || !isValidRevision(record.revision)) {
    throw invalidResponse("summary revision must be a positive safe integer");
  }
  for (const field of SUBSCRIPTION_SUMMARY_FIELDS) {
    if (!Object.hasOwn(record, field)) {
      throw invalidResponse(`summary field is required: ${field}`);
    }
  }
  if (typeof record.active !== "boolean" || typeof record.schedulable !== "boolean") {
    throw invalidResponse("summary active and schedulable fields must be booleans");
  }
  if (
    typeof record.retry_blocked !== "boolean" ||
    typeof record.force_eligible_once !== "boolean"
  ) {
    throw invalidResponse("summary retry and force flags must be booleans");
  }
  if (
    typeof record.media_kind !== "string" ||
    !SUBSCRIPTION_MEDIA_KINDS.has(
      /** @type {import("../contracts.js").SubscriptionMediaKind} */ (record.media_kind),
    )
  ) {
    throw invalidResponse("summary media_kind is invalid");
  }
  if (
    typeof record.lifecycle_state !== "string" ||
    !SUBSCRIPTION_LIFECYCLE_STATES.has(
      /** @type {import("../contracts.js").SubscriptionLifecycleState} */ (record.lifecycle_state),
    )
  ) {
    throw invalidResponse("summary lifecycle_state is invalid");
  }
  if (
    typeof record.execution_state !== "string" ||
    !SUBSCRIPTION_EXECUTION_STATES.has(
      /** @type {import("../contracts.js").SubscriptionExecutionState} */ (record.execution_state),
    )
  ) {
    throw invalidResponse("summary execution_state is invalid");
  }
  if (
    !isNullableUnsignedSafeInteger(record.inactive_at) ||
    !isNullableUnsignedSafeInteger(record.next_attempt_at) ||
    !isNullableUnsignedSafeInteger(record.douban_sort_time) ||
    !isUnsignedSafeInteger(record.updated_at)
  ) {
    throw invalidResponse("summary timestamps must be null or unsigned safe integers");
  }
  if (
    !isUnsignedSafeInteger(record.retry_count, 0xffff_ffff) ||
    !isUnsignedSafeInteger(record.max_retries, 0xffff_ffff)
  ) {
    throw invalidResponse("summary retry counts must be unsigned 32-bit integers");
  }
  if (!isNullableUnsignedSafeInteger(record.release_year, 0xffff)) {
    throw invalidResponse("summary release_year must be null or an unsigned 16-bit integer");
  }
  if (
    !isNullableString(record.last_seen_snapshot_id) ||
    !isNullableString(record.blocked_reason) ||
    !isNullableString(record.category_text) ||
    typeof record.title !== "string" ||
    typeof record.poster_url !== "string"
  ) {
    throw invalidResponse("summary text fields have invalid types");
  }
  if (
    !Array.isArray(record.attention_tags) ||
    record.attention_tags.some(
      (tag) =>
        typeof tag !== "string" ||
        !SUBSCRIPTION_ATTENTION_TAGS.has(
          /** @type {import("../contracts.js").SubscriptionAttentionTag} */ (tag),
        ),
    ) ||
    new Set(record.attention_tags).size !== record.attention_tags.length
  ) {
    throw invalidResponse("summary attention_tags are invalid");
  }

  /** @type {Record<string, unknown>} */
  const normalized = {};
  for (const field of SUBSCRIPTION_SUMMARY_FIELDS) {
    normalized[field] = record[field];
  }
  normalized.subject_id = id;
  return /** @type {import("../contracts.js").SubscriptionSummaryDto} */ (normalized);
}

/** @param {unknown} record */
export function normalizeSubscriptionSummaryRecord(record) {
  return normalizeSummaryRecord(record, responseError);
}

/**
 * @param {Record<string, unknown>} response
 * @returns {import("../contracts.js").NormalizedSubscriptionSummaryPage}
 */
function normalizeSummaryPage(response) {
  const keys = Object.keys(response);
  if (
    keys.length !== 2 ||
    !Object.hasOwn(response, "items") ||
    !Object.hasOwn(response, "next_cursor")
  ) {
    throw responseError("summary pages must contain exactly items and next_cursor");
  }
  if (!Array.isArray(response.items)) throw responseError("items must be an array");
  if (
    response.next_cursor !== null &&
    (typeof response.next_cursor !== "string" ||
      !response.next_cursor ||
      response.next_cursor.trim() !== response.next_cursor)
  ) {
    throw responseError("next_cursor must be null or a non-empty string");
  }

  /** @type {[string, import("../contracts.js").SubscriptionSummaryDto][]} */
  const entries = [];
  const ids = new Set();
  for (const record of response.items) {
    const normalized = normalizeSubscriptionSummaryRecord(record);
    const id = normalized.subject_id;
    if (ids.has(id)) throw responseError(`duplicate record ID: ${id}`);
    ids.add(id);
    entries.push([id, normalized]);
  }

  return {
    next_cursor: response.next_cursor,
    ordered_ids: entries.map(([id]) => id),
    records: Object.fromEntries(entries),
  };
}

/**
 * @param {unknown} response
 * @returns {import("../contracts.js").NormalizedSubscriptionSummaryPage}
 */
export function normalizeWantedSubscriptionsPage(response) {
  if (!isRecord(response)) throw responseError("expected an object");
  return normalizeSummaryPage(response);
}

/** @param {unknown} filters */
function normalizedListFilterParams(filters) {
  if (!isRecord(filters)) throw new TypeError("subscription list filters must be an object");
  for (const field of Object.keys(filters)) {
    if (!SUBSCRIPTION_LIST_FILTER_FIELDS.includes(field)) {
      throw new TypeError(`unsupported subscription list filter: ${field}`);
    }
  }

  const params = new URLSearchParams({ limit: String(SUBSCRIPTION_LIST_PAGE_SIZE) });
  if (filters.active != null) {
    if (typeof filters.active !== "boolean") {
      throw new TypeError("subscription active filter must be a boolean");
    }
    params.set("active", String(filters.active));
  }
  if (filters.media_kind != null) {
    if (
      typeof filters.media_kind !== "string" ||
      !SUBSCRIPTION_MEDIA_KINDS.has(
        /** @type {import("../contracts.js").SubscriptionMediaKind} */ (filters.media_kind),
      )
    ) {
      throw new TypeError("subscription media_kind filter is invalid");
    }
    params.set("media_kind", filters.media_kind);
  }
  if (filters.lifecycle_state != null) {
    if (
      typeof filters.lifecycle_state !== "string" ||
      !SUBSCRIPTION_LIFECYCLE_STATES.has(
        /** @type {import("../contracts.js").SubscriptionLifecycleState} */ (
          filters.lifecycle_state
        ),
      )
    ) {
      throw new TypeError("subscription lifecycle_state filter is invalid");
    }
    params.set("lifecycle_state", filters.lifecycle_state);
  }
  if (filters.attention_tag != null) {
    if (
      typeof filters.attention_tag !== "string" ||
      !SUBSCRIPTION_ATTENTION_TAGS.has(
        /** @type {import("../contracts.js").SubscriptionAttentionTag} */ (filters.attention_tag),
      )
    ) {
      throw new TypeError("subscription attention_tag filter is invalid");
    }
    params.set("attention_tag", filters.attention_tag);
  }
  return params;
}

/** @param {AbortSignal | null | undefined} signal */
function throwIfAborted(signal) {
  if (!signal?.aborted) return;
  if (typeof signal.throwIfAborted === "function") signal.throwIfAborted();
  const error = new Error("The request was aborted");
  error.name = "AbortError";
  throw error;
}

/** @param {URLSearchParams} baseParams @param {string | null} cursor */
function wantedSubscriptionsPath(baseParams, cursor) {
  const params = new URLSearchParams(baseParams);
  if (cursor !== null) params.set("cursor", cursor);
  return `/api/subscriptions/wanted?${params}`;
}

/**
 * @param {SubscriptionRequestOptions & {filters?: SubscriptionListFilters}} [options]
 * @returns {Promise<import("../contracts.js").NormalizedSubscriptionSummaryState>}
 */
export async function getWantedSubscriptions({
  client = defaultApiClient,
  filters = {},
  ...requestOptions
} = {}) {
  const baseParams = normalizedListFilterParams(filters);
  const signal = requestOptions.signal;
  /** @type {[string, import("../contracts.js").SubscriptionSummaryDto][]} */
  const entries = [];
  const orderedIds = [];
  const recordIds = new Set();
  const cursors = new Set();
  /** @type {string | null} */
  let cursor = null;

  for (let pageNumber = 1; pageNumber <= MAX_PAGES; pageNumber += 1) {
    throwIfAborted(signal);
    const response = await client.request(
      wantedSubscriptionsPath(baseParams, cursor),
      requestOptions,
    );
    throwIfAborted(signal);
    const page = normalizeWantedSubscriptionsPage(response);

    for (const id of page.ordered_ids) {
      if (recordIds.has(id)) throw responseError(`duplicate record ID across pages: ${id}`);
      recordIds.add(id);
      orderedIds.push(id);
      entries.push([id, page.records[id]]);
      if (entries.length > MAX_RECORDS) {
        throw responseError(`record limit exceeded: ${MAX_RECORDS}`);
      }
    }

    if (page.next_cursor === null) {
      return {
        next_cursor: null,
        ordered_ids: orderedIds,
        records: Object.fromEntries(entries),
      };
    }
    if (cursors.has(page.next_cursor)) {
      throw responseError("repeated next_cursor");
    }
    cursors.add(page.next_cursor);
    cursor = page.next_cursor;
    if (pageNumber === MAX_PAGES) {
      throw responseError(`page limit exceeded: ${MAX_PAGES}`);
    }
  }

  throw responseError(`page limit exceeded: ${MAX_PAGES}`);
}

/** @param {unknown} value @param {string} field @returns {asserts value is Record<string, unknown>} */
function assertDetailRecord(value, field) {
  if (!isRecord(value)) throw detailResponseError(`${field} must be an object`);
}

/** @param {unknown} value @param {string} field @returns {asserts value is Record<string, unknown>[]} */
function assertDetailRecordArray(value, field) {
  if (!Array.isArray(value) || value.some((item) => !isRecord(item))) {
    throw detailResponseError(`${field} must be an array of objects`);
  }
}

/**
 * @param {unknown} response
 * @param {unknown} expectedId
 * @returns {import("../contracts.js").SubscriptionDetailDto}
 */
export function normalizeWantedSubscriptionDetailResponse(response, expectedId) {
  const subjectId = requestedSubscriptionId(expectedId);
  if (!isRecord(response)) throw detailResponseError("expected an object");
  const keys = Object.keys(response);
  if (
    keys.length !== SUBSCRIPTION_DETAIL_FIELDS.length ||
    SUBSCRIPTION_DETAIL_FIELDS.some((field) => !Object.hasOwn(response, field))
  ) {
    throw detailResponseError("unexpected top-level DTO shape");
  }

  const summary = normalizeSummaryRecord(response.summary, detailResponseError);
  if (summary.subject_id !== subjectId) {
    throw detailResponseError("summary ID does not match the requested path ID");
  }
  assertDetailRecord(response.source, "source");
  assertDetailRecord(response.observation, "observation");
  assertDetailRecordArray(response.issues, "issues");
  assertDetailRecordArray(response.candidates, "candidates");
  assertDetailRecordArray(response.downloads, "downloads");
  assertDetailRecordArray(response.links, "links");
  if (response.skip_reason !== null && typeof response.skip_reason !== "string") {
    throw detailResponseError("skip_reason must be null or a string");
  }
  if (response.tv !== null) assertDetailRecord(response.tv, "tv");

  return {
    summary,
    source: /** @type {import("../contracts.js").SubscriptionSourceDto} */ (response.source),
    observation: /** @type {import("../contracts.js").SubscriptionObservationDto} */ (
      response.observation
    ),
    issues: /** @type {import("../contracts.js").SubscriptionIssueDto[]} */ (response.issues),
    skip_reason: response.skip_reason,
    candidates: /** @type {import("../contracts.js").SubscriptionCandidateDto[]} */ (
      response.candidates
    ),
    tv: /** @type {import("../contracts.js").SubscriptionTvDetailDto | null} */ (response.tv),
    downloads: /** @type {import("../contracts.js").SubscriptionDownloadArtifactDto[]} */ (
      response.downloads
    ),
    links: /** @type {import("../contracts.js").SubscriptionLinkArtifactDto[]} */ (response.links),
  };
}

/**
 * @param {unknown} id
 * @param {SubscriptionRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").SubscriptionDetailDto>}
 */
export async function getWantedSubscriptionDetail(
  id,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const subjectId = requestedSubscriptionId(id);
  const signal = requestOptions.signal;
  throwIfAborted(signal);
  const response = await client.request(
    `/api/subscriptions/wanted/${encodeURIComponent(subjectId)}`,
    requestOptions,
  );
  throwIfAborted(signal);
  return normalizeWantedSubscriptionDetailResponse(response, subjectId);
}

/** @param {SubscriptionRequestOptions} [options] */
export function pollWantedSubscriptions({ client = defaultApiClient, ...requestOptions } = {}) {
  return client.request("/api/subscriptions/wanted/poll", {
    ...requestOptions,
    method: "POST",
    body: {},
  });
}

/**
 * @param {unknown} id
 * @param {SubscriptionRequestOptions} [options]
 * @returns {Promise<import("../contracts.js").SubscriptionSummaryDto>}
 */
export async function retrySubscription(
  id,
  { client = defaultApiClient, ...requestOptions } = {},
) {
  const subjectId = requestedSubscriptionId(id);
  const signal = requestOptions.signal;
  throwIfAborted(signal);
  const response = await client.request(
    `/api/subscriptions/wanted/${encodeURIComponent(subjectId)}/retry`,
    { ...requestOptions, method: "POST", body: {} },
  );
  throwIfAborted(signal);
  return normalizeSubscriptionSummaryRecord(response);
}
