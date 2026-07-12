<template>
  <section id="page-main" class="app-page is-active">
    <header class="top">
      <h1>影视检索</h1>
      <p class="sub">TMDB / 豆瓣资料与 M-Team 种子</p>
    </header>

    <section class="search-bar">
      <div class="source-switch tabs tabs-boxed" role="group" aria-label="搜索来源">
        <button
          type="button"
          class="source-pill tab"
          :class="{ 'tab-active is-active': searchSource === 'tmdb' }"
          :aria-pressed="searchSource === 'tmdb' ? 'true' : 'false'"
          @click="setSearchSource('tmdb')"
        >
          TMDB
        </button>
        <button
          type="button"
          class="source-pill tab"
          :class="{ 'tab-active is-active': searchSource === 'douban' }"
          :aria-pressed="searchSource === 'douban' ? 'true' : 'false'"
          @click="setSearchSource('douban')"
        >
          豆瓣
        </button>
      </div>
      <input
        v-model.trim="query"
        type="search"
        class="input input-bordered"
        :placeholder="searchSource === 'douban' ? '搜索豆瓣影视标题…' : '搜索电影或剧集标题…'"
        autocomplete="off"
        @keydown.enter="runSearch"
      />
      <button type="button" class="btn btn-primary" :disabled="searchLoading" @click="runSearch">
        搜索
      </button>
    </section>

    <main
      class="layout search-layout"
      id="main-layout"
      aria-live="polite"
      :aria-busy="searchLoading ? 'true' : 'false'"
    >
      <div v-if="searchLoading" class="layout-loading" role="status">
        <div class="spinner" aria-hidden="true"></div>
        <span class="loading-text">{{ searchLoadingText }}</span>
      </div>
      <section id="movies-section">
        <h2 id="movies-title">
          {{ searchSource === "douban" ? "豆瓣影视" : "电影" }}
        </h2>
        <div class="media-grid">
          <p v-if="!movies.length" class="empty-hint">无结果</p>
          <article
            v-for="item in movies"
            :key="cardKey(item, 'movie')"
            class="card media-card media-card-search bg-base-100 border border-base-300 shadow-sm"
            role="button"
            tabindex="0"
            :aria-label="`打开详情 ${item.title || '(无标题)'}`"
            @click="openCardDetail(item, 'movie')"
            @keydown.enter.prevent="openCardDetail(item, 'movie')"
            @keydown.space.prevent="openCardDetail(item, 'movie')"
          >
            <img :src="itemImageUrl(item) || transparentPixel" alt="" loading="lazy" />
            <div class="meta">
              <div class="title">{{ item.title || "(无标题)" }}</div>
              <div class="subtle">{{ cardSubtitle(item) }}</div>
            </div>
          </article>
        </div>
        <div v-if="showDoubanSearchPager" class="search-pager" aria-label="豆瓣搜索分页">
          <button
            type="button"
            class="btn btn-secondary"
            :disabled="searchLoading || doubanSearchPage.page <= 1"
            @click="loadDoubanSearchPage(doubanSearchPage.page - 1)"
          >
            上一页
          </button>
          <span class="search-pager-status">{{ doubanSearchPagerText }}</span>
          <button
            type="button"
            class="btn btn-secondary"
            :disabled="searchLoading || !doubanSearchPage.has_more"
            @click="loadDoubanSearchPage(doubanSearchPage.page + 1)"
          >
            下一页
          </button>
        </div>
      </section>
      <section v-show="searchSource !== 'douban'" id="tv-section">
        <h2 id="tv-title">剧集</h2>
        <div class="media-grid">
          <p v-if="!tv.length" class="empty-hint">无结果</p>
          <article
            v-for="item in tv"
            :key="cardKey(item, 'tv')"
            class="card media-card media-card-search bg-base-100 border border-base-300 shadow-sm"
            role="button"
            tabindex="0"
            :aria-label="`打开详情 ${item.title || item.name || '(无标题)'}`"
            @click="openCardDetail(item, 'tv')"
            @keydown.enter.prevent="openCardDetail(item, 'tv')"
            @keydown.space.prevent="openCardDetail(item, 'tv')"
          >
            <img :src="itemImageUrl(item) || transparentPixel" alt="" loading="lazy" />
            <div class="meta">
              <div class="title">{{ item.title || item.name || "(无标题)" }}</div>
              <div class="subtle">{{ cardSubtitle(item) }}</div>
            </div>
          </article>
        </div>
      </section>
    </main>
  </section>
</template>

<script setup>
import { inject, onBeforeUnmount } from "vue";
import { useRoute, useRouter } from "vue-router";
import { detailRouteLocationFromMediaCard } from "../app/detail-routes.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import { SEARCH_CONTEXT_KEY } from "../features/search/context.js";
import { cardKey, cardSubtitle } from "../features/search/domain.js";
import { createSearchRouteSync } from "../features/search/route.js";
import { itemImageUrl } from "../shared/media/images.js";

const transparentPixel =
  "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

const searchContext = inject(SEARCH_CONTEXT_KEY, null);
if (!searchContext) throw new Error("SearchPage requires a search context");

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const route = useRoute();
const router = useRouter();
const searchStore = searchContext.store;
const searchSource = searchStore.source;
const query = searchStore.query;
const movies = searchStore.movies;
const tv = searchStore.tv;
const searchLoading = searchStore.loading;
const searchLoadingText = searchStore.loadingText;
const doubanSearchPage = searchStore.doubanPage;
const showDoubanSearchPager = searchStore.showDoubanPager;
const doubanSearchPagerText = searchStore.doubanPagerText;
const routeSync = createSearchRouteSync({
  route,
  router,
  store: searchStore,
  onError: (error) =>
    notifications.showError(error instanceof Error ? error.message : String(error)),
});

onBeforeUnmount(() => routeSync.dispose());

function setSearchSource(source) {
  notifications.clearError();
  return routeSync.selectSource(source).catch((error) => {
    notifications.showError(error instanceof Error ? error.message : String(error));
    return null;
  });
}

function runSearch() {
  return executeSearch(1);
}

function loadDoubanSearchPage(pageNumber) {
  return executeSearch(pageNumber, true);
}

async function executeSearch(pageNumber, doubanPage = false) {
  notifications.clearError();
  try {
    return await routeSync.submit(doubanPage ? pageNumber : 1);
  } catch (error) {
    notifications.showError(error instanceof Error ? error.message : String(error));
    return null;
  }
}

function openCardDetail(item, fallbackType) {
  const location = detailRouteLocationFromMediaCard(item, fallbackType);
  if (!location) return;
  searchContext.markDetailOpened();
  router.push(location).catch((error) => {
    searchContext.clearDetailOrigin();
    const message = error instanceof Error ? error.message : String(error || "");
    if (/duplicated|redundant|same route/i.test(message)) return;
    notifications.showError(message || "更新详情 URL 失败");
  });
}
</script>

<style src="../features/search/styles.css"></style>
