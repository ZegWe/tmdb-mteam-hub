<template>
  <section
    class="subscription-watcher-banner alert items-start shadow-sm"
    :class="{
      'alert-info': banner.tone === 'info',
      'alert-warning': banner.tone === 'warning',
      'alert-error': banner.tone === 'danger',
    }"
    :data-watcher-mode="banner.key"
    role="status"
    aria-live="polite"
  >
    <div class="min-w-0">
      <strong class="block">{{ banner.title }}</strong>
      <p class="m-0 text-sm leading-relaxed">{{ banner.message }}</p>
    </div>
  </section>
</template>

<script setup>
import { computed, inject } from "vue";
import { SETTINGS_STORE_KEY } from "../settings/store.js";
import { subscriptionWatcherRuntimeBanner } from "../settings/form-model.js";

const settingsStore = inject(SETTINGS_STORE_KEY, null);
if (!settingsStore) {
  throw new Error("SubscriptionWatcherBanner requires a provided settings store");
}

const banner = computed(() =>
  subscriptionWatcherRuntimeBanner({
    runtimeLoaded: settingsStore.runtimeLoaded.value,
    watcher: settingsStore.runtimeSubscriptionWatcher.value,
  }),
);
</script>
