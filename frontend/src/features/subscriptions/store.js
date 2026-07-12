import { computed, reactive, readonly, ref } from "vue";
import {
  getWantedSubscriptionDetail,
  getWantedSubscriptions,
  isValidSubscriptionId,
  normalizeWantedSubscriptionDetailResponse,
  normalizeSubscriptionSummaryRecord,
  pollWantedSubscriptions,
} from "../../shared/api/endpoints/subscriptions.js";
import { subscriptionSummary as buildSubscriptionSummary } from "./domain.js";

const DEFAULT_SUBSCRIPTION_POLL_INTERVAL_MS = 5000;

const defaultTransport = Object.freeze({
  load: (options) => getWantedSubscriptions(options),
  loadDetail: (id, options) => getWantedSubscriptionDetail(id, options),
  poll: (options) => pollWantedSubscriptions(options),
});

const SUBSCRIPTION_SUMMARY_PROJECTION = "summary";
const SUBSCRIPTION_DETAIL_PROJECTION = "detail";
const MAX_STALE_DETAIL_RETRIES = 1;

function finiteNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function serverRevision(value) {
  const revision = finiteNumber(value?.revision ?? value?.entity_revision ?? value?.row_revision);
  return Number.isSafeInteger(revision) && revision > 0 ? revision : null;
}

function updatedAt(value) {
  return finiteNumber(value?.updated_at);
}

function freshness(value, sequence) {
  return {
    revision: serverRevision(value),
    updatedAt: updatedAt(value),
    sequence,
  };
}

function compareFreshness(incoming, current) {
  if (!current) return 1;
  if (incoming.revision != null && current.revision != null) {
    return incoming.revision - current.revision;
  }
  if (incoming.updatedAt != null && current.updatedAt != null) {
    return incoming.updatedAt - current.updatedAt;
  }
  return incoming.sequence - current.sequence;
}

function compareProjectionFreshness(summaryFreshness, detailFreshness) {
  if (!summaryFreshness || !detailFreshness) return 1;
  if (summaryFreshness.revision != null || detailFreshness.revision != null) {
    if (summaryFreshness.revision == null) return -1;
    if (detailFreshness.revision == null) return 1;
    return summaryFreshness.revision - detailFreshness.revision;
  }
  if (summaryFreshness.updatedAt != null || detailFreshness.updatedAt != null) {
    if (summaryFreshness.updatedAt == null) return -1;
    if (detailFreshness.updatedAt == null) return 1;
    return summaryFreshness.updatedAt - detailFreshness.updatedAt;
  }
  return summaryFreshness.sequence - detailFreshness.sequence;
}

function recordsObject(value) {
  return value?.records && typeof value.records === "object" && !Array.isArray(value.records)
    ? value.records
    : {};
}

function recordFrom(records, id) {
  return Object.hasOwn(records, id) ? records[id] : null;
}

function requestedSubscriptionId(value) {
  return typeof value === "string" && isValidSubscriptionId(value) ? value : null;
}

function normalizedStoreRecordEntries(records) {
  const normalized = [];
  for (const [mapId, record] of Object.entries(records)) {
    const id = requestedSubscriptionId(mapId);
    if (!id || !record || typeof record !== "object" || Array.isArray(record)) {
      throw new TypeError("subscription transport returned an invalid record");
    }

    const summary = normalizeSubscriptionSummaryRecord(record);
    if (summary.subject_id !== id) {
      throw new TypeError("subscription summary ID does not match its cache key");
    }
    normalized.push([id, summary]);
  }
  return normalized;
}

function normalizedSummaryOrder(state, recordEntries) {
  if (!Object.hasOwn(state, "ordered_ids") || !Array.isArray(state.ordered_ids)) {
    throw new TypeError("subscription summary ordered_ids must be an array");
  }

  const recordIds = new Set(recordEntries.map(([id]) => id));
  const orderedIds = [];
  const seenIds = new Set();
  for (const rawId of state.ordered_ids) {
    const id = requestedSubscriptionId(rawId);
    if (!id) throw new TypeError("subscription summary ordered_ids contains an invalid ID");
    if (seenIds.has(id)) {
      throw new TypeError(`subscription summary ordered_ids contains a duplicate ID: ${id}`);
    }
    if (!recordIds.has(id)) {
      throw new TypeError(`subscription summary ordered_ids contains an unknown ID: ${id}`);
    }
    seenIds.add(id);
    orderedIds.push(id);
  }
  if (seenIds.size !== recordIds.size) {
    throw new TypeError("subscription summary ordered_ids must match records exactly");
  }
  return orderedIds;
}

function currentRecordEntries(state, records) {
  return Array.isArray(state?.ordered_ids) ? state.ordered_ids.map((id) => [id, records[id]]) : [];
}

function hiddenDocument(documentRef) {
  return !!documentRef && (documentRef.hidden === true || documentRef.visibilityState === "hidden");
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function isAbortError(error) {
  return (
    error?.name === "AbortError" ||
    error?.code === "request_aborted" ||
    error?.cause?.name === "AbortError"
  );
}

function nestedDetailEntity(detail, id) {
  const normalized = normalizeWantedSubscriptionDetailResponse(detail, id);
  return {
    ...normalized.summary,
    source: normalized.source,
    observation: normalized.observation,
    issues: normalized.issues,
    skip_reason: normalized.skip_reason,
    candidates: normalized.candidates,
    tv: normalized.tv,
    downloads: normalized.downloads,
    links: normalized.links,
  };
}

export function createSubscriptionStore({
  transport = defaultTransport,
  pollIntervalMs = DEFAULT_SUBSCRIPTION_POLL_INTERVAL_MS,
  documentRef = typeof document === "undefined" ? null : document,
  setTimeoutFn = (...args) => globalThis.setTimeout(...args),
  clearTimeoutFn = (...args) => globalThis.clearTimeout(...args),
  onBackgroundError = () => {},
} = {}) {
  const state = ref(null);
  const selectedId = ref("");
  const stateLoading = ref(false);
  const pollLoading = ref(false);
  const pollingActive = ref(false);
  const lastError = ref("");

  const summaryFreshness = reactive(new Map());
  const detailFreshness = reactive(new Map());
  const detailLoadingIds = reactive(new Set());
  const detailErrors = reactive(new Map());
  let stateFreshness = null;
  let sequence = 0;
  let operationTail = Promise.resolve();
  let refreshPromise = null;
  let pollPromise = null;
  let active = false;
  let timerId = null;
  let visibilityListening = false;
  const detailRequests = new Map();

  const records = computed(() => {
    const currentState = state.value;
    const currentRecords = recordsObject(currentState);
    return Array.isArray(currentState?.ordered_ids)
      ? currentState.ordered_ids.map((id) => currentRecords[id])
      : [];
  });
  const summary = computed(() => buildSubscriptionSummary(records.value));
  const selected = computed(() => {
    const id = selectedId.value;
    if (!id) return null;
    return recordFrom(recordsObject(state.value), id);
  });
  const loading = computed(() => stateLoading.value || pollLoading.value);

  function nextSequence() {
    sequence += 1;
    return sequence;
  }

  function enqueue(operation) {
    const result = operationTail.catch(() => undefined).then(operation);
    operationTail = result.catch(() => undefined);
    return result;
  }

  function setSelectedId(id) {
    if (id == null || id === "") {
      selectedId.value = "";
      return;
    }
    const normalized = requestedSubscriptionId(id);
    if (!normalized) throw new TypeError("subscription id is invalid");
    selectedId.value = normalized;
  }

  function getById(id) {
    const normalized = requestedSubscriptionId(id);
    return normalized ? recordFrom(recordsObject(state.value), normalized) : null;
  }

  function hasFreshDetail(id) {
    const normalized = requestedSubscriptionId(id);
    if (!normalized || !recordFrom(recordsObject(state.value), normalized)) return false;
    return (
      compareProjectionFreshness(
        summaryFreshness.get(normalized),
        detailFreshness.get(normalized),
      ) <= 0
    );
  }

  function isDetailLoading(id) {
    const normalized = requestedSubscriptionId(id);
    return normalized ? detailLoadingIds.has(normalized) : false;
  }

  function detailError(id) {
    const normalized = requestedSubscriptionId(id);
    return normalized ? detailErrors.get(normalized) || "" : "";
  }

  function mergeRecordValue({
    record,
    id,
    requestSequence,
    projection,
    currentRecords = recordsObject(state.value),
  }) {
    const incomingFreshness = freshness(record, requestSequence);
    const currentSummaryFreshness = summaryFreshness.get(id);
    const current = recordFrom(currentRecords, id);
    if (current && compareFreshness(incomingFreshness, currentSummaryFreshness) < 0) {
      return current;
    }

    // This map is the single mutable entity authority. A summary page may refresh
    // summary-owned fields, but must not erase detail-only fields already present.
    const merged = current ? { ...current, ...record } : record;
    if (
      !currentSummaryFreshness ||
      compareFreshness(incomingFreshness, currentSummaryFreshness) > 0
    ) {
      summaryFreshness.set(id, incomingFreshness);
    }
    if (projection === SUBSCRIPTION_DETAIL_PROJECTION) {
      detailFreshness.set(id, incomingFreshness);
    }
    return merged;
  }

  function mergeDetail(detail, id, requestSequence = nextSequence()) {
    const normalizedId = requestedSubscriptionId(id);
    if (!normalizedId) throw new TypeError("subscription id is invalid");
    const record = nestedDetailEntity(detail, normalizedId);
    const currentState = state.value;
    const currentRecords = recordsObject(currentState);
    const merged = mergeRecordValue({
      record,
      id: normalizedId,
      requestSequence,
      projection: SUBSCRIPTION_DETAIL_PROJECTION,
      currentRecords,
    });
    const currentOrder = Array.isArray(currentState?.ordered_ids) ? currentState.ordered_ids : [];
    const appendToSummaryOrder = !Object.hasOwn(currentRecords, normalizedId);
    state.value = {
      next_cursor: null,
      ordered_ids: appendToSummaryOrder ? [...currentOrder, normalizedId] : currentOrder,
      records: {
        ...currentRecords,
        [normalizedId]: merged,
      },
    };
    return merged;
  }

  function cancelDetailLoad(id) {
    const normalized = requestedSubscriptionId(id);
    if (!normalized) return;
    detailRequests.get(normalized)?.controller.abort();
  }

  function cancelAllDetailLoads() {
    for (const request of detailRequests.values()) request.controller.abort();
  }

  function loadDetail(id, { force = false } = {}) {
    const normalizedId = requestedSubscriptionId(id);
    if (!normalizedId) return Promise.reject(new TypeError("subscription id is invalid"));
    if (!force && hasFreshDetail(normalizedId)) {
      return Promise.resolve(getById(normalizedId));
    }

    const activeRequest = detailRequests.get(normalizedId);
    if (activeRequest && !force) return activeRequest.promise;
    activeRequest?.controller.abort();

    const controller = new AbortController();
    const requestIdentity = { controller, promise: null };
    detailLoadingIds.add(normalizedId);
    detailErrors.delete(normalizedId);

    const request = (async () => {
      for (let attempt = 0; attempt <= MAX_STALE_DETAIL_RETRIES; attempt += 1) {
        const requestSequence = nextSequence();
        const detail = await transport.loadDetail(normalizedId, { signal: controller.signal });
        if (controller.signal.aborted) {
          const error = new Error("The request was aborted");
          error.name = "AbortError";
          throw error;
        }
        mergeDetail(detail, normalizedId, requestSequence);
        if (hasFreshDetail(normalizedId)) {
          detailErrors.delete(normalizedId);
          return getById(normalizedId);
        }
      }
      throw new Error("订阅详情响应已过期，请重试");
    })()
      .catch((error) => {
        if (!isAbortError(error)) detailErrors.set(normalizedId, errorMessage(error));
        throw error;
      })
      .finally(() => {
        if (detailRequests.get(normalizedId) !== requestIdentity) return;
        detailRequests.delete(normalizedId);
        detailLoadingIds.delete(normalizedId);
      });
    requestIdentity.promise = request;
    detailRequests.set(normalizedId, requestIdentity);
    return request;
  }

  function applyWholeState(incomingState, requestSequence) {
    const keys =
      incomingState && typeof incomingState === "object" && !Array.isArray(incomingState)
        ? Object.keys(incomingState)
        : [];
    if (
      keys.length !== 3 ||
      !Object.hasOwn(incomingState, "next_cursor") ||
      incomingState.next_cursor !== null ||
      !Object.hasOwn(incomingState, "ordered_ids") ||
      !Array.isArray(incomingState.ordered_ids) ||
      !Object.hasOwn(incomingState, "records") ||
      !incomingState.records ||
      typeof incomingState.records !== "object" ||
      Array.isArray(incomingState.records)
    ) {
      throw new TypeError("subscription transport returned an invalid list state");
    }
    const normalizedState = incomingState;
    const incomingRecords = recordsObject(normalizedState);
    const incomingEntries = normalizedStoreRecordEntries(incomingRecords);
    const incomingOrder = normalizedSummaryOrder(normalizedState, incomingEntries);
    const incomingStateFreshness = freshness(normalizedState, requestSequence);
    if (state.value && compareFreshness(incomingStateFreshness, stateFreshness) < 0) {
      return state.value;
    }

    const currentRecords = recordsObject(state.value);
    const nextRecords = Object.create(null);
    const nextOrder = [...incomingOrder];
    const seenIds = new Set();

    for (const [id, record] of incomingEntries) {
      seenIds.add(id);
      nextRecords[id] = mergeRecordValue({
        record,
        id,
        requestSequence,
        projection: SUBSCRIPTION_SUMMARY_PROJECTION,
        currentRecords,
      });
    }

    for (const [id, record] of currentRecordEntries(state.value, currentRecords)) {
      if (seenIds.has(id)) continue;
      const currentRecordFreshness = summaryFreshness.get(id);
      if (currentRecordFreshness && currentRecordFreshness.sequence > requestSequence) {
        nextRecords[id] = record;
        nextOrder.push(id);
        continue;
      }
      summaryFreshness.delete(id);
      detailFreshness.delete(id);
    }

    state.value = {
      next_cursor: null,
      ordered_ids: nextOrder,
      records: nextRecords,
    };
    stateFreshness = incomingStateFreshness;
    lastError.value = "";
    return state.value;
  }

  async function loadState() {
    const requestSequence = nextSequence();
    const incomingState = await transport.load();
    return applyWholeState(incomingState, requestSequence);
  }

  function refresh() {
    if (refreshPromise) return refreshPromise;
    stateLoading.value = true;
    const request = enqueue(loadState);
    const wrappedRequest = request
      .catch((error) => {
        lastError.value = errorMessage(error);
        throw error;
      })
      .finally(() => {
        stateLoading.value = false;
        if (refreshPromise === wrappedRequest) refreshPromise = null;
      });
    refreshPromise = wrappedRequest;
    return wrappedRequest;
  }

  function poll() {
    if (pollPromise) return pollPromise;
    pollLoading.value = true;
    const request = enqueue(async () => {
      const outcome = await transport.poll();
      const nextState = await loadState();
      return { outcome, state: nextState };
    });
    const wrappedRequest = request
      .catch((error) => {
        lastError.value = errorMessage(error);
        throw error;
      })
      .finally(() => {
        pollLoading.value = false;
        if (pollPromise === wrappedRequest) pollPromise = null;
      });
    pollPromise = wrappedRequest;
    return wrappedRequest;
  }

  function clearTimer() {
    if (timerId == null) return;
    clearTimeoutFn(timerId);
    timerId = null;
  }

  function scheduleNext() {
    if (!active || hiddenDocument(documentRef) || timerId != null) return;
    timerId = setTimeoutFn(() => {
      timerId = null;
      void runBackgroundRefresh();
    }, pollIntervalMs);
  }

  async function runBackgroundRefresh() {
    if (!active || hiddenDocument(documentRef)) return;
    try {
      await refresh();
    } catch (error) {
      onBackgroundError(error);
    } finally {
      scheduleNext();
    }
  }

  function handleVisibilityChange() {
    if (!active) return;
    if (hiddenDocument(documentRef)) {
      clearTimer();
      return;
    }
    clearTimer();
    void runBackgroundRefresh();
  }

  function listenForVisibility() {
    if (visibilityListening || typeof documentRef?.addEventListener !== "function") return;
    documentRef.addEventListener("visibilitychange", handleVisibilityChange);
    visibilityListening = true;
  }

  function stopListeningForVisibility() {
    if (!visibilityListening || typeof documentRef?.removeEventListener !== "function") return;
    documentRef.removeEventListener("visibilitychange", handleVisibilityChange);
    visibilityListening = false;
  }

  function start() {
    if (active) return refreshPromise || Promise.resolve(state.value);
    active = true;
    pollingActive.value = true;
    listenForVisibility();
    if (hiddenDocument(documentRef)) return Promise.resolve(state.value);
    return runBackgroundRefresh();
  }

  function stop() {
    active = false;
    pollingActive.value = false;
    clearTimer();
    stopListeningForVisibility();
  }

  function dispose() {
    stop();
    cancelAllDetailLoads();
  }

  return Object.freeze({
    state: readonly(state),
    records,
    summary,
    selectedId: readonly(selectedId),
    selected,
    loading,
    pollingActive: readonly(pollingActive),
    lastError: readonly(lastError),
    setSelectedId,
    getById,
    hasFreshDetail,
    isDetailLoading,
    detailError,
    mergeDetail,
    loadDetail,
    cancelDetailLoad,
    cancelAllDetailLoads,
    refresh,
    poll,
    start,
    stop,
    dispose,
  });
}
