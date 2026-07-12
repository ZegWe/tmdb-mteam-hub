import { watch } from "vue";

function firstQueryValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

function normalizedSource(value) {
  return value === "douban" ? "douban" : "tmdb";
}

function normalizedPage(value) {
  const raw = String(firstQueryValue(value) || "");
  if (!/^\d+$/.test(raw)) return 1;
  const page = Number(raw);
  return Number.isSafeInteger(page) && page > 0 ? page : 1;
}

export function searchStateFromQuery(query = {}) {
  const source = normalizedSource(firstQueryValue(query.source));
  return {
    source,
    query: String(firstQueryValue(query.q) || "").trim(),
    page: source === "douban" ? normalizedPage(query.page) : 1,
  };
}

export function searchQueryFromState(value = {}) {
  const source = normalizedSource(value.source);
  const query = String(value.query || "").trim();
  const page = source === "douban" ? normalizedPage(value.page) : 1;
  const routeQuery = {};
  if (source === "douban") routeQuery.source = source;
  if (query) routeQuery.q = query;
  if (source === "douban" && page > 1) routeQuery.page = String(page);
  return routeQuery;
}

function searchStateKey(value = {}) {
  const normalized = searchStateFromQuery(searchQueryFromState(value));
  return JSON.stringify([normalized.source, normalized.query, normalized.page]);
}

export function createSearchRouteSync({ route, router, store, onError = () => {} }) {
  let disposed = false;

  function report(error) {
    if (!disposed) onError(error);
  }

  function loadState(state, { force = false, reportErrors = true } = {}) {
    store.hydrateRouteState(state);
    if (!state.query || (!force && store.hasResultsFor(state))) return Promise.resolve(null);
    const request = store.search(state.page);
    if (!reportErrors) return request;
    return request.catch((error) => {
      report(error);
      return null;
    });
  }

  const stopWatching = watch(
    [() => String(route.name || ""), () => searchStateKey(searchStateFromQuery(route.query))],
    ([routeName]) => {
      if (disposed || routeName !== "main") return;
      void loadState(searchStateFromQuery(route.query));
    },
    { immediate: true },
  );

  function navigate(state, { force = false } = {}) {
    const normalized = searchStateFromQuery(searchQueryFromState(state));
    const current = searchStateFromQuery(route.query);
    if (searchStateKey(normalized) === searchStateKey(current)) {
      return loadState(normalized, { force, reportErrors: false });
    }
    return router.push({ name: "main", query: searchQueryFromState(normalized) });
  }

  function selectSource(source) {
    const state = { source: normalizedSource(source), query: store.query.value, page: 1 };
    store.hydrateRouteState(state);
    return navigate(state);
  }

  function submit(page = 1) {
    const state = {
      source: store.source.value,
      query: store.query.value,
      page,
    };
    if (!String(state.query || "").trim()) {
      store.hydrateRouteState(state);
      return store.search(state.page);
    }
    return navigate(state, { force: true });
  }

  function dispose() {
    disposed = true;
    stopWatching();
  }

  return Object.freeze({ selectSource, submit, dispose });
}
