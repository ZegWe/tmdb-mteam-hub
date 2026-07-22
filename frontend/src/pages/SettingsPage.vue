<template>
  <section id="page-settings" class="app-page is-active">
    <header class="top settings-top">
      <div>
        <h1>设置</h1>
        <p class="sub">API、自动订阅、豆瓣登录与 qBittorrent</p>
      </div>
      <div class="actions">
        <button
          v-if="authContext?.canLogout?.value"
          type="button"
          class="btn btn-ghost"
          :disabled="authContext?.logoutLoading?.value"
          @click="authContext.logout()"
        >
          {{ authContext?.logoutLoading?.value ? "退出中…" : "退出登录" }}
        </button>
      </div>
    </header>

    <form id="settings-form" class="settings-page-form" @submit.prevent="saveSettings">
      <section class="settings-section card bg-base-100 border border-base-300">
        <h2>管理访问</h2>
        <p class="hint">
          管理 Token 至少 24 个字符。替换后当前浏览器会立即用新值重新登录；Token
          不会写入浏览器存储。
        </p>
        <label
          >管理 Token<input
            v-model="settings.admin_token"
            type="password"
            class="input input-bordered"
            autocomplete="new-password"
            minlength="24"
            :placeholder="secretPresence.admin_token ? '已配置；留空则保留' : '尚未配置'"
            @input="onSecretInput('admin_token')"
        /></label>
        <label v-if="secretPresence.admin_token" class="qb-insecure"
          ><input
            v-model="clearSecrets.admin_token"
            type="checkbox"
            class="checkbox checkbox-sm"
            @change="onSecretClearChange('admin_token')"
          />
          清除已保存的管理 Token</label
        >
        <p v-if="clearSecrets.admin_token" class="hint alert alert-warning" role="alert">
          只有 loopback
          监听允许清除最后一个管理凭据；远程或容器监听会拒绝保存。清除后请立即确认服务仍只对可信本机开放。
        </p>
      </section>

      <section class="settings-section card bg-base-100 border border-base-300">
        <h2>API 密钥</h2>
        <p class="hint">
          将写入运行目录下的 <code>config.toml</code>（或通过环境变量
          <code>CONFIG_PATH</code> 指定路径）。
        </p>
        <label
          >TMDB API Key<input
            v-model="settings.tmdb_api_key"
            type="password"
            class="input input-bordered"
            autocomplete="off"
            :placeholder="secretPresence.tmdb_api_key ? '已配置；留空则保留' : '尚未配置'"
            @input="onSecretInput('tmdb_api_key')"
        /></label>
        <label v-if="secretPresence.tmdb_api_key" class="qb-insecure"
          ><input
            v-model="clearSecrets.tmdb_api_key"
            type="checkbox"
            class="checkbox checkbox-sm"
            @change="onSecretClearChange('tmdb_api_key')"
          />
          清除已保存的 TMDB API Key</label
        >
        <label
          >M-Team OpenAPI Key<input
            v-model="settings.mteam_api_key"
            type="password"
            class="input input-bordered"
            autocomplete="off"
            :placeholder="secretPresence.mteam_api_key ? '已配置；留空则保留' : '尚未配置'"
            @input="onSecretInput('mteam_api_key')"
        /></label>
        <label v-if="secretPresence.mteam_api_key" class="qb-insecure"
          ><input
            v-model="clearSecrets.mteam_api_key"
            type="checkbox"
            class="checkbox checkbox-sm"
            @change="onSecretClearChange('mteam_api_key')"
          />
          清除已保存的 M-Team OpenAPI Key</label
        >
        <label
          >豆瓣 Cookie<textarea
            v-model="settings.douban_cookie"
            class="textarea textarea-bordered"
            rows="3"
            autocomplete="off"
            spellcheck="false"
            :placeholder="
              secretPresence.douban_cookie
                ? '已配置；留空则保留，也可重新扫码更新'
                : 'dbcl2=...; ck=...'
            "
            @input="onSecretInput('douban_cookie')"
          ></textarea>
        </label>
        <label v-if="secretPresence.douban_cookie" class="qb-insecure"
          ><input
            v-model="clearSecrets.douban_cookie"
            type="checkbox"
            class="checkbox checkbox-sm"
            @change="onSecretClearChange('douban_cookie')"
          />
          清除已保存的豆瓣 Cookie</label
        >
        <div class="douban-login-tools">
          <button
            type="button"
            class="btn btn-secondary"
            :disabled="qrLoading"
            @click="startDoubanQrLogin"
          >
            QR 登录获取 Cookie
          </button>
          <span class="hint subtle" aria-live="polite">{{ doubanQrStatus }}</span>
        </div>
        <div v-if="doubanQrImage" class="douban-qr-box">
          <img :src="doubanQrImage" alt="豆瓣登录二维码" />
        </div>
      </section>

      <section
        class="settings-section subscription-automation-fieldset card bg-base-100 border border-base-300"
      >
        <h2>订阅自动化</h2>
        <p class="hint">默认关闭。启用后服务会定时读取想看订阅并推进检索、下载与硬链接流程。</p>
        <label class="qb-insecure"
          ><input
            v-model="settings.subscription_watcher.enabled"
            type="checkbox"
            class="checkbox checkbox-sm"
          />
          启用自动订阅</label
        >
        <label class="qb-insecure"
          ><input
            v-model="settings.subscription_watcher.dry_run"
            type="checkbox"
            class="checkbox checkbox-sm"
          />
          试运行（dry-run）</label
        >
        <p v-if="!settings.subscription_watcher.enabled" class="hint subtle" role="status">
          当前关闭：不会运行自动订阅流水线。
        </p>
        <p
          v-else-if="settings.subscription_watcher.dry_run"
          class="hint alert alert-warning"
          role="status"
        >
          试运行会访问外部服务并更新调度状态，但不会推送 qB 或创建硬链接。
        </p>
        <p v-else class="hint alert alert-error" role="alert">
          实时模式会自动推送 qB，并可能在下载完成后创建硬链接；保存时需要再次确认。
        </p>
      </section>

      <section
        class="settings-section subscription-categories-fieldset card bg-base-100 border border-base-300"
      >
        <h2>订阅分类</h2>
        <p class="hint">
          “想看”只能选择这里配置的文本；分类保存后会写入配置文件，后续自动下载与硬链接使用同一组字段。
        </p>
        <div class="subscription-categories-list">
          <p
            v-if="!settings.subscription_categories.length"
            class="subtle subscription-category-empty"
          >
            未配置订阅分类，可点下方「添加分类」
          </p>
          <div
            v-for="(category, idx) in settings.subscription_categories"
            :key="idx"
            class="subscription-category-row"
          >
            <label
              >分类名<input
                v-model="category.name"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 电影"
            /></label>
            <label
              >想看文本<input
                v-model="category.wanted_tag"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 电影"
            /></label>
            <label
              >qB 服务器<select
                v-model="category.qb_server_id"
                class="select select-bordered select-sm"
                :disabled="!settings.qb_servers.length"
              >
                <option v-if="!settings.qb_servers.length" value="">请先添加 qB 服务器</option>
                <option v-for="server in settings.qb_servers" :key="server.id" :value="server.id">
                  {{ qbServerOptionLabel(server) }}
                </option>
              </select></label
            >
            <label
              >qB 下载分类<input
                v-model="category.qb_category"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 movie"
            /></label>
            <label
              >qB 保存目录名<input
                v-model="category.qb_save_dir_name"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 movies"
            /></label>
            <label
              >真实下载目录<input
                v-model="category.download_dir"
                type="text"
                class="input input-bordered input-sm"
                placeholder="/downloads/movies"
            /></label>
            <label
              >硬链接目标目录<input
                v-model="category.link_target_dir"
                type="text"
                class="input input-bordered input-sm"
                placeholder="/media/movies"
            /></label>
            <div class="subscription-category-actions">
              <p class="hint subtle">
                修改想看文本后，已有订阅记录可能仍保留旧文本；后续状态迁移需按订阅记录处理。
              </p>
              <button
                type="button"
                class="btn btn-sm btn-ghost"
                @click="removeSubscriptionCategory(idx)"
              >
                移除
              </button>
            </div>
          </div>
        </div>
        <button type="button" class="btn btn-secondary" @click="addSubscriptionCategory">
          添加分类
        </button>
      </section>

      <section
        class="settings-section torrent-rules-fieldset card bg-base-100 border border-base-300"
      >
        <h2>种子匹配规则</h2>
        <p class="hint">
          数字越大越先尝试；高优先级没有候选命中时才尝试低优先级。关键词用逗号分隔。
        </p>
        <div class="torrent-rules-list">
          <p v-if="!settings.torrent_match_rules.length" class="subtle torrent-rule-empty">
            未配置规则；自动推送会使用首个候选种子。
          </p>
          <div
            v-for="(rule, idx) in settings.torrent_match_rules"
            :key="idx"
            class="torrent-rule-row"
          >
            <label
              >规则名<input
                v-model="rule.name"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 优先 2160p BluRay"
            /></label>
            <label
              >优先级<input
                v-model.number="rule.priority"
                type="number"
                class="input input-bordered input-sm"
                step="1"
                placeholder="100"
            /></label>
            <label
              >匹配模式<select
                v-model="rule.mode"
                class="select select-bordered select-sm"
                :title="rule.mode === 'all' ? '全部满足' : '任一满足'"
              >
                <option value="all">全部满足</option>
                <option value="any">任一满足</option>
              </select></label
            >
            <label
              >标题关键词<input
                v-model="rule.title_keywords_text"
                type="text"
                class="input input-bordered input-sm"
                placeholder="2160p, 4K"
            /></label>
            <label
              >分辨率关键词<input
                v-model="rule.resolution_keywords_text"
                type="text"
                class="input input-bordered input-sm"
                placeholder="1080p, 2160p"
            /></label>
            <label
              >版本/来源关键词<input
                v-model="rule.source_keywords_text"
                type="text"
                class="input input-bordered input-sm"
                placeholder="BluRay, REMUX, WEB-DL"
            /></label>
            <div class="torrent-rule-actions">
              <p class="hint subtle">保存后自动订阅推送会按优先级生成可解释的候选匹配结果。</p>
              <button type="button" class="btn btn-sm btn-ghost" @click="removeTorrentRule(idx)">
                移除
              </button>
            </div>
          </div>
        </div>
        <button type="button" class="btn btn-secondary" @click="addTorrentRule">添加规则</button>
      </section>

      <section class="settings-section qb-servers-fieldset card bg-base-100 border border-base-300">
        <h2>qBittorrent</h2>
        <p class="hint">
          在本机可访问的 qB Web UI；保存后会写入配置文件，下次打开设置会从此处加载。
        </p>
        <div class="qb-servers-list">
          <p v-if="!settings.qb_servers.length" class="subtle qb-empty">
            未配置 qB 服务器，可点下方「添加」
          </p>
          <div v-for="(server, idx) in settings.qb_servers" :key="idx" class="qb-server-row">
            <label
              >显示名<input
                v-model="server.name"
                type="text"
                class="input input-bordered input-sm"
                placeholder="如 家用 NAS"
            /></label>
            <label
              >Web UI 根地址<input
                v-model="server.base_url"
                type="text"
                class="input input-bordered input-sm"
                placeholder="http://127.0.0.1:8080"
            /></label>
            <label
              >用户名<input
                v-model="server.username"
                type="text"
                class="input input-bordered input-sm"
                autocomplete="off"
            /></label>
            <label
              >密码<input
                v-model="server.password"
                type="password"
                class="input input-bordered input-sm"
                autocomplete="off"
                :placeholder="server.has_password ? '已配置；留空则保留' : '未配置密码'"
                @input="onQbPasswordInput(server)"
            /></label>
            <div class="qb-row-actions">
              <label class="qb-insecure"
                ><input
                  v-model="server.insecure_tls"
                  type="checkbox"
                  class="checkbox checkbox-sm"
                />
                忽略 HTTPS 证书错误</label
              >
              <label v-if="server.has_password" class="qb-insecure"
                ><input
                  v-model="server.clear_password"
                  type="checkbox"
                  class="checkbox checkbox-sm"
                  @change="onQbPasswordClearChange(server)"
                />
                清除已保存密码</label
              >
              <div class="qb-row-tail">
                <button
                  type="button"
                  class="btn btn-sm btn-secondary"
                  :disabled="server.testing || !isQbServerSaved(server)"
                  :title="isQbServerSaved(server) ? '测试已保存配置' : '请先保存后测试'"
                  @click="testQbServer(server)"
                >
                  测试连接
                </button>
                <span v-if="!isQbServerSaved(server)" class="hint subtle">请先保存后测试</span>
                <span
                  class="qb-test-msg"
                  :class="
                    server.testKind === 'err'
                      ? 'qb-test-msg-error'
                      : server.testKind === 'ok'
                        ? 'qb-test-msg-ok'
                        : 'subtle'
                  "
                  aria-live="polite"
                  >{{ server.testMessage }}</span
                >
                <button type="button" class="btn btn-sm btn-ghost" @click="removeQbServer(idx)">
                  移除
                </button>
              </div>
            </div>
          </div>
        </div>
        <button type="button" class="btn btn-secondary" @click="addQbServer">添加服务器</button>
      </section>

      <div class="form-actions">
        <p
          id="settings-save-status"
          class="form-status"
          :class="settingsStatus.kind ? `is-${settingsStatus.kind}` : ''"
          role="status"
          aria-live="polite"
        >
          {{ settingsStatus.message }}
        </p>
        <button type="submit" class="btn btn-primary" :disabled="savingSettings || !settingsLoaded">
          保存设置
        </button>
      </div>
    </form>
  </section>
</template>

<script setup>
import { inject, onBeforeUnmount, onMounted } from "vue";
import { AUTH_CONTEXT_KEY } from "../app/auth-context.js";
import { APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS } from "../app/notifications.js";
import { qbServerOptionLabel } from "../features/settings/form-model.js";
import { SETTINGS_STORE_KEY } from "../features/settings/store.js";

const settingsStore = inject(SETTINGS_STORE_KEY, null);
if (!settingsStore) throw new Error("SettingsPage requires a provided settings store");

const notifications = inject(APP_NOTIFICATIONS_KEY, NOOP_APP_NOTIFICATIONS);
const authContext = inject(AUTH_CONTEXT_KEY, null);
const settings = settingsStore.form;
const secretPresence = settingsStore.secretPresence;
const clearSecrets = settingsStore.clearSecrets;
const settingsLoaded = settingsStore.pageLoaded;
const savingSettings = settingsStore.saving;
const settingsStatus = settingsStore.saveStatus;
const qrLoading = settingsStore.qrLoading;
const doubanQrStatus = settingsStore.qrStatus;
const doubanQrImage = settingsStore.qrImage;

const onSecretInput = settingsStore.onSecretInput;
const onSecretClearChange = settingsStore.onSecretClearChange;
const onQbPasswordInput = settingsStore.onQbPasswordInput;
const onQbPasswordClearChange = settingsStore.onQbPasswordClearChange;
const addSubscriptionCategory = settingsStore.addSubscriptionCategory;
const addTorrentRule = settingsStore.addTorrentRule;
const addQbServer = settingsStore.addQbServer;
const isQbServerSaved = settingsStore.isQbServerSaved;
const testQbServer = settingsStore.testQbServer;

onMounted(async () => {
  notifications.clearError();
  try {
    await settingsStore.enterPage();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const detail = `加载设置失败：${message}`;
    notifications.showError(detail);
    notifications.showToast(detail, "err");
  }
});

onBeforeUnmount(() => {
  settingsStore.leavePage();
});

async function saveSettings() {
  notifications.clearError();
  try {
    const snapshot = await settingsStore.save({
      confirmEnableAutomation: confirmAutomationEnable,
    });
    if (snapshot) notifications.showToast("设置已保存", "ok");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    notifications.showError(`保存设置失败：${message}`);
    notifications.showToast(`保存失败：${message}`, "err");
  }
}

function confirmAutomationEnable({ dryRun } = {}) {
  if (typeof globalThis.confirm !== "function") return false;
  const effects = dryRun
    ? "当前为试运行：不会推送 qB 或创建硬链接，但仍会访问外部服务并更新订阅调度状态。"
    : "当前为实时模式：会自动检索种子、推送到 qBittorrent，并可能在下载完成后创建硬链接。";
  return globalThis.confirm(`即将启用订阅自动化。\n\n${effects}\n\n确认继续保存吗？`);
}

function startDoubanQrLogin() {
  notifications.clearError();
  return settingsStore.startQrLogin({
    notify: (message, kind) => notifications.showToast(message, kind),
  });
}

function removeSubscriptionCategory(index) {
  settingsStore.removeSubscriptionCategory(index);
}

function removeTorrentRule(index) {
  settingsStore.removeTorrentRule(index);
}

function removeQbServer(index) {
  settingsStore.removeQbServer(index);
}
</script>

<style src="../features/settings/styles.css"></style>
