import { reactive } from "vue";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";
import { classifyTvTorrentTitle, imdbFromDetail } from "./domain.js";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function extractTorrentRows(response) {
  return Array.isArray(response?.items) ? response.items : [];
}

function torrentSources({ mediaType, data, doubanId, seasonNumber }) {
  if (!data || typeof data !== "object") return [];

  const imdb = imdbFromDetail(data);
  const keyword = mediaType === "douban" ? data.title || "" : data.original_title || "";
  const sources = [];
  if (mediaType === "tv" && keyword && seasonNumber) {
    sources.push({
      source: "tv_season",
      label: "第 " + seasonNumber + " 季",
      params: {
        source: "keyword",
        keyword: keyword + " S" + String(seasonNumber).padStart(2, "0"),
      },
    });
  }
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
    mediaType: "",
    data: null,
    doubanId: "",
    seasonNumber: null,
    episodeTotal: null,
    seasons: [],
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
    state.mediaType = "";
    state.data = null;
    state.doubanId = "";
    state.seasonNumber = null;
    state.episodeTotal = null;
    state.seasons = [];
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
    state.mediaType = mediaType;
    state.data = data;
    state.doubanId = doubanId;
    state.seasons = mediaType === "tv" ? normalizedSeasons(data?.seasons) : [];
    const defaultSeason =
      state.seasons.findLast((season) => season.episode_count) ?? state.seasons.at(-1);
    state.seasonNumber = defaultSeason?.season_number ?? null;
    state.episodeTotal = defaultSeason?.episode_count ?? null;
    state.sources = torrentSources({
      mediaType,
      data,
      doubanId,
      seasonNumber: state.seasonNumber,
    });
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
      const rows = decorateRows(extractTorrentRows(response), state);
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

  function selectSeason(seasonNumber) {
    if (disposed || state.mediaType !== "tv") return null;
    const selected = state.seasons.find((season) => season.season_number === Number(seasonNumber));
    if (!selected || selected.season_number === state.seasonNumber) return null;
    contextRevision += 1;
    requests.cancel();
    for (const key of Object.keys(cache)) delete cache[key];
    state.seasonNumber = selected.season_number;
    state.episodeTotal = selected.episode_count;
    state.sources = torrentSources({
      mediaType: state.mediaType,
      data: state.data,
      doubanId: state.doubanId,
      seasonNumber: state.seasonNumber,
    });
    state.rows = [];
    state.error = "";
    state.activeSource = "";
    return state.sources.length ? select(state.sources[0].source, contextRevision) : null;
  }

  function dispose() {
    disposed = true;
    reset();
  }

  return Object.freeze({ state, initialize, select, selectSeason, reset, dispose });
}

function normalizedSeasons(seasons) {
  if (!Array.isArray(seasons)) return [];
  return seasons
    .filter((season) => Number.isInteger(season?.season_number) && season.season_number > 0)
    .map((season) => ({
      season_number: season.season_number,
      episode_count:
        Number.isInteger(season.episode_count) && season.episode_count > 0
          ? season.episode_count
          : null,
      name: String(season.name || ""),
    }))
    .sort((left, right) => left.season_number - right.season_number);
}

function decorateRows(rows, state) {
  if (state.mediaType !== "tv") return rows;
  return rows.map((row) => ({
    ...row,
    tv_match: classifyTvTorrentTitle(
      row?.name || row?.title || "",
      state.seasonNumber,
      state.episodeTotal,
    ),
  }));
}
