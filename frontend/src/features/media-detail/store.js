import { computed, ref } from "vue";
import {
  getMediaDetail,
  getDoubanTagHistory,
  getTvSeasonEpisodes,
  saveDoubanInterest as requestSaveDoubanInterest,
  searchMteamTorrents,
} from "../../shared/api/endpoints/media-details.js";
import { createLatestRequestWins } from "../../shared/api/latest-request-wins.js";
import { createDoubanInterestStore } from "./interest-store.js";
import { createMteamTorrentStore } from "./mteam-store.js";
import { createMediaDetailPrimaryStore } from "./primary-store.js";
import { createTvSeasonStore } from "./season-store.js";

const defaultTransport = Object.freeze({
  loadDetail: (mediaType, id, options) => getMediaDetail(mediaType, id, options),
  loadSeason: (tvId, seasonNumber, options) => getTvSeasonEpisodes(tvId, seasonNumber, options),
  loadInterest: (doubanId, options) => getMediaDetail("douban", doubanId, options),
  saveInterest: (doubanId, payload, options) =>
    requestSaveDoubanInterest(doubanId, payload, options),
  searchTorrents: (params, options) => searchMteamTorrents(params, options),
  loadTags: ({ forceRefresh = false, ...requestOptions } = {}) => {
    void forceRefresh;
    return getDoubanTagHistory({ limit: 80, ...requestOptions });
  },
});

export function createMediaDetailStore({
  subscriptionCategories = ref([]),
  transport = defaultTransport,
  primaryRequests = createLatestRequestWins(),
  interestRequests = createLatestRequestWins(),
  torrentRequests = createLatestRequestWins(),
} = {}) {
  const primaryStore = createMediaDetailPrimaryStore({ transport, requests: primaryRequests });
  const primary = primaryStore.state;
  const primaryModel = primaryStore.model;
  const interestStore = createDoubanInterestStore({
    subscriptionCategories,
    transport,
    requests: interestRequests,
  });
  const interest = interestStore.state;
  const interestModel = interestStore.model;
  const seasonStore = createTvSeasonStore({ transport });
  const seasonEpisodes = seasonStore.episodes;
  const seasonLoading = seasonStore.loading;
  const seasonErrors = seasonStore.errors;
  const mteamStore = createMteamTorrentStore({ transport, requests: torrentRequests });
  const mteam = mteamStore.state;
  let routeRevision = 0;
  let disposed = false;
  const model = computed(() => ({
    primary: primaryModel.value,
    interest: interestModel.value,
    seasonEpisodes,
    seasonLoading,
    seasonErrors,
    mteam: {
      sources: mteam.sources,
      activeSource: mteam.activeSource,
      rows: mteam.rows,
      loading: mteam.loading,
      error: mteam.error,
    },
  }));

  function resetOptionalState({
    disposeInterest = false,
    disposeMteam = false,
    disposeSeason = false,
  } = {}) {
    if (disposeInterest) interestStore.dispose();
    else interestStore.reset();
    if (disposeMteam) mteamStore.dispose();
    else mteamStore.reset();
    if (disposeSeason) seasonStore.dispose();
    else seasonStore.reset();
  }

  async function load({ mediaType, id, doubanTags = "" } = {}) {
    const revision = ++routeRevision;
    resetOptionalState();
    seasonStore.initialize({ tvId: String(id || "") });
    const data = await primaryStore.load({ mediaType, id });
    if (disposed || revision !== routeRevision) return null;

    interestStore.initialize({ data, doubanId: primary.doubanId, doubanTags });

    // Primary detail is ready before optional panels begin network work.
    void interestStore.loadTagHistory();
    if (primary.doubanId) void interestStore.hydrate();
    void mteamStore.initialize({
      mediaType: primary.mediaType,
      data: primary.data,
      doubanId: primary.doubanId,
    });
    return data;
  }

  function loadSeason(seasonNumber) {
    return seasonStore.load(seasonNumber);
  }

  function selectTorrentSource(source) {
    return mteamStore.select(source);
  }

  function setInterest(value) {
    interestStore.setInterest(value);
  }

  function updateRating(value) {
    interestStore.updateRating(value);
  }

  function updateCategory(value) {
    interestStore.updateCategory(value);
  }

  function updateTags(value) {
    interestStore.updateTags(value);
  }

  function applyTagSuggestion(tag) {
    interestStore.applyTagSuggestion(tag);
  }

  function saveInterest() {
    return interestStore.save();
  }

  function dispose() {
    disposed = true;
    routeRevision += 1;
    primaryStore.dispose();
    resetOptionalState({ disposeInterest: true, disposeMteam: true, disposeSeason: true });
  }

  return Object.freeze({
    primary,
    interest,
    mteam,
    model,
    pageTitle: primaryStore.pageTitle,
    pageSubtitle: primaryStore.pageSubtitle,
    load,
    loadSeason,
    selectTorrentSource,
    setInterest,
    updateRating,
    updateCategory,
    updateTags,
    applyTagSuggestion,
    saveInterest,
    dispose,
  });
}
