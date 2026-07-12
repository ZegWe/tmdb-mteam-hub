import { formatUnixSeconds, normalizedStatus } from "../../shared/lib/formatters.js";

export const OPERATION_LOG_CATEGORIES = Object.freeze([
  { value: "subscription_sync", label: "订阅同步" },
  { value: "search", label: "搜索订阅" },
  { value: "torrent_search", label: "搜索种子" },
  { value: "qb_push", label: "推送 qB" },
  { value: "download_progress", label: "下载进度" },
  { value: "completion", label: "完成检测" },
  { value: "hardlink", label: "硬链接" },
  { value: "configuration", label: "配置保存" },
  { value: "system_error", label: "系统/错误" },
]);

export const OPERATION_LOG_STATUSES = Object.freeze([
  { value: "success", label: "成功" },
  { value: "failed", label: "失败" },
  { value: "processing", label: "处理中" },
]);

const OPERATION_LOG_ACTION_LABELS = Object.freeze({
  poll_wanted: "轮询想看",
  refresh_local: "刷新本地列表",
  search_media: "搜索影视",
  search_torrents: "搜索种子",
  match_candidates: "匹配候选种子",
  push_torrent: "订阅推送 qB",
  manual_push_torrent: "手动推送 qB",
  sync_progress: "同步下载进度",
  check_completion: "完成检测",
  link_result: "硬链接结果",
  save_config: "保存配置",
  mark_interest: "豆瓣标记",
  update_subscription_status: "更新订阅状态",
  subscription_sync_error: "订阅同步错误",
});

const OPERATION_LOG_RELATED_LABELS = Object.freeze({
  candidate_count: "候选",
  match_count: "匹配",
  selected_torrent_id: "种子",
  torrent_id: "种子",
  qb_server: "qB",
  qb_category: "分类",
  download_progress: "进度",
  file_count: "文件",
  plan_file_count: "计划",
  fetched_items: "抓取",
  inserted: "新增",
  updated: "更新",
  unchanged: "未变",
  reactivated: "恢复",
  deactivated: "停用",
  snapshot_complete: "完整快照",
});

function filterText(value) {
  return String(value ?? "").trim();
}

export function createOperationLogFilters(value = {}) {
  return {
    category: normalizedStatus(value.category),
    status: normalizedStatus(value.status),
    q: filterText(value.q),
  };
}

export function operationLogFilterKey(value = {}) {
  const filters = createOperationLogFilters(value);
  return JSON.stringify([filters.category, filters.status, filters.q]);
}

export function operationLogCategoryLabel(value) {
  const normalized = normalizedStatus(value);
  return (
    OPERATION_LOG_CATEGORIES.find((item) => item.value === normalized)?.label ||
    filterText(value) ||
    "未分类"
  );
}

export function operationLogStatusLabel(value) {
  const normalized = normalizedStatus(value);
  return (
    OPERATION_LOG_STATUSES.find((item) => item.value === normalized)?.label ||
    filterText(value) ||
    "未知"
  );
}

export function operationLogActionLabel(value) {
  const action = filterText(value);
  return OPERATION_LOG_ACTION_LABELS[action] || action || "操作";
}

export function operationLogStatusClass(value) {
  return normalizedStatus(value);
}

export function formatOperationLogTime(value) {
  return formatUnixSeconds(value);
}

export function operationLogSummary(page = {}, entries = [], filters = {}) {
  const normalizedFilters = createOperationLogFilters(filters);
  const total = Number(page.total || 0);
  const shown = Array.isArray(entries) ? entries.length : 0;
  const bits = [`共 ${total} 条`, `已显示 ${shown} 条`];
  if (normalizedFilters.category) {
    bits.push(`分类 ${operationLogCategoryLabel(normalizedFilters.category)}`);
  }
  if (normalizedFilters.status) {
    bits.push(`状态 ${operationLogStatusLabel(normalizedFilters.status)}`);
  }
  if (normalizedFilters.q) bits.push(`关键词 ${normalizedFilters.q}`);
  return bits.join(" · ");
}

export function operationLogTarget(entry = {}) {
  const parts = [
    entry.target_title || "",
    entry.target_type ? `对象 ${entry.target_type}` : "",
    entry.target_id ? `ID ${entry.target_id}` : "",
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : "无关联对象";
}

function operationLogRelatedLabel(key) {
  return OPERATION_LOG_RELATED_LABELS[key] || key;
}

export function operationLogRelated(entry = {}) {
  const fields = Array.isArray(entry?.related?.fields) ? entry.related.fields : [];
  return fields
    .filter((field) => field && field.key && field.value != null && field.value !== "")
    .slice(0, 6)
    .map((field) => `${operationLogRelatedLabel(field.key)} ${field.value}`);
}

export function operationLogTorrentMatches(entry = {}) {
  return Array.isArray(entry?.related?.torrent_matches) ? entry.related.torrent_matches : [];
}

export function operationLogMatchStats(match = {}) {
  return [
    match.torrent_id ? `ID ${match.torrent_id}` : "",
    match.seeders != null ? `做种 ${match.seeders}` : "做种 —",
    match.leechers != null ? `下载 ${match.leechers}` : "下载 —",
    match.size ? `大小 ${match.size}` : "",
    match.uploaded_at || "",
  ]
    .filter(Boolean)
    .join(" · ");
}

export function operationLogMatchedKeywords(match = {}) {
  return Array.isArray(match.matched_keywords) && match.matched_keywords.length
    ? match.matched_keywords.join("、")
    : "";
}

export function operationLogRuleEvaluationSummary(match = {}) {
  const rows = Array.isArray(match.rule_evaluations) ? match.rule_evaluations : [];
  return rows
    .map((item) => {
      const bits = [`${item.rule_name || "未命名规则"} ${item.matched ? "命中" : "未命中"}`];
      if (Array.isArray(item.matched_keywords) && item.matched_keywords.length) {
        bits.push(`命中 ${item.matched_keywords.join("、")}`);
      }
      if (Array.isArray(item.missing_keywords) && item.missing_keywords.length) {
        bits.push(`缺少 ${item.missing_keywords.join("、")}`);
      }
      if (item.excluded_reason) bits.push(item.excluded_reason);
      return bits.join("，");
    })
    .join("；");
}
