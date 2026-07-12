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
      <SubscriptionWatcherBanner class="mb-3" />
      <p class="hint">{{ subscriptionSummary }}</p>
    </section>
    <section id="subscription-list" class="subscription-list" aria-live="polite">
      <p v-if="!subscriptionRecords.length" class="empty-hint">暂无订阅记录</p>
      <SubscriptionCard
        v-for="record in subscriptionRecords"
        :key="record.subject_id"
        :record="record"
        @open="openSubscriptionDetail"
      />
    </section>
  </section>
</template>

<script setup>
import { inject, onBeforeUnmount, onMounted } from "vue";
import { useRouter } from "vue-router";
import { detailRouteLocationFromSubscriptionRecord } from "../app/detail-routes.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import SubscriptionCard from "../features/subscriptions/SubscriptionCard.vue";
import SubscriptionWatcherBanner from "../features/subscriptions/SubscriptionWatcherBanner.vue";
import { SUBSCRIPTION_CONTEXT_KEY } from "../features/subscriptions/context.js";
import { subscriptionPollToast } from "../features/subscriptions/domain.js";

const subscriptionContext = inject(SUBSCRIPTION_CONTEXT_KEY, null);
if (!subscriptionContext) throw new Error("SubscriptionsPage requires a subscription context");

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const router = useRouter();
const subscriptionStore = subscriptionContext.store;
const subscriptionRecords = subscriptionStore.records;
const subscriptionSummary = subscriptionStore.summary;
const subscriptionsLoading = subscriptionStore.loading;

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
