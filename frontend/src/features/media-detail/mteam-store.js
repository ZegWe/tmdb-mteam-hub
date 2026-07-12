import { reactive } from "vue";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";
import { imdbFromDetail } from "./domain.js";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function extractTorrentRows(response) {
  return Array.isArray(response?.items) ? response.items : [];
}

function torrentSources({ mediaType, data, doubanId }) {
  if (!data || typeof data !== "object") return [];

  const imdb = imdbFromDetail(data);
  const keyword = mediaType === "douban" ? data.title || "" : data.original_title || "";
  const sources = [];
  if (imdb) sources.push({ source: "imdb", label: "IMDb", params: { imdb_id: imdb } });
  if (doubanId) {
    sources.push({
      source: "douban",
      label: "豆瓣 ID",
      params: { douban_id: String(doubanId) },
    });
  }
  if (keyword) sources.push({ source: "keyword", label: "原标题", params: { keyword } });
  return sources;
}

export function createMteamTorrentStore({ transport, requests = createLatestRequestWins() } = {}) {
  const state = reactive({
    sources: [],
    activeSource: "",
    rows: [],
    loading: false,
    error: "",
  });
  const cache = Object.create(null);
  let contextRevision = 0;
  let disposed = false;

  function clearState() {
    state.sources = [];
    state.activeSource = "";
    state.rows = [];
    state.loading = false;
    state.error = "";
    for (const key of Object.keys(cache)) delete cache[key];
  }

  function reset() {
    contextRevision += 1;
    requests.cancel();
    clearState();
  }

  function initialize({ mediaType = "", data = null, doubanId = "" } = {}) {
    reset();
    if (disposed || !data) return null;

    const revision = contextRevision;
    state.sources = torrentSources({ mediaType, data, doubanId });
    if (!state.sources.length) return null;
    return select(state.sources[0].source, revision);
  }

  async function select(source, revision = contextRevision) {
    if (disposed || revision !== contextRevision) return null;

    const selectedSource = String(source || "");
    requests.cancel();
    state.loading = false;
    state.activeSource = selectedSource;
    state.error = "";
    if (Object.hasOwn(cache, selectedSource)) {
      state.rows = cache[selectedSource];
      return state.rows;
    }

    const selected = state.sources.find((item) => item.source === selectedSource);
    if (!selected) return null;

    let requestId = 0;
    state.loading = true;
    try {
      const response = await requests.run(({ signal, requestId: currentRequestId }) => {
        requestId = currentRequestId;
        return transport.searchTorrents({ source: selectedSource, ...selected.params }, { signal });
      });
      if (disposed || revision !== contextRevision) return null;
      const rows = extractTorrentRows(response);
      cache[selectedSource] = rows;
      state.rows = rows;
      return rows;
    } catch (error) {
      if (error instanceof StaleRequestError) return null;
      if (!disposed && revision === contextRevision) {
        state.error = errorMessage(error);
        state.rows = [];
      }
      return null;
    } finally {
      if (requests.isCurrent(requestId) && !disposed && revision === contextRevision) {
        state.loading = false;
      }
    }
  }

  function dispose() {
    disposed = true;
    reset();
  }

  return Object.freeze({ state, initialize, select, reset, dispose });
}
