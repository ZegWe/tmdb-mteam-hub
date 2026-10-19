import { computed, reactive } from "vue";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";
import {
  doubanFromDetail,
  doubanMetaRows,
  doubanUrlFromDetail,
  imdbFromDetail,
  imdbHref,
  tmdbMetaRows,
} from "./domain.js";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

export function createMediaDetailPrimaryStore({
  transport,
  requests = createLatestRequestWins(),
} = {}) {
  const state = reactive({
    loading: false,
    error: "",
    mediaType: "",
    numericId: "",
    doubanId: "",
    data: null,
  });
  const title = computed(() => state.data?.title || "");
  const date = computed(() => {
    if (state.mediaType === "douban") return state.data?.date_published || "";
    return state.data?.release_date || state.data?.first_air_date || "";
  });
  const overview = computed(() =>
    state.mediaType === "douban" ? state.data?.summary || "" : state.data?.overview || "",
  );
  const poster = computed(() => {
    if (state.mediaType === "douban") {
      return state.data?.poster_url || state.data?.image || "";
    }
    return state.data?.poster_url || "";
  });
  const seasons = computed(() =>
    Array.isArray(state.data?.seasons)
      ? [...state.data.seasons].sort(
          (left, right) => (left.season_number ?? 0) - (right.season_number ?? 0),
        )
      : [],
  );
  const metaRows = computed(() => {
    if (!state.data) return [];
    return state.mediaType === "douban"
      ? doubanMetaRows(state.data)
      : tmdbMetaRows(state.data, state.mediaType);
  });
  const externalLinks = computed(() => {
    const data = state.data;
    if (!data) return [];

    const links = [];
    if (state.mediaType !== "douban" && state.numericId) {
      const type = state.mediaType === "tv" ? "tv" : "movie";
      links.push({
        href: `https://www.themoviedb.org/${type}/${state.numericId}`,
        label: `TMDB · ${state.numericId}`,
      });
    }

    const imdb = imdbFromDetail(data);
    const imdbUrl = imdbHref(imdb);
    if (imdb && imdbUrl) links.push({ href: imdbUrl, label: `IMDb · ${imdb}` });

    const doubanId = state.mediaType === "douban" ? state.doubanId : doubanFromDetail(data);
    const doubanUrl =
      state.mediaType === "douban"
        ? data.url || `https://movie.douban.com/subject/${doubanId}/`
        : doubanUrlFromDetail(data);
    if (doubanId && doubanUrl) links.push({ href: doubanUrl, label: `豆瓣 · ${doubanId}` });
    return links;
  });
  const model = computed(() => ({
    data: state.data,
    mediaType: state.mediaType,
    numericId: state.numericId,
    doubanId: state.doubanId,
    title: title.value,
    date: date.value,
    overview: overview.value,
    poster: poster.value,
    seasons: seasons.value,
    metaRows: metaRows.value,
    externalLinks: externalLinks.value,
  }));
  const pageTitle = computed(() => title.value || "影视详情");
  const pageSubtitle = computed(() => {
    if (state.mediaType === "douban") return "豆瓣资料、标记与 M-Team 种子";
    if (state.mediaType === "tv") return "剧集资料、分集与 M-Team 种子";
    return "电影资料、豆瓣标记与 M-Team 种子";
  });

  let contextRevision = 0;
  let disposed = false;

  function isCurrentContext(revision) {
    return !disposed && revision === contextRevision;
  }

  function reset() {
    contextRevision += 1;
    requests.cancel();
    state.loading = false;
    state.error = "";
    state.mediaType = "";
    state.numericId = "";
    state.doubanId = "";
    state.data = null;
  }

  async function load({ mediaType, id } = {}) {
    reset();
    if (disposed) return null;

    const revision = contextRevision;
    state.loading = true;
    state.mediaType = mediaType;
    state.numericId = String(id || "");
    let requestId = 0;
    try {
      const data = await requests.run(({ signal, requestId: currentRequestId }) => {
        requestId = currentRequestId;
        return transport.loadDetail(mediaType, id, { signal });
      });
      if (!isCurrentContext(revision)) return null;
      state.data = data;
      state.doubanId =
        mediaType === "douban" ? String(data?.subject_id || "") : doubanFromDetail(data) || "";
      return data;
    } catch (error) {
      if (error instanceof StaleRequestError) return null;
      if (isCurrentContext(revision)) state.error = errorMessage(error);
      throw error;
    } finally {
      if (requests.isCurrent(requestId) && isCurrentContext(revision)) state.loading = false;
    }
  }

  function dispose() {
    disposed = true;
    reset();
  }

  return Object.freeze({ state, model, pageTitle, pageSubtitle, load, reset, dispose });
}
