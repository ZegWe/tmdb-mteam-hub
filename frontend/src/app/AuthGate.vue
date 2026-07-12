<template>
  <main v-if="checkingStatus" class="auth-gate" aria-live="polite">
    <section class="auth-card card bg-base-100 border border-base-300" role="status">
      <div class="spinner" aria-hidden="true"></div>
      <p>正在检查登录状态…</p>
    </section>
  </main>

  <main v-else-if="!authenticated" class="auth-gate">
    <section class="auth-card card bg-base-100 border border-base-300">
      <header>
        <p class="auth-eyebrow">TMDB M-Team Hub</p>
        <h1>管理登录</h1>
        <p>请输入服务端配置的管理 Token。登录成功后由浏览器管理 HttpOnly 会话 Cookie。</p>
      </header>

      <form class="auth-form" @submit.prevent="login">
        <label for="management-token">管理 Token</label>
        <input
          id="management-token"
          v-model="token"
          name="management-token"
          type="password"
          class="input input-bordered"
          autocomplete="off"
          spellcheck="false"
          autofocus
          @input="error = ''"
        />
        <p v-if="error" class="auth-error" role="alert">{{ error }}</p>
        <button type="submit" class="btn btn-primary" :disabled="loginLoading || !token.trim()">
          {{ loginLoading ? "登录中…" : "登录" }}
        </button>
        <button
          v-if="statusCheckFailed"
          type="button"
          class="btn btn-ghost"
          :disabled="loginLoading"
          @click="checkStatus"
        >
          重新检查状态
        </button>
      </form>
    </section>
  </main>

  <div v-else class="auth-session">
    <p v-if="authStatus?.bootstrap_allowed" class="auth-bootstrap-warning" role="alert">
      当前处于仅限 loopback 的本地 Bootstrap 模式。请立即在设置页创建管理 Token；不要通过代理或 LAN
      共享此模式。
    </p>
    <App />
    <p v-if="error" class="auth-session-error" role="alert">{{ error }}</p>
    <button
      v-if="canLogout"
      type="button"
      class="auth-logout btn btn-ghost"
      :disabled="logoutLoading"
      @click="logout"
    >
      {{ logoutLoading ? "退出中…" : "退出登录" }}
    </button>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import App from "../App.vue";
import {
  getAuthStatus,
  loginAuthSession,
  logoutAuthSession,
} from "../shared/api/endpoints/auth.js";
import { AUTH_SESSION_CHANGED_EVENT } from "../shared/api/auth-session.js";

const authStatus = ref(null);
const checkingStatus = ref(true);
const statusCheckFailed = ref(false);
const loginLoading = ref(false);
const logoutLoading = ref(false);
const token = ref("");
const error = ref("");
const lifecycleController = new AbortController();

const authenticated = computed(() => authStatus.value?.authenticated === true);
const canLogout = computed(
  () => authenticated.value && authStatus.value?.token_configured === true,
);

onMounted(() => {
  globalThis.addEventListener?.(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
  checkStatus();
});

onBeforeUnmount(() => {
  globalThis.removeEventListener?.(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
  lifecycleController.abort(new DOMException("Auth gate unmounted", "AbortError"));
  token.value = "";
});

function normalizeStatus(value) {
  return {
    authenticated: value?.authenticated === true,
    token_configured: value?.token_configured === true,
    bootstrap_allowed: value?.bootstrap_allowed === true,
  };
}

function handleAuthSessionChanged(event) {
  authStatus.value = normalizeStatus(event?.detail);
  checkingStatus.value = false;
  statusCheckFailed.value = false;
  error.value = "";
}

async function checkStatus() {
  checkingStatus.value = true;
  statusCheckFailed.value = false;
  error.value = "";
  try {
    authStatus.value = normalizeStatus(await getAuthStatus({ signal: lifecycleController.signal }));
  } catch {
    if (lifecycleController.signal.aborted) return;
    authStatus.value = normalizeStatus(null);
    statusCheckFailed.value = true;
    error.value = "无法检查登录状态，请确认服务正在运行后重试";
  } finally {
    if (!lifecycleController.signal.aborted) checkingStatus.value = false;
  }
}

async function login() {
  const submittedToken = token.value.trim();
  if (!submittedToken || loginLoading.value) return;
  token.value = "";
  error.value = "";
  loginLoading.value = true;
  try {
    const nextStatus = normalizeStatus(
      await loginAuthSession(submittedToken, { signal: lifecycleController.signal }),
    );
    if (!nextStatus.authenticated) throw new Error("authentication rejected");
    authStatus.value = nextStatus;
    statusCheckFailed.value = false;
  } catch {
    if (lifecycleController.signal.aborted) return;
    authStatus.value = {
      ...(authStatus.value || normalizeStatus(null)),
      authenticated: false,
    };
    error.value = "登录失败，请检查管理 Token 后重试";
  } finally {
    loginLoading.value = false;
  }
}

async function logout() {
  if (logoutLoading.value) return;
  error.value = "";
  logoutLoading.value = true;
  try {
    authStatus.value = normalizeStatus(
      await logoutAuthSession({ signal: lifecycleController.signal }),
    );
  } catch {
    if (lifecycleController.signal.aborted) return;
    error.value = "退出登录失败，请稍后重试";
  } finally {
    logoutLoading.value = false;
  }
}
</script>

<style scoped>
.auth-gate {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1.25rem;
  background:
    radial-gradient(
      circle at top,
      color-mix(in srgb, var(--accent-bg) 52%, transparent),
      transparent 38%
    ),
    var(--background);
}

.auth-card {
  width: min(28rem, 100%);
  display: grid;
  gap: 1.25rem;
  padding: 1.5rem;
  box-shadow: var(--shadow-md);
}

.auth-card[role="status"] {
  justify-items: center;
  color: var(--muted);
}

.auth-card header,
.auth-card header p {
  margin: 0;
}

.auth-card h1 {
  margin: 0.2rem 0 0.45rem;
  font-size: 1.55rem;
}

.auth-card header > p:last-child {
  color: var(--muted);
  line-height: 1.55;
}

.auth-eyebrow {
  color: var(--muted);
  font-size: 0.75rem;
  font-weight: 650;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.auth-form {
  display: grid;
  gap: 0.75rem;
}

.auth-form label {
  color: var(--muted);
  font-size: 0.82rem;
  font-weight: 600;
}

.auth-error,
.auth-session-error {
  margin: 0;
  color: var(--danger);
  font-size: 0.84rem;
  line-height: 1.45;
}

.auth-bootstrap-warning {
  position: fixed;
  top: 0.8rem;
  left: 50%;
  z-index: 45;
  width: min(44rem, calc(100vw - 8rem));
  margin: 0;
  padding: 0.55rem 0.8rem;
  border: 1px solid color-mix(in srgb, var(--warning) 55%, var(--border));
  border-radius: var(--radius);
  color: var(--foreground);
  background: color-mix(in srgb, var(--warning) 12%, var(--surface));
  box-shadow: var(--shadow-md);
  font-size: 0.82rem;
  line-height: 1.45;
  transform: translateX(-50%);
}

.auth-logout {
  position: fixed;
  top: 0.8rem;
  right: 1rem;
  z-index: 45;
  min-height: 2rem;
  padding: 0.35rem 0.65rem;
  background: color-mix(in srgb, var(--surface) 92%, transparent);
  backdrop-filter: blur(10px);
}

.auth-session-error {
  position: fixed;
  top: 3.5rem;
  right: 1rem;
  z-index: 45;
  max-width: min(24rem, calc(100vw - 2rem));
  padding: 0.65rem 0.8rem;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, var(--border));
  border-radius: var(--radius);
  background: var(--surface);
  box-shadow: var(--shadow-md);
}

@media (max-width: 640px) {
  .auth-card {
    padding: 1.1rem;
  }

  .auth-logout {
    top: 0.55rem;
    right: 0.65rem;
  }

  .auth-bootstrap-warning {
    top: 3.25rem;
    width: calc(100vw - 1.3rem);
  }
}
</style>
