import { reactive, readonly, ref } from "vue";
import { getAuthStatus, loginAuthSession } from "../../shared/api/endpoints/auth.js";
import { notifyAuthSessionChanged } from "../../shared/api/auth-session.js";
import {
  getSettings,
  pollDoubanQrSession,
  startDoubanQrSession,
  testQbServer as requestQbServerTest,
  updateSettings,
} from "./api.js";
import {
  createQbServerDraft,
  createSecretClearState,
  createSecretPresence,
  createSettingsForm,
  createSubscriptionCategoryDraft,
  createTorrentRuleDraft,
  isSavedQbServerDraft,
  qbServerDtos,
  qbTestPayload,
  requiresAutomationEnableConfirmation,
  settingsFormFromSnapshot,
  settingsMetadataFromSnapshot,
  settingsUpdatePayload,
  subscriptionCategoryDtos,
  subscriptionWatcherDto,
} from "./form-model.js";

export const SETTINGS_STORE_KEY = Symbol("settings-store");
const SETTINGS_QR_POLL_INTERVAL_MS = 2000;

const NOOP_NOTIFY = () => {};

const defaultTransport = Object.freeze({
  load: (options) => getSettings(options),
  save: (payload, options) => updateSettings(payload, options),
  testQb: (payload, options) => requestQbServerTest(payload, options),
  startQr: (options) => startDoubanQrSession(options),
  pollQr: (sessionId, options) => pollDoubanQrSession(sessionId, options),
  authStatus: (options) => getAuthStatus(options),
  login: (token, options) => loginAuthSession(token, options),
});

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

export function createSettingsStore({
  transport = defaultTransport,
  qrPollIntervalMs = SETTINGS_QR_POLL_INTERVAL_MS,
  setIntervalFn = (...args) => globalThis.setInterval(...args),
  clearIntervalFn = (...args) => globalThis.clearInterval(...args),
  now = () => Date.now(),
} = {}) {
  const form = reactive(createSettingsForm());
  const revision = ref(0);
  const secretPresence = reactive(createSecretPresence());
  const clearSecrets = reactive(createSecretClearState());
  const savedQbServerFingerprints = ref({});
  const pageLoading = ref(false);
  const pageLoaded = ref(false);
  const saving = ref(false);
  const saveStatus = reactive({ message: "", kind: "" });

  const runtimeQbServers = ref([]);
  const runtimeSubscriptionCategories = ref([]);
  const runtimeSubscriptionWatcher = ref(subscriptionWatcherDto());
  const runtimeLoaded = ref(false);

  const qrLoading = ref(false);
  const qrStatus = ref("");
  const qrImage = ref("");
  const qrSessionId = ref("");

  let snapshotPromise = null;
  let pageSession = 0;
  let qrTimer = 0;
  let qrNotify = NOOP_NOTIFY;

  function requestSnapshot() {
    if (snapshotPromise) return snapshotPromise;
    const request = Promise.resolve().then(() => transport.load());
    const wrapped = request.finally(() => {
      if (snapshotPromise === wrapped) snapshotPromise = null;
    });
    snapshotPromise = wrapped;
    return wrapped;
  }

  function applyRuntimeSnapshot(snapshot = {}) {
    runtimeQbServers.value = qbServerDtos(snapshot.qb_servers);
    runtimeSubscriptionCategories.value = subscriptionCategoryDtos(
      snapshot.subscription_categories,
    );
    runtimeSubscriptionWatcher.value = subscriptionWatcherDto(snapshot.subscription_watcher);
    runtimeLoaded.value = true;
  }

  function resetDraft() {
    Object.assign(form, createSettingsForm());
    revision.value = 0;
    Object.assign(secretPresence, createSecretPresence());
    Object.assign(clearSecrets, createSecretClearState());
    savedQbServerFingerprints.value = {};
    saveStatus.message = "";
    saveStatus.kind = "";
  }

  function applyDraftSnapshot(snapshot = {}) {
    const model = settingsFormFromSnapshot(snapshot);
    revision.value = model.revision;
    Object.assign(secretPresence, model.secretPresence);
    Object.assign(clearSecrets, model.clearSecrets);
    Object.assign(form, model.form);
    savedQbServerFingerprints.value = model.savedQbServerFingerprints;
  }

  async function ensureRuntimeLoaded() {
    if (runtimeLoaded.value) return;
    const snapshot = await requestSnapshot();
    applyRuntimeSnapshot(snapshot);
  }

  async function enterPage() {
    const session = ++pageSession;
    stopQrSession();
    pageLoading.value = true;
    pageLoaded.value = false;
    saveStatus.message = "";
    saveStatus.kind = "";
    try {
      const snapshot = await requestSnapshot();
      applyRuntimeSnapshot(snapshot);
      if (session !== pageSession) return snapshot;
      applyDraftSnapshot(snapshot);
      pageLoaded.value = true;
      return snapshot;
    } finally {
      if (session === pageSession) pageLoading.value = false;
    }
  }

  function leavePage() {
    pageSession += 1;
    stopQrSession();
    pageLoading.value = false;
    pageLoaded.value = false;
    resetDraft();
  }

  function onSecretInput(field) {
    if (field in clearSecrets) clearSecrets[field] = false;
  }

  function onSecretClearChange(field) {
    if (field in clearSecrets && clearSecrets[field]) form[field] = "";
  }

  function onQbPasswordInput(server) {
    server.clear_password = false;
  }

  function onQbPasswordClearChange(server) {
    if (server.clear_password) server.password = "";
  }

  function addSubscriptionCategory() {
    form.subscription_categories.push(createSubscriptionCategoryDraft(form.qb_servers));
  }

  function removeSubscriptionCategory(index) {
    form.subscription_categories.splice(index, 1);
  }

  function addTorrentRule() {
    form.torrent_match_rules.push(createTorrentRuleDraft());
  }

  function removeTorrentRule(index) {
    form.torrent_match_rules.splice(index, 1);
  }

  function addQbServer() {
    form.qb_servers.push(createQbServerDraft());
  }

  function removeQbServer(index) {
    form.qb_servers.splice(index, 1);
  }

  function isQbServerSaved(server) {
    return isSavedQbServerDraft(server, savedQbServerFingerprints.value);
  }

  async function testQbServer(server) {
    if (!isQbServerSaved(server)) {
      server.testMessage = "请先保存后测试";
      server.testKind = "err";
      return null;
    }
    server.testing = true;
    server.testMessage = "正在测试…";
    server.testKind = "";
    try {
      const data = await transport.testQb(qbTestPayload(server));
      server.testMessage = `可连通${data.version ? ` · ${data.version}` : ""}`;
      server.testKind = "ok";
      return data;
    } catch (error) {
      server.testMessage = errorMessage(error);
      server.testKind = "err";
      return null;
    } finally {
      server.testing = false;
    }
  }

  async function save({ confirmEnableAutomation } = {}) {
    if (saving.value || !pageLoaded.value) return null;
    const session = pageSession;
    const requiresAutomationConfirmation = requiresAutomationEnableConfirmation(
      runtimeSubscriptionWatcher.value,
      form.subscription_watcher,
    );
    let automationConfirmed = false;
    if (requiresAutomationConfirmation) {
      automationConfirmed =
        typeof confirmEnableAutomation === "function" &&
        (await confirmEnableAutomation({ dryRun: form.subscription_watcher.dry_run })) === true;
      if (!automationConfirmed || session !== pageSession || !pageLoaded.value) {
        if (session === pageSession && pageLoaded.value) {
          saveStatus.message = "已取消启用订阅自动化，未保存任何设置";
          saveStatus.kind = "";
        }
        return null;
      }
    }
    saving.value = true;
    saveStatus.message = "正在保存设置…";
    saveStatus.kind = "pending";
    const payload = settingsUpdatePayload({
      form,
      expectedRevision: revision.value,
      clearSecrets,
      subscriptionWatcher: runtimeSubscriptionWatcher.value,
      confirmEnableAutomation: automationConfirmed,
    });
    const replacementAdminToken = payload.admin_token || "";
    const clearsAdminToken = payload.clear_admin_token === true;
    try {
      const snapshot = await transport.save(payload);
      applyRuntimeSnapshot(snapshot);
      if (session === pageSession) {
        applyDraftSnapshot(snapshot);
        pageLoaded.value = true;
        saveStatus.message = snapshot.restart_required
          ? "设置已保存；部分网络配置需重启生效"
          : "设置已保存";
        saveStatus.kind = "ok";
      }
      if (replacementAdminToken) {
        try {
          const status = await transport.login(replacementAdminToken);
          notifyAuthSessionChanged(status);
        } catch {
          notifyAuthSessionChanged({
            authenticated: false,
            token_configured: snapshot?.has_admin_token === true,
          });
          if (session === pageSession) {
            saveStatus.message = "管理 Token 已更新，但浏览器重新登录失败；请使用新 Token 登录";
            saveStatus.kind = "err";
          }
        }
      } else if (clearsAdminToken) {
        try {
          const status = await transport.authStatus();
          notifyAuthSessionChanged(status);
        } catch {
          notifyAuthSessionChanged({ authenticated: false });
        }
      }
      return snapshot;
    } catch (error) {
      if (session === pageSession) {
        saveStatus.message = `保存失败：${errorMessage(error)}`;
        saveStatus.kind = "err";
      }
      throw error;
    } finally {
      saving.value = false;
    }
  }

  function clearQrTimer() {
    if (!qrTimer) return;
    clearIntervalFn(qrTimer);
    qrTimer = 0;
  }

  function stopQrSession() {
    clearQrTimer();
    qrSessionId.value = "";
    qrImage.value = "";
    qrStatus.value = "";
    qrLoading.value = false;
    qrNotify = NOOP_NOTIFY;
  }

  function applyQrMetadata(snapshot = {}) {
    const metadata = settingsMetadataFromSnapshot(snapshot);
    revision.value = metadata.revision;
    Object.assign(secretPresence, metadata.secretPresence);
    form.douban_cookie = "";
    clearSecrets.douban_cookie = false;
    secretPresence.douban_cookie = true;
    pageLoaded.value = true;
  }

  async function pollQrLogin() {
    if (!qrSessionId.value) return null;
    const data = await transport.pollQr(qrSessionId.value);
    qrStatus.value = data.description || data.message || data.login_status || "等待扫码…";
    if (!data.done) return data;

    clearQrTimer();
    if (!data.cookie_saved) {
      qrStatus.value = "登录完成，但服务端未保存 Cookie";
      qrNotify(qrStatus.value, "err");
      return data;
    }

    try {
      const snapshot = await transport.load();
      applyQrMetadata(snapshot);
      qrStatus.value = "Cookie 已安全保存";
      qrNotify("豆瓣 Cookie 已保存", "ok");
    } catch {
      pageLoaded.value = false;
      qrStatus.value = "Cookie 已保存，但配置 revision 刷新失败；请重新打开设置页";
      qrNotify(qrStatus.value, "err");
    }
    return data;
  }

  async function startQrLogin({ notify = NOOP_NOTIFY } = {}) {
    stopQrSession();
    qrNotify = notify;
    qrLoading.value = true;
    qrStatus.value = "正在生成二维码…";
    try {
      const data = await transport.startQr();
      if (!data.session_id || !data.image_url) {
        throw new Error("豆瓣 QR 登录响应缺少会话信息");
      }
      qrSessionId.value = data.session_id;
      qrImage.value = `${data.image_url}&t=${now()}`;
      qrStatus.value = "等待扫码确认…";
      qrTimer = setIntervalFn(() => pollQrLogin().catch(() => {}), qrPollIntervalMs);
      await pollQrLogin();
      return data;
    } catch (error) {
      const message = errorMessage(error);
      qrStatus.value = message;
      qrNotify(message, "err");
      return null;
    } finally {
      qrLoading.value = false;
    }
  }

  function dispose() {
    leavePage();
    runtimeQbServers.value = [];
    runtimeSubscriptionCategories.value = [];
    runtimeSubscriptionWatcher.value = subscriptionWatcherDto();
    runtimeLoaded.value = false;
  }

  return Object.freeze({
    form,
    revision: readonly(revision),
    secretPresence,
    clearSecrets,
    pageLoading: readonly(pageLoading),
    pageLoaded: readonly(pageLoaded),
    saving: readonly(saving),
    saveStatus,
    runtimeQbServers: readonly(runtimeQbServers),
    runtimeSubscriptionCategories: readonly(runtimeSubscriptionCategories),
    runtimeSubscriptionWatcher: readonly(runtimeSubscriptionWatcher),
    runtimeLoaded: readonly(runtimeLoaded),
    qrLoading: readonly(qrLoading),
    qrStatus: readonly(qrStatus),
    qrImage: readonly(qrImage),
    enterPage,
    leavePage,
    ensureRuntimeLoaded,
    onSecretInput,
    onSecretClearChange,
    onQbPasswordInput,
    onQbPasswordClearChange,
    addSubscriptionCategory,
    removeSubscriptionCategory,
    addTorrentRule,
    removeTorrentRule,
    addQbServer,
    removeQbServer,
    isQbServerSaved,
    testQbServer,
    save,
    startQrLogin,
    dispose,
  });
}
