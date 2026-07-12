import { reactive } from "vue";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function clearReactiveRecord(record) {
  for (const key of Object.keys(record)) delete record[key];
}

export function createTvSeasonStore({ transport } = {}) {
  const episodes = reactive({});
  const loading = reactive({});
  const errors = reactive({});
  const controllers = new Map();
  let contextRevision = 0;
  let currentTvId = "";
  let disposed = false;

  function isCurrentContext(revision, tvId) {
    return !disposed && revision === contextRevision && String(tvId || "") === currentTvId;
  }

  function reset() {
    contextRevision += 1;
    currentTvId = "";
    for (const controller of controllers.values()) controller.abort();
    controllers.clear();
    clearReactiveRecord(episodes);
    clearReactiveRecord(loading);
    clearReactiveRecord(errors);
  }

  function initialize({ tvId = "" } = {}) {
    reset();
    if (disposed) return null;
    currentTvId = String(tvId || "");
    return currentTvId;
  }

  async function load(seasonNumber) {
    const normalizedSeason = Number(seasonNumber);
    if (
      disposed ||
      !Number.isFinite(normalizedSeason) ||
      episodes[normalizedSeason] ||
      loading[normalizedSeason]
    ) {
      return null;
    }

    const revision = contextRevision;
    const tvId = currentTvId;
    const controller = new AbortController();
    controllers.set(normalizedSeason, controller);
    loading[normalizedSeason] = true;
    errors[normalizedSeason] = "";
    try {
      const data = await transport.loadSeason(tvId, normalizedSeason, {
        signal: controller.signal,
      });
      if (!isCurrentContext(revision, tvId)) return null;
      episodes[normalizedSeason] = Array.isArray(data?.episodes) ? data.episodes : [];
      return data;
    } catch (error) {
      if (isCurrentContext(revision, tvId)) errors[normalizedSeason] = errorMessage(error);
      return null;
    } finally {
      if (controllers.get(normalizedSeason) === controller) {
        controllers.delete(normalizedSeason);
      }
      if (isCurrentContext(revision, tvId)) loading[normalizedSeason] = false;
    }
  }

  function dispose() {
    disposed = true;
    reset();
  }

  return Object.freeze({ episodes, loading, errors, initialize, load, reset, dispose });
}
