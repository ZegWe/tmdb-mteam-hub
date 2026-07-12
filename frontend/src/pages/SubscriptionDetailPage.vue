<template>
  <section id="page-subscription-detail" class="app-page detail-page is-active">
    <header class="top detail-page-top">
      <div>
        <h1>{{ selectedSubscription?.title || "订阅详情" }}</h1>
        <p class="sub">订阅状态、下载进度与硬链接结果</p>
      </div>
      <div class="actions">
        <button type="button" class="btn btn-secondary" @click="returnToSubscriptions">返回</button>
      </div>
    </header>

    <div class="detail-body">
      <SubscriptionWatcherBanner class="mb-4" />
      <div v-if="detailLoading && !selectedDetail" class="detail-loading" role="status">
        <div class="spinner" aria-hidden="true"></div>
        <p>加载详情…</p>
      </div>
      <p v-else-if="detailError" class="empty-hint">加载失败：{{ detailError }}</p>
      <SubscriptionDetailView v-else-if="selectedDetail" :selected-subscription="selectedDetail" />
    </div>
  </section>
</template>

<script setup>
import { computed, inject, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { normalizeDetailRoute } from "../app/detail-routes.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import SubscriptionDetailView from "../components/SubscriptionDetailView.vue";
import SubscriptionWatcherBanner from "../features/subscriptions/SubscriptionWatcherBanner.vue";
import { SUBSCRIPTION_CONTEXT_KEY } from "../features/subscriptions/context.js";

const subscriptionContext = inject(SUBSCRIPTION_CONTEXT_KEY, null);
if (!subscriptionContext) {
  throw new Error("SubscriptionDetailPage requires a subscription context");
}

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const route = useRoute();
const router = useRouter();
const subscriptionStore = subscriptionContext.store;
const selectedSubscription = subscriptionStore.selected;
const routeSyncLoading = ref(false);
const detailError = ref("");
let mounted = false;
let routeEntered = false;
let routeStartPromise = null;
let detailRequestRevision = 0;
let activeDetailId = "";

const selectedDetail = computed(() => {
  const record = selectedSubscription.value;
  return record && subscriptionStore.hasFreshDetail(record.subject_id) ? record : null;
});
const detailLoading = computed(
  () =>
    routeSyncLoading.value ||
    (activeDetailId ? subscriptionStore.isDetailLoading(activeDetailId) : false),
);

onMounted(() => {
  mounted = true;
  void syncSubscriptionDetail();
});

onBeforeUnmount(() => {
  mounted = false;
  detailRequestRevision += 1;
  if (activeDetailId) subscriptionStore.cancelDetailLoad(activeDetailId);
  activeDetailId = "";
  subscriptionStore.setSelectedId("");
  leaveSubscriptionRoute();
});

watch(
  () => route.params.id,
  () => {
    if (mounted) void syncSubscriptionDetail();
  },
);

watch(
  () => {
    const parsed = normalizeDetailRoute(route);
    if (!parsed || parsed.kind !== "subscription") return null;
    const record = subscriptionStore.getById(parsed.id);
    return {
      id: parsed.id,
      revision: record?.revision ?? record?.updated_at ?? null,
      fresh: subscriptionStore.hasFreshDetail(parsed.id),
    };
  },
  (current, previous) => {
    if (
      !mounted ||
      !current ||
      current.fresh ||
      routeSyncLoading.value ||
      subscriptionStore.isDetailLoading(current.id)
    ) {
      return;
    }
    if (
      previous &&
      current.id === previous.id &&
      current.revision === previous.revision &&
      current.fresh === previous.fresh
    ) {
      return;
    }
    void syncSubscriptionDetail();
  },
);

function enterSubscriptionRoute() {
  if (routeEntered) return routeStartPromise;
  routeEntered = true;
  try {
    const request = subscriptionContext.enterRoute();
    if (!request || typeof request.then !== "function") return null;
    const tracked = Promise.resolve(request).finally(() => {
      if (routeStartPromise === tracked) routeStartPromise = null;
    });
    routeStartPromise = tracked;
    return tracked;
  } catch (error) {
    leaveSubscriptionRoute();
    throw error;
  }
}

function leaveSubscriptionRoute() {
  if (!routeEntered) return;
  routeEntered = false;
  routeStartPromise = null;
  subscriptionContext.leaveRoute();
}

async function syncSubscriptionDetail() {
  const requestRevision = ++detailRequestRevision;
  notifications.clearError();
  detailError.value = "";
  const parsed = normalizeDetailRoute(route);
  if (!parsed || parsed.kind !== "subscription") {
    if (activeDetailId) subscriptionStore.cancelDetailLoad(activeDetailId);
    activeDetailId = "";
    subscriptionStore.setSelectedId("");
    leaveSubscriptionRoute();
    routeSyncLoading.value = false;
    detailError.value = "订阅 ID 无效";
    return;
  }

  const id = parsed.id;
  if (activeDetailId && activeDetailId !== id) {
    subscriptionStore.cancelDetailLoad(activeDetailId);
  }
  activeDetailId = id;
  routeSyncLoading.value = true;
  try {
    subscriptionStore.setSelectedId(id);
    const startRequest = enterSubscriptionRoute();
    if (startRequest) await startRequest;
    if (!mounted || requestRevision !== detailRequestRevision) return;
    if (!subscriptionStore.hasFreshDetail(id)) await subscriptionStore.loadDetail(id);
    if (!mounted || requestRevision !== detailRequestRevision) return;
    if (!subscriptionStore.getById(id) || !subscriptionStore.hasFreshDetail(id)) {
      detailError.value = `未找到订阅记录：${id}`;
    }
  } catch (error) {
    if (!mounted || requestRevision !== detailRequestRevision) return;
    const message = isMissingSubscription(error)
      ? `未找到订阅记录：${id}`
      : error instanceof Error
        ? error.message
        : String(error);
    detailError.value = message;
    if (!isRequestAbort(error)) notifications.showError(message);
  } finally {
    if (mounted && requestRevision === detailRequestRevision) routeSyncLoading.value = false;
  }
}

function isMissingSubscription(error) {
  return error?.status === 404 || error?.code === "subscription_not_found";
}

function isRequestAbort(error) {
  return error?.name === "AbortError" || error?.code === "request_aborted";
}

function returnToSubscriptions() {
  router.push({ name: "subscriptions" }).catch(() => {});
}
</script>

<style src="../features/subscriptions/styles.css"></style>
