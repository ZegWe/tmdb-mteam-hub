import { joinKeywordList, splitKeywordList } from "../../shared/lib/formatters.js";

const SECRET_FIELDS = Object.freeze({
  tmdb_api_key: Object.freeze({
    presenceField: "has_tmdb_api_key",
    clearField: "clear_tmdb_api_key",
  }),
  mteam_api_key: Object.freeze({
    presenceField: "has_mteam_api_key",
    clearField: "clear_mteam_api_key",
  }),
  douban_cookie: Object.freeze({
    presenceField: "has_douban_cookie",
    clearField: "clear_douban_cookie",
  }),
  admin_token: Object.freeze({
    presenceField: "has_admin_token",
    clearField: "clear_admin_token",
  }),
});

export const DEFAULT_SUBSCRIPTION_WATCHER = Object.freeze({
  enabled: false,
  dry_run: true,
  poll_interval_secs: 3600,
  library_limit: 200,
  max_retries: 3,
  search_interval_secs: 1800,
  progress_interval_secs: 5,
  link_retry_interval_secs: 900,
  system_retry_interval_secs: 600,
  bootstrap_existing_as_skipped: true,
});

let localQbServerSequence = 0;

function normalizedText(value) {
  return String(value ?? "").trim();
}

function settingsRevision(value) {
  const revision = Number(value);
  return Number.isSafeInteger(revision) && revision > 0 ? revision : 0;
}

function normalizedNonNegativeInteger(value, fallback) {
  const number = Number(value);
  return Number.isSafeInteger(number) && number >= 0 ? number : fallback;
}

export function subscriptionWatcherDto(watcher = {}) {
  return {
    enabled: watcher?.enabled === true,
    dry_run: watcher?.dry_run !== false,
    poll_interval_secs: normalizedNonNegativeInteger(
      watcher?.poll_interval_secs,
      DEFAULT_SUBSCRIPTION_WATCHER.poll_interval_secs,
    ),
    library_limit: normalizedNonNegativeInteger(
      watcher?.library_limit,
      DEFAULT_SUBSCRIPTION_WATCHER.library_limit,
    ),
    max_retries: normalizedNonNegativeInteger(
      watcher?.max_retries,
      DEFAULT_SUBSCRIPTION_WATCHER.max_retries,
    ),
    search_interval_secs: normalizedNonNegativeInteger(
      watcher?.search_interval_secs,
      DEFAULT_SUBSCRIPTION_WATCHER.search_interval_secs,
    ),
    progress_interval_secs: normalizedNonNegativeInteger(
      watcher?.progress_interval_secs,
      DEFAULT_SUBSCRIPTION_WATCHER.progress_interval_secs,
    ),
    link_retry_interval_secs: normalizedNonNegativeInteger(
      watcher?.link_retry_interval_secs,
      DEFAULT_SUBSCRIPTION_WATCHER.link_retry_interval_secs,
    ),
    system_retry_interval_secs: normalizedNonNegativeInteger(
      watcher?.system_retry_interval_secs,
      DEFAULT_SUBSCRIPTION_WATCHER.system_retry_interval_secs,
    ),
    bootstrap_existing_as_skipped: watcher?.bootstrap_existing_as_skipped !== false,
  };
}

export function subscriptionWatcherRuntimeBanner({ runtimeLoaded, watcher } = {}) {
  if (runtimeLoaded !== true) {
    return {
      key: "loading",
      tone: "info",
      title: "订阅自动化状态读取中",
      message: "正在读取后端运行配置；加载完成前不会推断自动化模式。",
    };
  }

  const runtime = subscriptionWatcherDto(watcher);
  if (!runtime.enabled) {
    return {
      key: "disabled",
      tone: "info",
      title: "订阅自动化已停用",
      message: "后台 watcher 不会自动搜索、推送 qB 或创建硬链接；手动操作仍由后端逐次校验。",
    };
  }
  if (runtime.dry_run) {
    return {
      key: "dry_run",
      tone: "warning",
      title: "订阅自动化：试运行",
      message: "会执行轮询与匹配并更新调度状态，但不会推送 qB 或创建硬链接。",
    };
  }
  return {
    key: "live",
    tone: "danger",
    title: "订阅自动化：实时执行",
    message:
      "已启用真实副作用：匹配成功后可能推送 qB，并在下载完成后创建硬链接；请确认下载与目标目录配置正确。",
  };
}

export function requiresAutomationEnableConfirmation(currentWatcher, draftWatcher) {
  return currentWatcher?.enabled !== true && draftWatcher?.enabled === true;
}

function subscriptionWatcherDraft(watcher = {}) {
  return {
    enabled: watcher?.enabled === true,
    dry_run: watcher?.dry_run !== false,
  };
}

function nextLocalQbServerId() {
  localQbServerSequence += 1;
  return `qb-${Date.now().toString(36)}-${localQbServerSequence}`;
}

function uniqueClientQbServerId(raw, usedIds) {
  const base =
    normalizedText(raw)
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, "-")
      .replace(/^-+|-+$/g, "") || nextLocalQbServerId();
  if (!usedIds.has(base)) {
    usedIds.add(base);
    return base;
  }
  for (let index = 2; ; index += 1) {
    const candidate = `${base}-${index}`;
    if (!usedIds.has(candidate)) {
      usedIds.add(candidate);
      return candidate;
    }
  }
}

export function createSettingsForm() {
  return {
    tmdb_api_key: "",
    mteam_api_key: "",
    douban_cookie: "",
    admin_token: "",
    qb_servers: [],
    subscription_categories: [],
    subscription_watcher: subscriptionWatcherDraft(),
    torrent_match_rules: [],
  };
}

export function createSecretPresence() {
  return Object.fromEntries(Object.keys(SECRET_FIELDS).map((field) => [field, false]));
}

export function createSecretClearState() {
  return Object.fromEntries(Object.keys(SECRET_FIELDS).map((field) => [field, false]));
}

export function settingsMetadataFromSnapshot(snapshot = {}) {
  const secretPresence = createSecretPresence();
  for (const [field, definition] of Object.entries(SECRET_FIELDS)) {
    secretPresence[field] = snapshot?.[definition.presenceField] === true;
  }
  return {
    revision: settingsRevision(snapshot?.revision),
    secretPresence,
  };
}

export function secretPatchFields({ value, clear, valueField, clearField } = {}) {
  if (!valueField || !clearField) throw new TypeError("secret patch field names are required");
  if (clear === true) return { [clearField]: true };
  const replacement = String(value ?? "");
  return replacement.trim() ? { [valueField]: replacement } : {};
}

export function qbServerDtos(servers) {
  return (Array.isArray(servers) ? servers : []).map((server) => ({
    id: String(server?.id ?? ""),
    name: String(server?.name ?? ""),
    base_url: String(server?.base_url ?? ""),
    username: String(server?.username ?? ""),
    insecure_tls: server?.insecure_tls === true,
    has_password: server?.has_password === true,
  }));
}

export function subscriptionCategoryDtos(categories) {
  return (Array.isArray(categories) ? categories : []).map((category) => ({ ...category }));
}

export function createQbServerDraft(server = {}, { usedIds = new Set() } = {}) {
  const [normalizedServer] = qbServerDtos([server]);
  const id = uniqueClientQbServerId(
    normalizedServer.id || normalizedServer.name || normalizedServer.base_url,
    usedIds,
  );
  return {
    ...normalizedServer,
    id,
    password: "",
    clear_password: false,
    testMessage: "",
    testKind: "",
    testing: false,
  };
}

export function createQbServerDrafts(servers) {
  const usedIds = new Set();
  return qbServerDtos(servers).map((server) => createQbServerDraft(server, { usedIds }));
}

export function createSubscriptionCategoryDraft(qbServers = []) {
  return {
    name: "",
    wanted_tag: "",
    qb_server_id: normalizedText(qbServers[0]?.id),
    qb_category: "",
    qb_save_dir_name: "",
    download_dir: "",
    link_target_dir: "",
  };
}

export function createTorrentRuleDraft(rule = {}) {
  return {
    name: rule.name || "",
    priority: Number.isFinite(Number(rule.priority)) ? Number(rule.priority) : 0,
    mode: rule.mode === "any" ? "any" : "all",
    title_keywords_text: joinKeywordList(rule.title_keywords),
    resolution_keywords_text: joinKeywordList(rule.resolution_keywords),
    source_keywords_text: joinKeywordList(rule.source_keywords),
  };
}

export function settingsFormFromSnapshot(snapshot = {}) {
  const metadata = settingsMetadataFromSnapshot(snapshot);
  const qbServers = qbServerDtos(snapshot.qb_servers);
  const qbServerDrafts = createQbServerDrafts(qbServers);
  const subscriptionCategories = subscriptionCategoryDtos(snapshot.subscription_categories);
  const subscriptionWatcher = subscriptionWatcherDto(snapshot.subscription_watcher);
  const form = createSettingsForm();
  form.qb_servers = qbServerDrafts;
  form.subscription_categories = subscriptionCategories.map((category) => ({
    ...category,
    qb_server_id: normalizedText(category.qb_server_id) || qbServerDrafts[0]?.id || "",
  }));
  form.subscription_watcher = subscriptionWatcherDraft(subscriptionWatcher);
  form.torrent_match_rules = (
    Array.isArray(snapshot.torrent_match_rules) ? snapshot.torrent_match_rules : []
  ).map(createTorrentRuleDraft);
  return {
    ...metadata,
    clearSecrets: createSecretClearState(),
    form,
    qbServers,
    subscriptionCategories,
    subscriptionWatcher,
    savedQbServerFingerprints: qbServerFingerprints(qbServers),
  };
}

export function qbServerOptionLabel(server) {
  const name = normalizedText(server?.name);
  const url = normalizedText(server?.base_url);
  if (name && url) return `${name} · ${url}`;
  return name || url || server?.id || "未命名 qB";
}

export function qbServerPatch(server = {}) {
  const patch = {
    id: normalizedText(server.id),
    name: normalizedText(server.name),
    base_url: normalizedText(server.base_url),
    username: normalizedText(server.username),
    insecure_tls: server.insecure_tls === true,
  };
  const password = String(server.password ?? "");
  if (server.clear_password === true) patch.clear_password = true;
  else if (password) patch.password = password;
  return patch;
}

function qbServerFingerprint(server) {
  return JSON.stringify(qbServerPatch(server));
}

export function qbServerFingerprints(servers) {
  return Object.fromEntries(
    (Array.isArray(servers) ? servers : []).map((server) => [
      normalizedText(server?.id),
      qbServerFingerprint(server),
    ]),
  );
}

export function isSavedQbServerDraft(server, savedFingerprints = {}) {
  const id = normalizedText(server?.id);
  return !!id && savedFingerprints[id] === qbServerFingerprint(server);
}

export function qbTestPayload(server) {
  return { server_id: normalizedText(server?.id) };
}

function subscriptionCategoryPayload(category = {}) {
  return {
    name: normalizedText(category.name),
    wanted_tag: normalizedText(category.wanted_tag),
    qb_server_id: normalizedText(category.qb_server_id),
    qb_category: normalizedText(category.qb_category),
    qb_save_dir_name: normalizedText(category.qb_save_dir_name),
    download_dir: normalizedText(category.download_dir),
    link_target_dir: normalizedText(category.link_target_dir),
  };
}

function categoryPayloadHasAnyValue(category) {
  return Object.values(category).some((value) => normalizedText(value) !== "");
}

function torrentRulePayload(rule = {}) {
  return {
    name: normalizedText(rule.name),
    priority: Number(rule.priority || 0) || 0,
    mode: rule.mode === "any" ? "any" : "all",
    title_keywords: splitKeywordList(rule.title_keywords_text),
    resolution_keywords: splitKeywordList(rule.resolution_keywords_text),
    source_keywords: splitKeywordList(rule.source_keywords_text),
  };
}

function torrentRulePayloadHasAnyValue(rule) {
  return (
    rule.name ||
    rule.priority ||
    rule.title_keywords.length ||
    rule.resolution_keywords.length ||
    rule.source_keywords.length
  );
}

export function settingsUpdatePayload({
  form = {},
  expectedRevision = 0,
  clearSecrets = {},
  subscriptionWatcher = DEFAULT_SUBSCRIPTION_WATCHER,
  confirmEnableAutomation = false,
} = {}) {
  const payload = { expected_revision: expectedRevision };
  if (confirmEnableAutomation === true) payload.confirm_enable_automation = true;
  for (const [field, definition] of Object.entries(SECRET_FIELDS)) {
    Object.assign(
      payload,
      secretPatchFields({
        value: form[field],
        clear: clearSecrets[field],
        valueField: field,
        clearField: definition.clearField,
      }),
    );
  }
  payload.qb_servers = (Array.isArray(form.qb_servers) ? form.qb_servers : [])
    .map(qbServerPatch)
    .filter((server) => server.base_url);
  payload.subscription_categories = (
    Array.isArray(form.subscription_categories) ? form.subscription_categories : []
  )
    .map(subscriptionCategoryPayload)
    .filter(categoryPayloadHasAnyValue);
  payload.subscription_watcher = {
    ...subscriptionWatcherDto(subscriptionWatcher),
    ...subscriptionWatcherDraft(form.subscription_watcher),
  };
  payload.torrent_match_rules = (
    Array.isArray(form.torrent_match_rules) ? form.torrent_match_rules : []
  )
    .map(torrentRulePayload)
    .filter(torrentRulePayloadHasAnyValue);
  return payload;
}
