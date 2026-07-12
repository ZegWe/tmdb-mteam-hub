import { computed, reactive, ref } from "vue";
import {
  createLatestRequestWins,
  StaleRequestError,
} from "../../shared/api/latest-request-wins.js";
import { normalizeDoubanTags } from "../../shared/lib/formatters.js";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function initializedTags(data, doubanTags) {
  const tags =
    doubanTags ||
    normalizeDoubanTags(Array.isArray(data?.tags) ? data.tags.join(" ") : data?.tags || "");
  return normalizeDoubanTags(tags);
}

export function createDoubanInterestStore({
  subscriptionCategories = ref([]),
  transport,
  requests = createLatestRequestWins(),
} = {}) {
  const state = reactive({
    loading: false,
    saving: false,
    error: "",
    status: "",
    mark: { interest: "", rating: "", tags: "", category: "" },
    tagHistory: [],
    tagHistoryLoading: false,
    tagHistoryError: "",
  });
  const ratingLabel = computed(() => (state.mark.rating ? `${state.mark.rating} 星` : "未评分"));
  const saveDisabled = computed(
    () =>
      state.saving ||
      !state.mark.interest ||
      (state.mark.interest === "wish" && !state.mark.category),
  );
  const categoryLabel = computed(() => {
    const categories = subscriptionCategories.value;
    if (!state.mark.category) return categories.length ? "选择订阅分类" : "未配置订阅分类";
    const category = categories.find((item) => item.wanted_tag === state.mark.category);
    return category
      ? `${category.name || category.wanted_tag} · ${category.wanted_tag}`
      : state.mark.category;
  });
  const model = computed(() => ({
    loading: state.loading,
    saving: state.saving,
    error: state.error,
    status: state.status,
    mark: { ...state.mark },
    ratingLabel: ratingLabel.value,
    saveDisabled: saveDisabled.value,
    categoryLabel: categoryLabel.value,
    categories: subscriptionCategories.value,
    tagHistory: state.tagHistory,
    tagHistoryLoading: state.tagHistoryLoading,
    tagHistoryError: state.tagHistoryError,
  }));

  let contextRevision = 0;
  let currentDoubanId = "";
  let tagHistoryPromise = null;
  let disposed = false;

  function isCurrentContext(revision, doubanId) {
    return !disposed && revision === contextRevision && String(doubanId || "") === currentDoubanId;
  }

  function reset() {
    contextRevision += 1;
    currentDoubanId = "";
    requests.cancel();
    state.loading = false;
    state.saving = false;
    state.error = "";
    state.status = "";
    Object.assign(state.mark, { interest: "", rating: "", tags: "", category: "" });
  }

  function initialize({ data = null, doubanId = "", doubanTags = "" } = {}) {
    reset();
    if (disposed) return null;

    currentDoubanId = String(doubanId || "");
    const tags = initializedTags(data, doubanTags);
    state.mark.interest =
      data?.user_interest === "wish" || data?.user_interest === "collect" ? data.user_interest : "";
    state.mark.rating = data?.user_rating != null ? String(data.user_rating) : "";
    state.mark.tags = tags;
    state.mark.category = tags.split(/\s+/).filter(Boolean)[0] || "";
    state.status =
      state.mark.interest === "wish" ? "已想看" : state.mark.interest === "collect" ? "已看过" : "";
    return state;
  }

  async function hydrate() {
    const revision = contextRevision;
    const doubanId = currentDoubanId;
    if (disposed || !doubanId) return null;

    state.loading = true;
    state.error = "";
    state.status = state.status || "读取豆瓣状态…";
    let requestId = 0;
    try {
      const data = await requests.run(({ signal, requestId: currentRequestId }) => {
        requestId = currentRequestId;
        return transport.loadInterest(doubanId, { signal });
      });
      if (!isCurrentContext(revision, doubanId)) return null;
      if (data?.user_interest === "wish" || data?.user_interest === "collect") {
        state.mark.interest = data.user_interest;
      }
      state.mark.rating = data?.user_rating != null ? String(data.user_rating) : "";
      state.status =
        data?.user_interest === "wish"
          ? "已想看"
          : data?.user_interest === "collect"
            ? "已看过"
            : "";
      return data;
    } catch (error) {
      if (error instanceof StaleRequestError) return null;
      if (isCurrentContext(revision, doubanId)) {
        state.error = errorMessage(error);
        if (state.status === "读取豆瓣状态…") state.status = "";
      }
      return null;
    } finally {
      if (requests.isCurrent(requestId) && isCurrentContext(revision, doubanId)) {
        state.loading = false;
      }
    }
  }

  async function loadTagHistory(forceRefresh = false) {
    if (state.tagHistory.length && !forceRefresh) return state.tagHistory;
    if (tagHistoryPromise && !forceRefresh) return tagHistoryPromise;
    if (disposed) return state.tagHistory;

    state.tagHistoryLoading = true;
    state.tagHistoryError = "";
    const request = transport
      .loadTags({ forceRefresh })
      .then((data) => {
        if (!disposed) {
          state.tagHistory = Array.isArray(data?.tags) ? data.tags.filter(Boolean) : [];
        }
        return state.tagHistory;
      })
      .catch((error) => {
        if (!disposed) state.tagHistoryError = errorMessage(error);
        return state.tagHistory;
      })
      .finally(() => {
        state.tagHistoryLoading = false;
        if (tagHistoryPromise === request) tagHistoryPromise = null;
      });
    tagHistoryPromise = request;
    return request;
  }

  function setInterest(value) {
    state.mark.interest = value === "wish" || value === "collect" ? value : "";
    state.status = "";
    state.error = "";
  }

  function updateRating(value) {
    state.mark.rating = String(value || "");
  }

  function updateCategory(value) {
    state.mark.category = String(value || "");
  }

  function updateTags(value) {
    state.mark.tags = String(value || "");
  }

  function applyTagSuggestion(tag) {
    if (state.mark.interest === "wish") {
      state.mark.category = tag;
      return;
    }
    const normalized = normalizeDoubanTags(tag);
    if (!normalized) return;
    const tags = normalizeDoubanTags(state.mark.tags).split(/\s+/).filter(Boolean);
    if (!tags.includes(normalized)) tags.push(normalized);
    state.mark.tags = tags.join(" ");
  }

  function rememberTags(tagsText) {
    const allowed = new Set(
      subscriptionCategories.value
        .map((category) => String(category.wanted_tag || "").trim())
        .filter(Boolean),
    );
    const tags = normalizeDoubanTags(tagsText)
      .split(/\s+/)
      .filter((tag) => allowed.has(tag));
    if (!tags.length) return;
    state.tagHistory = [...tags, ...state.tagHistory.filter((tag) => !tags.includes(tag))];
  }

  async function save() {
    const revision = contextRevision;
    const doubanId = currentDoubanId;
    if (disposed || !doubanId || !state.mark.interest || state.saving) return null;

    state.saving = true;
    state.error = "";
    state.status = "保存中…";
    try {
      const tags =
        state.mark.interest === "wish"
          ? normalizeDoubanTags(state.mark.category)
          : normalizeDoubanTags(state.mark.tags);
      if (state.mark.interest === "wish" && !tags) throw new Error("请选择订阅分类");
      const response = await transport.saveInterest(doubanId, {
        interest: state.mark.interest,
        rating:
          state.mark.interest === "collect" && state.mark.rating
            ? Number(state.mark.rating)
            : undefined,
        tags,
      });
      if (!isCurrentContext(revision, doubanId)) return response;
      state.status = state.mark.interest === "wish" ? "已标记想看" : "已标记看过";
      rememberTags(tags);
      return response;
    } catch (error) {
      if (isCurrentContext(revision, doubanId)) {
        state.error = errorMessage(error);
        state.status = state.error;
      }
      throw error;
    } finally {
      if (isCurrentContext(revision, doubanId)) state.saving = false;
    }
  }

  function dispose() {
    disposed = true;
    reset();
  }

  return Object.freeze({
    state,
    model,
    initialize,
    hydrate,
    loadTagHistory,
    setInterest,
    updateRating,
    updateCategory,
    updateTags,
    applyTagSuggestion,
    save,
    reset,
    dispose,
  });
}
