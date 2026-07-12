<template>
  <div class="app-shell">
    <aside class="side-nav menu bg-base-100 text-base-content" aria-label="主导航">
      <div class="brand">
        <h1>影视检索</h1>
        <p class="sub">TMDB / 豆瓣 / M-Team</p>
      </div>
      <nav class="nav-list" aria-label="页面">
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'main' }"
          :aria-current="navPage === 'main' ? 'page' : undefined"
          @click="go('main')"
        >
          主功能
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'subscriptions' }"
          :aria-current="navPage === 'subscriptions' ? 'page' : undefined"
          @click="go('subscriptions')"
        >
          订阅
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'logs' }"
          :aria-current="navPage === 'logs' ? 'page' : undefined"
          @click="go('logs')"
        >
          日志
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'settings' }"
          :aria-current="navPage === 'settings' ? 'page' : undefined"
          @click="go('settings')"
        >
          设置
        </button>
      </nav>
      <button
        type="button"
        class="theme-toggle btn btn-ghost btn-square"
        :aria-label="themeToggleLabel"
        :title="themeToggleLabel"
        @click="cycleThemeMode"
      >
        <svg
          v-if="themeMode === 'system'"
          viewBox="0 0 24 24"
          aria-hidden="true"
          class="theme-toggle-icon"
        >
          <path d="M4 5.5h16v10H4z" />
          <path d="M9 19h6" />
          <path d="M12 15.5V19" />
        </svg>
        <svg
          v-else-if="themeMode === 'dark'"
          viewBox="0 0 24 24"
          aria-hidden="true"
          class="theme-toggle-icon"
        >
          <path d="M20 14.5A7.5 7.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z" />
        </svg>
        <svg v-else viewBox="0 0 24 24" aria-hidden="true" class="theme-toggle-icon">
          <path d="M12 4V2" />
          <path d="M12 22v-2" />
          <path d="M4.93 4.93 3.51 3.51" />
          <path d="m20.49 20.49-1.42-1.42" />
          <path d="M4 12H2" />
          <path d="M22 12h-2" />
          <path d="m4.93 19.07-1.42 1.42" />
          <path d="m20.49 3.51-1.42 1.42" />
          <circle cx="12" cy="12" r="4" />
        </svg>
      </button>
    </aside>

    <div class="app-content">
      <div v-if="error" id="err" class="banner err alert alert-error" role="alert">{{ error }}</div>
      <div v-if="toast.message" id="toast" class="app-toast" role="status" aria-live="polite">
        <div class="app-toast-message" :class="toast.kind === 'err' ? 'toast-err' : 'toast-ok'">
          {{ toast.message }}
        </div>
      </div>
      <RouterView />
    </div>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, provide, reactive, ref, watch } from "vue";
import { RouterView, useRoute, useRouter } from "vue-router";
import { APP_NOTIFICATIONS_KEY } from "./notifications.js";
import { createSearchContext, SEARCH_CONTEXT_KEY } from "../features/search/context.js";
import {
  createSubscriptionContext,
  SUBSCRIPTION_CONTEXT_KEY,
} from "../features/subscriptions/context.js";
import { createSettingsStore, SETTINGS_STORE_KEY } from "../features/settings/store.js";
import {
  nextThemeMode,
  normalizeThemeMode,
  resolveThemeScheme,
  THEME_STORAGE_KEY,
  themeModeLabel,
} from "../shared/theme/theme-mode.js";

function readStoredThemeMode() {
  if (typeof window === "undefined") return "system";
  try {
    return normalizeThemeMode(window.localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return "system";
  }
}

function storeThemeMode(mode) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, normalizeThemeMode(mode));
  } catch {}
}

function readSystemPrefersDark() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyThemeScheme(scheme) {
  if (typeof document === "undefined") return;
  const normalized = scheme === "dark" ? "dark" : "light";
  document.documentElement.dataset.colorScheme = normalized;
  document.documentElement.style.colorScheme = normalized;
  if (document.body) {
    document.body.dataset.theme = normalized === "dark" ? "mediahub-dark" : "mediahub";
  }
}

const route = useRoute();
const router = useRouter();
const navPage = computed(() => String(route.meta.navPage || ""));
const error = ref("");
const toast = reactive({ message: "", kind: "ok", timer: 0 });
const themeMode = ref(readStoredThemeMode());
const systemPrefersDark = ref(readSystemPrefersDark());
const resolvedThemeScheme = computed(() =>
  resolveThemeScheme(themeMode.value, systemPrefersDark.value),
);
const themeToggleLabel = computed(() => themeModeLabel(themeMode.value));
let themePreferenceCleanup = null;
applyThemeScheme(resolvedThemeScheme.value);

const settingsStore = createSettingsStore();
const subscriptionContext = createSubscriptionContext();
const searchContext = createSearchContext();

provide(SETTINGS_STORE_KEY, settingsStore);
provide(SUBSCRIPTION_CONTEXT_KEY, subscriptionContext);
provide(SEARCH_CONTEXT_KEY, searchContext);
provide(
  APP_NOTIFICATIONS_KEY,
  Object.freeze({
    clearError,
    showError,
    showToast,
  }),
);

watch(
  () => route.name,
  () => clearError(),
);

watch(resolvedThemeScheme, (scheme) => {
  applyThemeScheme(scheme);
});

watch(themeMode, (mode) => {
  storeThemeMode(mode);
});

onMounted(() => {
  settingsStore.ensureRuntimeLoaded().catch(() => {});
  themePreferenceCleanup = watchSystemThemePreference((prefersDark) => {
    systemPrefersDark.value = prefersDark;
  });
});

onBeforeUnmount(() => {
  searchContext.dispose();
  subscriptionContext.dispose();
  settingsStore.dispose();
  clearTimeout(toast.timer);
  if (themePreferenceCleanup) {
    themePreferenceCleanup();
    themePreferenceCleanup = null;
  }
});

function watchSystemThemePreference(onChange) {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  onChange(media.matches);
  const listener = (event) => onChange(event.matches);
  if (typeof media.addEventListener === "function") {
    media.addEventListener("change", listener);
    return () => media.removeEventListener("change", listener);
  }
  if (typeof media.addListener === "function") {
    media.addListener(listener);
    return () => media.removeListener(listener);
  }
  return null;
}

function cycleThemeMode() {
  themeMode.value = nextThemeMode(themeMode.value);
}

function go(target) {
  router.push({ name: target });
}

function showToast(message, kind = "ok") {
  toast.message = message;
  toast.kind = kind;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => {
    toast.message = "";
  }, 3800);
}

function showError(message) {
  error.value = message;
}

function clearError() {
  error.value = "";
}
</script>
