<template>
  <section id="page-subscriptions" class="app-page is-active">
    <header class="top subscriptions-top">
      <h1>订阅</h1>
      <p class="sub">想看订阅、下载进度与硬链接结果</p>
      <div class="actions">
        <button
          type="button"
          class="btn btn-secondary"
          :disabled="subscriptionsLoading"
          title="从豆瓣获取最新想看订阅"
          @click="refreshSubscriptions"
        >
          刷新
        </button>
      </div>
    </header>
    <section class="subscription-toolbar" aria-live="polite">
      <SubscriptionWatcherBanner />
      <div class="subscription-stats">
        <div>
          <span>全部订阅</span><strong>{{ subscriptionRecords.length }}</strong>
        </div>
        <div>
          <span>正在下载</span><strong>{{ downloadingCount }}</strong>
        </div>
        <div>
          <span>已完成</span><strong>{{ completedCount }}</strong>
        </div>
      </div>
      <p class="hint">{{ subscriptionSummary }}</p>
      <div class="subscription-filters" role="group" aria-label="订阅状态筛选">
        <button
          v-for="filter in filters"
          :key="filter.key"
          type="button"
          :class="{ 'is-active': selectedFilter === filter.key }"
          :aria-pressed="selectedFilter === filter.key"
          @click="selectedFilter = filter.key"
        >
          {{ filter.label }}
        </button>
      </div>
    </section>
    <section id="subscription-list" class="subscription-list" aria-live="polite">
      <p v-if="!filteredRecords.length" class="empty-hint">
        {{
          subscriptionsLoading
            ? "正在加载订阅…"
            : subscriptionRecords.length
              ? "该状态下暂无订阅"
              : "暂无订阅记录"
        }}
      </p>
      <SubscriptionCard
        v-for="record in filteredRecords"
        :key="record.subject_id"
        :record="record"
        @open="openSubscriptionDetail"
      />
    </section>
  </section>
</template>

<script setup>
import { computed, inject, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { detailRouteLocationFromSubscriptionRecord } from "../app/detail-routes.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import SubscriptionCard from "../features/subscriptions/SubscriptionCard.vue";
import SubscriptionWatcherBanner from "../features/subscriptions/SubscriptionWatcherBanner.vue";
import { SUBSCRIPTION_CONTEXT_KEY } from "../features/subscriptions/context.js";
import {
  subscriptionDisplayStatus,
  subscriptionPollToast,
} from "../features/subscriptions/domain.js";

const subscriptionContext = inject(SUBSCRIPTION_CONTEXT_KEY, null);
if (!subscriptionContext) throw new Error("SubscriptionsPage requires a subscription context");

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const router = useRouter();
const subscriptionStore = subscriptionContext.store;
const subscriptionRecords = subscriptionStore.records;
const subscriptionSummary = subscriptionStore.summary;
const subscriptionsLoading = subscriptionStore.loading;
const selectedFilter = ref("all");
const filters = [
  { key: "all", label: "全部" },
  { key: "downloading", label: "正在下载" },
  { key: "pending", label: "处理中" },
  { key: "attention", label: "需关注" },
  { key: "completed", label: "已完成" },
];
function matchesFilter(record, key) {
  const status = subscriptionDisplayStatus(record).key;
  if (key === "all") return true;
  if (key === "pending") return ["queued", "meta", "searching", "linking"].includes(status);
  if (key === "attention")
    return ["failed", "skipped", "retry_blocked", "needs_confirmation", "waiting_release"].includes(
      status,
    );
  return status === key;
}
const filteredRecords = computed(() =>
  subscriptionRecords.value.filter((record) => matchesFilter(record, selectedFilter.value)),
);
const downloadingCount = computed(
  () => subscriptionRecords.value.filter((record) => matchesFilter(record, "downloading")).length,
);
const completedCount = computed(
  () => subscriptionRecords.value.filter((record) => matchesFilter(record, "completed")).length,
);

onMounted(() => {
  notifications.clearError();
  subscriptionContext.enterRoute().catch(() => {});
});

onBeforeUnmount(() => {
  subscriptionContext.leaveRoute();
});

async function refreshSubscriptions() {
  notifications.clearError();
  try {
    const result = await subscriptionStore.poll();
    notifications.showToast(subscriptionPollToast(result.outcome), "ok");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const detail = `刷新豆瓣订阅失败：${message}`;
    notifications.showError(detail);
    notifications.showToast(detail, "err");
  }
}

function openSubscriptionDetail(record) {
  const location = detailRouteLocationFromSubscriptionRecord(record);
  if (!location) return;
  router.push(location).catch((error) => {
    const message = error instanceof Error ? error.message : String(error || "");
    if (!/duplicated|redundant|same route/i.test(message)) notifications.showError(message);
  });
}
</script>

<style src="../features/subscriptions/styles.css"></style>
