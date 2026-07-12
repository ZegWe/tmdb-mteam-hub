import { computed, reactive, readonly, ref } from "vue";
import { getOperationLogs } from "../../shared/api/endpoints/logs.js";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";
import { createOperationLogFilters, operationLogSummary } from "./domain.js";

const DEFAULT_OPERATION_LOG_PAGE_SIZE = 30;

const defaultTransport = Object.freeze({
  load: (filters, options) => getOperationLogs(filters, options),
});

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

export function createLogsStore({
  transport = defaultTransport,
  requests = createLatestRequestWins(),
  pageSize = DEFAULT_OPERATION_LOG_PAGE_SIZE,
  setTimeoutFn = (...args) => globalThis.setTimeout(...args),
  clearTimeoutFn = (...args) => globalThis.clearTimeout(...args),
} = {}) {
  const entries = ref([]);
  const filters = reactive(createOperationLogFilters());
  const appliedFilters = reactive(createOperationLogFilters());
  const page = reactive({
    page: 1,
    page_size: Math.max(1, Number(pageSize) || DEFAULT_OPERATION_LOG_PAGE_SIZE),
    total: 0,
    has_more: false,
  });
  const loading = ref(false);
  const lastError = ref("");
  const toast = reactive({ message: "", kind: "ok" });
  let toastTimer = 0;

  const summary = computed(() => operationLogSummary(page, entries.value, appliedFilters));

  function setFilters(value = {}) {
    Object.assign(filters, createOperationLogFilters(value));
  }

  function applyFilters(value = {}) {
    const normalized = createOperationLogFilters(value);
    Object.assign(filters, normalized);
    Object.assign(appliedFilters, normalized);
  }

  function resetFilters() {
    setFilters();
  }

  function showToast(message, kind = "ok") {
    toast.message = message;
    toast.kind = kind;
    clearTimeoutFn(toastTimer);
    toastTimer = setTimeoutFn(() => {
      toast.message = "";
    }, 3800);
  }

  async function load({ page: requestedPage = 1, append = false, silent = false } = {}) {
    const targetPage = Math.max(1, Number(requestedPage) || 1);
    const requestPageSize = Math.max(1, Number(page.page_size) || DEFAULT_OPERATION_LOG_PAGE_SIZE);
    const requestFilters = createOperationLogFilters(appliedFilters);
    const appendResults = append === true;
    const notify = silent !== true;
    let requestId = 0;
    lastError.value = "";
    loading.value = true;

    try {
      const data = await requests.run(({ signal, requestId: currentRequestId }) => {
        requestId = currentRequestId;
        return transport.load(
          {
            page: targetPage,
            page_size: requestPageSize,
            category: requestFilters.category,
            status: requestFilters.status,
            q: requestFilters.q,
          },
          { signal },
        );
      });
      const items = Array.isArray(data?.items) ? data.items : [];
      entries.value = appendResults ? [...entries.value, ...items] : items;
      page.page = Number(data?.page || targetPage) || targetPage;
      page.page_size = Number(data?.page_size || requestPageSize) || requestPageSize;
      page.total = Number(data?.total || 0);
      page.has_more = data?.has_more === true;
      if (notify) showToast("日志已加载", "ok");
      return data;
    } catch (error) {
      if (error instanceof StaleRequestError) return null;
      lastError.value = `加载日志失败：${errorMessage(error)}`;
      if (notify) showToast(lastError.value, "err");
      return null;
    } finally {
      if (requests.isCurrent(requestId)) loading.value = false;
    }
  }

  function refresh(options = {}) {
    return load({ page: 1, ...options });
  }

  function loadMore() {
    if (loading.value || !page.has_more) return Promise.resolve(null);
    return load({ page: Number(page.page || 1) + 1, append: true });
  }

  function dispose() {
    requests.cancel();
    loading.value = false;
    clearTimeoutFn(toastTimer);
    toastTimer = 0;
  }

  return Object.freeze({
    entries: readonly(entries),
    filters,
    appliedFilters: readonly(appliedFilters),
    page: readonly(page),
    loading: readonly(loading),
    lastError: readonly(lastError),
    toast: readonly(toast),
    summary,
    setFilters,
    applyFilters,
    resetFilters,
    load,
    refresh,
    loadMore,
    dispose,
  });
}
