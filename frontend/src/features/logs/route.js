import { watch } from "vue";
import { createOperationLogFilters, operationLogFilterKey } from "./domain.js";

function firstQueryValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

export function operationLogFiltersFromQuery(query = {}) {
  return createOperationLogFilters({
    category: firstQueryValue(query.category),
    status: firstQueryValue(query.status),
    q: firstQueryValue(query.q),
  });
}

export function operationLogQueryFromFilters(value = {}) {
  const filters = createOperationLogFilters(value);
  const query = {};
  if (filters.category) query.category = filters.category;
  if (filters.status) query.status = filters.status;
  if (filters.q) query.q = filters.q;
  return query;
}

export function createLogsRouteSync({ route, router, store }) {
  let disposed = false;
  let notifyFilterKey = "";

  const stopWatching = watch(
    [
      () => String(route.name || ""),
      () => operationLogFilterKey(operationLogFiltersFromQuery(route.query)),
    ],
    ([routeName, routeFilterKey]) => {
      if (disposed || routeName !== "logs") return;
      const filters = operationLogFiltersFromQuery(route.query);
      store.applyFilters(filters);
      const notify = routeFilterKey === notifyFilterKey;
      notifyFilterKey = "";
      void store.load({ page: 1, silent: !notify });
    },
    { immediate: true },
  );

  function applyFilters({ replace = false } = {}) {
    const filters = createOperationLogFilters(store.filters);
    store.setFilters(filters);
    const nextFilterKey = operationLogFilterKey(filters);
    const currentFilterKey = operationLogFilterKey(operationLogFiltersFromQuery(route.query));
    if (nextFilterKey === currentFilterKey) {
      store.applyFilters(filters);
      return store.load({ page: 1 });
    }

    notifyFilterKey = nextFilterKey;
    const navigate = replace ? router.replace.bind(router) : router.push.bind(router);
    return navigate({ name: "logs", query: operationLogQueryFromFilters(filters) }).catch(
      (error) => {
        if (notifyFilterKey === nextFilterKey) notifyFilterKey = "";
        throw error;
      },
    );
  }

  function resetFilters() {
    store.resetFilters();
    return applyFilters();
  }

  function dispose() {
    disposed = true;
    notifyFilterKey = "";
    stopWatching();
  }

  return Object.freeze({ applyFilters, resetFilters, dispose });
}
