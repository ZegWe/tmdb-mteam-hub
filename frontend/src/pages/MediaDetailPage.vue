<template>
  <section id="page-detail" class="app-page detail-page is-active">
    <header class="top detail-page-top">
      <div>
        <h1>{{ detailPageTitle }}</h1>
        <p class="sub">{{ detailPageSubtitle }}</p>
      </div>
      <div class="actions">
        <button type="button" class="btn btn-secondary" @click="closeDetail">返回</button>
      </div>
    </header>

    <div class="detail-body">
      <div v-if="detailStore.primary.loading" class="detail-loading" role="status">
        <div class="spinner" aria-hidden="true"></div>
        <p>加载详情…</p>
      </div>
      <p v-else-if="detailStore.primary.error" class="empty-hint">
        加载失败：{{ detailStore.primary.error }}
      </p>
      <MediaDetailView
        v-else-if="detailStore.primary.data"
        :model="mediaModel"
        @set-interest="detailStore.setInterest"
        @update-rating="detailStore.updateRating"
        @update-category="detailStore.updateCategory"
        @update-tags="detailStore.updateTags"
        @save-interest="saveDoubanInterest"
        @tag-suggestion="detailStore.applyTagSuggestion"
        @load-season="detailStore.loadSeason"
        @select-torrent-source="detailStore.selectTorrentSource"
        @push-torrent="openQbPushDialog"
      />
    </div>

    <ControlledDialog
      class="modal"
      :open="qbDialogOpen"
      labelledby="qb-push-dialog-title"
      initial-focus="select, input, button"
      :return-focus="qbDialogTrigger"
      @request-close="closeQbPushDialog"
    >
      <form class="modal-box qb-push-dialog" @submit.prevent="submitQbPush">
        <h2 id="qb-push-dialog-title">推送到 qBittorrent</h2>
        <p class="hint subtle">{{ qbDialogLabel }}</p>
        <label
          >qB 服务器<select
            v-model="qbDialog.form.serverIndex"
            class="select select-bordered"
            :disabled="!qbServers.length"
            :title="selectedQbServerLabel"
          >
            <option v-if="!qbServers.length" value="">未配置 qB（请打开 API 设置）</option>
            <option
              v-for="(server, index) in qbServers"
              :key="server.id || index"
              :value="String(index)"
            >
              {{ server.name || server.base_url || `服务器 ${index + 1}` }}
            </option>
          </select></label
        >
        <label
          >分类（可选）<input
            v-model.trim="qbDialog.form.category"
            type="text"
            class="input input-bordered"
            autocomplete="off"
            placeholder="留空则用 qB 默认"
        /></label>
        <label
          >保存路径（可选）<input
            v-model.trim="qbDialog.form.savepath"
            type="text"
            class="input input-bordered"
            autocomplete="off"
            placeholder="留空则用 qB 默认保存目录"
        /></label>
        <div class="form-actions">
          <button type="button" class="btn btn-secondary" @click="closeQbPushDialog">取消</button>
          <button type="submit" class="btn btn-primary" :disabled="qbDialogLoading">
            确认推送
          </button>
        </div>
      </form>
    </ControlledDialog>
  </section>
</template>

<script setup>
import { inject, onBeforeUnmount, shallowRef, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  detailBackRouteLocation,
  firstQueryValue,
  normalizeDetailRoute,
} from "../app/detail-routes.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import MediaDetailView from "../components/MediaDetailView.vue";
import { createMediaDetailStore } from "../features/media-detail/store.js";
import ControlledDialog from "../features/qb/ControlledDialog.vue";
import { createQbPushDialogStore } from "../features/qb/push-dialog-store.js";
import { SEARCH_CONTEXT_KEY } from "../features/search/context.js";
import { SETTINGS_STORE_KEY } from "../features/settings/store.js";

const searchContext = inject(SEARCH_CONTEXT_KEY, null);
const settingsStore = inject(SETTINGS_STORE_KEY, null);
if (!searchContext || !settingsStore) {
  throw new Error("MediaDetailPage requires search and settings contexts");
}

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const route = useRoute();
const router = useRouter();
const detailStore = createMediaDetailStore({
  subscriptionCategories: settingsStore.runtimeSubscriptionCategories,
});
const qbDialog = createQbPushDialogStore({ settingsStore });
const detailPageTitle = detailStore.pageTitle;
const detailPageSubtitle = detailStore.pageSubtitle;
const mediaModel = detailStore.model;
const qbDialogOpen = qbDialog.open;
const qbDialogLoading = qbDialog.loading;
const qbServers = qbDialog.servers;
const qbDialogLabel = qbDialog.label;
const selectedQbServerLabel = qbDialog.selectedServerLabel;
const qbDialogTrigger = shallowRef(null);

watch(
  () => [route.name, route.params.mediaType, route.params.id, route.query.doubanTags],
  () => loadRouteDetail(),
  { immediate: true },
);

onBeforeUnmount(() => {
  qbDialogTrigger.value = null;
  qbDialog.dispose();
  detailStore.dispose();
});

async function loadRouteDetail() {
  const parsed = normalizeDetailRoute(route);
  if (parsed?.kind !== "media") return;
  notifications.clearError();
  try {
    await detailStore.load({
      mediaType: parsed.mediaType,
      id: parsed.id,
      doubanTags: String(firstQueryValue(route.query.doubanTags) || ""),
    });
  } catch (error) {
    notifications.showError(error instanceof Error ? error.message : String(error));
  }
}

function closeDetail() {
  const parsed = normalizeDetailRoute(route);
  if (searchContext.consumeDetailOrigin()) {
    router.back();
    return;
  }
  router.replace(detailBackRouteLocation(parsed)).catch(() => {});
}

async function saveDoubanInterest() {
  try {
    await detailStore.saveInterest();
    notifications.showToast(detailStore.interest.status, "ok");
  } catch (error) {
    notifications.showToast(error instanceof Error ? error.message : String(error), "err");
  }
}

async function openQbPushDialog(torrent, triggerElement) {
  if (qbDialogLoading.value) return;
  qbDialogTrigger.value = triggerElement instanceof HTMLElement ? triggerElement : null;
  try {
    await qbDialog.openForTorrent(torrent);
  } catch (error) {
    qbDialogTrigger.value = null;
    notifications.showToast(error instanceof Error ? error.message : String(error), "err");
  }
}

function closeQbPushDialog() {
  qbDialog.close();
  qbDialogTrigger.value = null;
}

async function submitQbPush() {
  try {
    const result = await qbDialog.submit();
    if (!result) return;
    const label = String(result?.server?.name || "").trim() || result?.server?.base_url || "qB";
    notifications.showToast(`已推送到 ${label}`, "ok");
    qbDialogTrigger.value = null;
  } catch (error) {
    notifications.showToast(error instanceof Error ? error.message : String(error), "err");
  }
}
</script>

<style src="../features/media-detail/styles.css"></style>
