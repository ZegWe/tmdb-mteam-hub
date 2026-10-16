<template>
  <section
    class="subscription-watcher-banner"
    :data-watcher-mode="banner.key"
    role="status"
    aria-live="polite"
  >
    <span class="watcher-indicator" aria-hidden="true"></span>
    <strong>{{ banner.title }}</strong>
    <span class="watcher-description">{{ banner.message }}</span>
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
