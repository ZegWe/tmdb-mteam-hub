import { computed, reactive, readonly, ref } from "vue";
import { searchDouban, searchTmdb } from "../../shared/api/endpoints/search.js";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";

const defaultTransport = Object.freeze({
  searchTmdb: (query, options) => searchTmdb(query, options),
  searchDouban: (query, options) => searchDouban(query, options),
});

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function normalizedSource(value) {
  return value === "douban" ? "douban" : "tmdb";
}

function normalizedPage(value) {
  return Math.max(1, Number(value) || 1);
}

function resultKey(source, query, page) {
  return JSON.stringify([
    normalizedSource(source),
    String(query || "").trim(),
    normalizedSource(source) === "douban" ? normalizedPage(page) : 1,
  ]);
}

export function createSearchStore({
  transport = defaultTransport,
  requests = createLatestRequestWins(),
} = {}) {
  const source = ref("tmdb");
  const query = ref("");
  const movies = ref([]);
  const tv = ref([]);
  const loading = ref(false);
  const loadingText = ref("正在搜索 TMDB…");
  const lastError = ref("");
  const doubanPage = reactive({ page: 1, page_size: 20, has_more: false });
  let currentResultKey = "";

  const showDoubanPager = computed(
    () => source.value === "douban" && (movies.value.length > 0 || doubanPage.page > 1),
  );
  const doubanPagerText = computed(() => {
    const start = movies.value.length
      ? (Number(doubanPage.page || 1) - 1) * Number(doubanPage.page_size || 20) + 1
      : 0;
    const end = start ? start + movies.value.length - 1 : 0;
    return `第 ${doubanPage.page} 页 · ${start && end ? `${start}-${end}` : "0"}`;
  });

  function clearResults() {
    movies.value = [];
    tv.value = [];
    doubanPage.page = 1;
    doubanPage.has_more = false;
    currentResultKey = "";
  }

  function setSource(nextSource) {
    const next = normalizedSource(nextSource);
    if (source.value === next) return;
    requests.cancel();
    loading.value = false;
    source.value = next;
    clearResults();
    lastError.value = "";
  }

  function hydrateRouteState({ source: nextSource, query: nextQuery } = {}) {
    setSource(nextSource);
    query.value = String(nextQuery || "");
    lastError.value = "";
    if (!query.value.trim()) clearResults();
  }

  function hasResultsFor({ source: nextSource, query: nextQuery, page = 1 } = {}) {
    return currentResultKey === resultKey(nextSource, nextQuery, page);
  }

  async function search(pageNumber = 1) {
    const normalizedQuery = query.value.trim();
    if (!normalizedQuery) {
      requests.cancel();
      loading.value = false;
      lastError.value = "请输入搜索词";
      throw new Error(lastError.value);
    }

    const requestSource = source.value;
    const targetPage = normalizedPage(pageNumber);
    const pageSize = Math.max(1, Number(doubanPage.page_size) || 20);
    const targetResultKey = resultKey(requestSource, normalizedQuery, targetPage);
    let requestId = 0;
    if (currentResultKey !== targetResultKey) clearResults();
    loading.value = true;
    loadingText.value = requestSource === "douban" ? "正在搜索豆瓣…" : "正在搜索 TMDB…";
    lastError.value = "";

    try {
      const data = await requests.run(({ signal, requestId: currentRequestId }) => {
        requestId = currentRequestId;
        return requestSource === "douban"
          ? transport.searchDouban(normalizedQuery, {
              page: targetPage,
              pageSize,
              signal,
            })
          : transport.searchTmdb(normalizedQuery, { signal });
      });

      if (requestSource === "douban") {
        movies.value = Array.isArray(data?.items) ? data.items : [];
        tv.value = [];
        doubanPage.page = Number(data?.page || targetPage) || targetPage;
        doubanPage.page_size = Number(data?.page_size || pageSize) || pageSize;
        doubanPage.has_more = data?.has_more === true;
      } else {
        movies.value = Array.isArray(data?.movies) ? data.movies : [];
        tv.value = Array.isArray(data?.tv) ? data.tv : [];
        doubanPage.page = 1;
        doubanPage.has_more = false;
      }
      currentResultKey = targetResultKey;
      return data;
    } catch (error) {
      if (error instanceof StaleRequestError) return null;
      lastError.value = errorMessage(error);
      throw error;
    } finally {
      if (requests.isCurrent(requestId)) loading.value = false;
    }
  }

  function loadDoubanPage(pageNumber) {
    if (loading.value || source.value !== "douban") return Promise.resolve(null);
    return search(Math.max(1, Number(pageNumber) || 1));
  }

  function dispose() {
    requests.cancel();
    loading.value = false;
    query.value = "";
    clearResults();
    lastError.value = "";
  }

  return Object.freeze({
    source: readonly(source),
    query,
    movies: readonly(movies),
    tv: readonly(tv),
    loading: readonly(loading),
    loadingText: readonly(loadingText),
    lastError: readonly(lastError),
    doubanPage: readonly(doubanPage),
    showDoubanPager,
    doubanPagerText,
    setSource,
    hydrateRouteState,
    hasResultsFor,
    search,
    loadDoubanPage,
    dispose,
  });
}
