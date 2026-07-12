import { createSearchStore } from "./store.js";

export const SEARCH_CONTEXT_KEY = Symbol("search-context");

export function createSearchContext({ store = createSearchStore() } = {}) {
  let detailOpenedFromSearch = false;

  function markDetailOpened() {
    detailOpenedFromSearch = true;
  }

  function clearDetailOrigin() {
    detailOpenedFromSearch = false;
  }

  function consumeDetailOrigin() {
    const openedFromSearch = detailOpenedFromSearch;
    detailOpenedFromSearch = false;
    return openedFromSearch;
  }

  function dispose() {
    clearDetailOrigin();
    store.dispose();
  }

  return Object.freeze({
    store,
    markDetailOpened,
    clearDetailOrigin,
    consumeDetailOrigin,
    dispose,
  });
}
