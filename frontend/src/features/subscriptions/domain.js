import {
  formatBytes,
  formatUnixSeconds,
  joinDetailList,
  normalizedStatus,
} from "../../shared/lib/formatters.js";

const SUB_LIFECYCLE_LABELS = {
  queued: "待处理",
  meta: "准备元数据",
  searching: "搜索中",
  downloading: "下载中",
  linking: "硬链接中",
  completed: "已完成",
};

const SUB_ATTENTION_LABELS = {
  failed: "失败",
  skipped: "已跳过",
  waiting_release: "等待发布",
  retry_blocked: "重试阻塞",
};

const PUSH_STATUS_LABELS = {
  pushed: "已推送",
  downloading: "下载中",
  downloaded: "已下载",
  completed: "已链接",
  failed: "失败",
  dry_run: "预演",
  pending: "等待完成",
  planned: "计划链接",
  linked: "已链接",
  missing: "缺集",
  duplicate: "重复",
  conflict: "冲突",
  needs_review: "需确认",
  partial: "部分完成",
  ignored: "已忽略",
  superseded: "已替代",
};

const SUB_SKIP_REASON_LABELS = {
  initial_bootstrap_existing_wish: "历史想看，首次同步跳过",
};

const SUB_BLOCK_REASON_LABELS = {
  subscription_inactive: "订阅已停用",
  tv_not_supported: "TV 自动化尚未开放",
};

const SUB_LIFECYCLE_STEPS = [
  { key: "queued", label: "入队" },
  { key: "meta", label: "元数据" },
  { key: "searching", label: "搜索" },
  { key: "downloading", label: "下载" },
  { key: "linking", label: "硬链接中" },
  { key: "completed", label: "完成" },
];

const SUB_ATTENTION_PRIORITY = ["skipped", "retry_blocked", "failed", "waiting_release"];

const SUMMARY_LIFECYCLE_LABELS = [
  ["queued", "待处理"],
  ["meta", "元数据"],
  ["searching", "搜索中"],
  ["downloading", "下载中"],
  ["linking", "硬链接中"],
  ["completed", "完成"],
];

const SUMMARY_ATTENTION_LABELS = [
  ["failed", "失败"],
  ["skipped", "跳过"],
  ["waiting_release", "等待发布"],
  ["retry_blocked", "重试阻塞"],
];

function subscriptionAttentionKeys(record) {
  const tags = Array.isArray(record?.attention_tags)
    ? record.attention_tags.map((tag) => normalizedStatus(tag))
    : [];
  const activeTags = new Set(tags);
  return SUB_ATTENTION_PRIORITY.filter((tag) => activeTags.has(tag));
}

export function subscriptionSummary(records) {
  if (!Array.isArray(records) || !records.length) return "尚未加载订阅状态";

  const lifecycleCounts = Object.create(null);
  const attentionCounts = Object.create(null);
  for (const record of records) {
    const lifecycle = subscriptionLifecycleKey(record);
    lifecycleCounts[lifecycle] = (lifecycleCounts[lifecycle] || 0) + 1;
    for (const attention of subscriptionAttentionKeys(record)) {
      attentionCounts[attention] = (attentionCounts[attention] || 0) + 1;
    }
  }

  const bits = [`总计 ${records.length}`];
  for (const [key, label] of SUMMARY_LIFECYCLE_LABELS) {
    if (lifecycleCounts[key]) bits.push(`${label} ${lifecycleCounts[key]}`);
  }
  for (const [key, label] of SUMMARY_ATTENTION_LABELS) {
    if (attentionCounts[key]) bits.push(`${label} ${attentionCounts[key]}`);
  }
  return bits.join(" · ");
}

export function subscriptionPollToast(outcome) {
  if (!outcome || typeof outcome !== "object") return "订阅刷新完成";
  const snapshot = outcome.snapshot_complete ? "完整快照" : "非完整快照";
  return `订阅刷新完成：抓取 ${Number(outcome.fetched_items || 0)} · 新增 ${Number(outcome.inserted || 0)} · 更新 ${Number(outcome.updated || 0)} · 未变 ${Number(outcome.unchanged || 0)} · 恢复 ${Number(outcome.reactivated || 0)} · 停用 ${Number(outcome.deactivated || 0)} · ${snapshot}`;
}

function formatSubscriptionSkipReason(value) {
  const raw = String(value || "").trim();
  if (!raw) return "";
  const mapped = SUB_SKIP_REASON_LABELS[raw] || SUB_SKIP_REASON_LABELS[raw.toLowerCase()];
  if (mapped) return mapped;
  if (/^[a-z0-9_-]+$/i.test(raw)) return `跳过原因：${raw.replace(/[_-]+/g, " ")}`;
  return raw;
}

export function subscriptionDisplayStatus(record) {
  const lifecycle = subscriptionLifecycleKey(record);
  const attention = subscriptionAttentionKey(record);
  if (attention) {
    return { key: attention, text: SUB_ATTENTION_LABELS[attention] || attention };
  }
  return { key: lifecycle, text: SUB_LIFECYCLE_LABELS[lifecycle] || "待处理" };
}

export function subscriptionProgress(record) {
  const downloads = Array.isArray(record?.downloads) ? record.downloads : [];
  for (let index = downloads.length - 1; index >= 0; index -= 1) {
    const progress = Number(downloads[index]?.progress);
    if (Number.isFinite(progress)) return Math.max(0, Math.min(1, progress));
  }
  if (subscriptionLifecycleKey(record) === "completed") return 1;
  return null;
}

export function subscriptionLifecycleNodes(record) {
  const currentKey = subscriptionLifecycleKey(record);
  const currentIndex = Math.max(
    0,
    SUB_LIFECYCLE_STEPS.findIndex((step) => step.key === currentKey),
  );
  const attention = subscriptionAttentionKey(record);
  return SUB_LIFECYCLE_STEPS.map((step, index) => ({
    ...step,
    state: index < currentIndex ? "done" : index === currentIndex ? "current" : "todo",
    attention: index === currentIndex ? attention : "",
  }));
}

export function subscriptionLifecycleKey(record) {
  const lifecycle = normalizedStatus(record?.lifecycle_state);
  if (SUB_LIFECYCLE_STEPS.some((step) => step.key === lifecycle)) return lifecycle;
  return "queued";
}

export function subscriptionAttentionKey(record) {
  return subscriptionAttentionKeys(record)[0] || "";
}

export function subscriptionCapabilities(record) {
  const blockedReason = normalizedStatus(record?.blocked_reason);
  const inactive =
    record?.active === false || record?.active === 0 || blockedReason === "subscription_inactive";
  const mediaKind = normalizedStatus(record?.media_kind) === "tv" ? "tv" : "movie";
  const tvUnsupported = blockedReason === "tv_not_supported";
  const explicitSchedulable = normalizedBoolean(record?.schedulable);
  const schedulable =
    !inactive && !tvUnsupported && !blockedReason && explicitSchedulable !== false;
  const lifecycle = subscriptionLifecycleKey(record);
  const blockedReasonText = formatSubscriptionBlockedReason(blockedReason);
  const explanation = subscriptionCapabilityExplanation({
    inactive,
    tvUnsupported,
    schedulable,
    blockedReasonText,
    lifecycle,
    mediaKind,
  });

  const badges = [];
  if (inactive) badges.push({ key: "inactive", text: "已停用", tone: "muted" });
  if (tvUnsupported) badges.push({ key: "tv_unsupported", text: "TV 未开放", tone: "warning" });
  if (!inactive && !tvUnsupported && blockedReason) {
    badges.push({ key: "blocked_reason", text: blockedReasonText, tone: "danger" });
  }
  badges.push(
    schedulable
      ? { key: "schedulable", text: "可调度", tone: "success" }
      : { key: "blocked", text: "不可调度", tone: "muted" },
  );

  return {
    active: !inactive,
    mediaKind,
    tvUnsupported,
    schedulable,
    blockedReason,
    blockedReasonText,
    explanation,
    badges,
  };
}

function formatSubscriptionBlockedReason(value) {
  const raw = normalizedStatus(value);
  if (!raw) return "";
  const mapped = SUB_BLOCK_REASON_LABELS[raw];
  if (mapped) return mapped;
  if (/^[a-z0-9_-]+$/i.test(raw)) return `自动处理受限：${raw.replace(/[_-]+/g, " ")}`;
  return `自动处理受限：${raw}`;
}

export function subscriptionCardSubtitle(record) {
  return String(record?.release_year || "").trim() || record?.subject_id || "";
}

export function subscriptionDetailRows(record) {
  const source =
    record?.source && typeof record.source === "object" && !Array.isArray(record.source)
      ? record.source
      : {};
  const observation =
    record?.observation &&
    typeof record.observation === "object" &&
    !Array.isArray(record.observation)
      ? record.observation
      : {};
  const capabilities = subscriptionCapabilities(record);
  const tv = record?.tv;
  return [
    row("豆瓣 ID", record.subject_id),
    row("分类文本", record.category_text),
    row("上映日期", source.date_published),
    row("评分", subscriptionRatingText(source)),
    row("原名", source.original_title),
    row("又名", joinDetailList(source.aka)),
    row("类型", joinDetailList(source.genres)),
    row("国家/地区", joinDetailList(source.countries)),
    row("语言", joinDetailList(source.languages)),
    row("导演", joinDetailList(source.directors)),
    row("主演", joinDetailList(source.actors)),
    row("片长", source.duration),
    row("简介", source.synopsis),
    row("豆瓣时间", source.douban_date),
    row("上映年份", record.release_year),
    row("季数", tv ? tv.season_number : null),
    row("总集数", tv?.episode_total),
    row("目标范围", tv ? `${tv.target_start_episode} – ${tv.target_end_episode}` : null),
    row("订阅活动", capabilities.active ? "活跃" : "已停用"),
    row("媒体能力", capabilities.mediaKind === "tv" ? "TV 自动化" : "电影自动化"),
    row("调度能力", capabilities.schedulable ? "可调度" : "不可调度"),
    row("阻止原因", capabilities.blockedReasonText),
    row("跳过原因", formatSubscriptionSkipReason(record.skip_reason)),
    row("重试", `${record.retry_count || 0}/${record.max_retries || 0}`),
    row("首次看到", formatUnixSeconds(observation.first_seen_at)),
    row("最近看到", formatUnixSeconds(observation.last_seen_at)),
    row("最近更新", formatUnixSeconds(record.updated_at)),
  ].filter(Boolean);
}

function normalizedBoolean(value) {
  if (value === true || value === 1) return true;
  if (value === false || value === 0) return false;
  return null;
}

function subscriptionCapabilityExplanation({
  inactive,
  tvUnsupported,
  schedulable,
  blockedReasonText,
  lifecycle,
  mediaKind,
}) {
  if (inactive) {
    return "该订阅已不在当前完整想看快照中，仅保留历史记录；重新加入想看并完成同步后可恢复。";
  }
  if (tvUnsupported) {
    return "TV 自动化尚未开放；当前仅保留订阅与历史数据，不会执行搜索、下载或硬链接。";
  }
  if (!schedulable) {
    return blockedReasonText
      ? `${blockedReasonText}；后台任务会等待状态恢复。`
      : "后端已将此订阅标记为不可调度；后台任务会等待状态恢复。";
  }
  if (lifecycle === "completed") {
    return `该${mediaLabel(mediaKind)}订阅已完成。`;
  }
  return `该${mediaLabel(mediaKind)}订阅当前可由后台任务调度。`;
}

function mediaLabel(mediaKind) {
  return mediaKind === "tv" ? "TV" : "电影";
}

function subscriptionRatingText(source) {
  const rating = Number(source?.rating_value);
  if (!Number.isFinite(rating)) return "";
  const count = Number(source?.rating_count);
  return Number.isFinite(count) && count > 0
    ? `${rating.toFixed(1)}（${count.toLocaleString()} 人）`
    : rating.toFixed(1);
}

export function downloadArtifactRows(download) {
  const files = Array.isArray(download?.files) ? download.files : [];
  const completedFiles = files.filter((file) => Number(file?.progress) >= 1).length;
  return [
    row("种子", download.torrent_title),
    row("qB", download.qb_server_name || download.qb_server_id),
    row("分类", download.qb_category),
    row("保存目录", download.qb_save_dir_name),
    row("qB 状态", download.qb_state || pushStatusLabel(download.state)),
    row("qB hash", download.qb_hash),
    row("qB 名称", download.qb_name),
    row("文件", files.length ? `${completedFiles}/${files.length}` : ""),
    row("大小", formatBytes(download.total_size)),
    row("推送时间", formatUnixSeconds(download.pushed_at)),
    row("检查时间", formatUnixSeconds(download.checked_at)),
    row("完成时间", formatUnixSeconds(download.completed_at)),
  ].filter(Boolean);
}

export function downloadEpisodeLabel(download) {
  const files = Array.isArray(download?.files) ? download.files : [];
  const episodes = files
    .filter((f) => Number.isInteger(f?.episode_number) || f?.episode_label)
    .map((f) => ({
      season: f.season_number,
      episode: f.episode_number,
    }));

  if (!episodes.length) return "";

  const bySeason = new Map();
  for (const ep of episodes) {
    const s = ep.season ?? 0;
    if (!bySeason.has(s)) bySeason.set(s, []);
    bySeason.get(s).push(ep.episode);
  }

  const parts = [];
  for (const [season, epNums] of bySeason) {
    const sorted = [...new Set(epNums.filter((n) => Number.isInteger(n)))].sort((a, b) => a - b);
    if (!sorted.length) continue;
    const range = formatEpisodeRange(sorted);
    const prefix = season > 0 ? `S${String(season).padStart(2, "0")}` : "";
    parts.push(prefix ? `${prefix}${range}` : range);
  }

  return parts.join(" / ");
}

function formatEpisodeRange(nums) {
  if (nums.length === 1) return `E${String(nums[0]).padStart(2, "0")}`;
  const min = nums[0];
  const max = nums[nums.length - 1];
  if (max - min + 1 === nums.length && nums.every((n, i) => n === min + i)) {
    return `E${String(min).padStart(2, "0")}-E${String(max).padStart(2, "0")}`;
  }
  return nums.map((n) => `E${String(n).padStart(2, "0")}`).join(",");
}

export function matchLinksToDownloads(downloads, links) {
  const downloadList = Array.isArray(downloads) ? downloads : [];
  const linkList = Array.isArray(links) ? links : [];
  const usedLinkIds = new Set();

  const tasks = downloadList.map((download, index) => {
    const matchedLinks = linkList.filter(
      (link) => link.download_artifact_id === download.id,
    );
    for (const link of matchedLinks) usedLinkIds.add(link.id || link.key);

    const downloadFiles = (Array.isArray(download?.files) ? download.files : []).map((file) => ({
      ...file,
      status: file.progress != null ? "" : download.state,
    }));

    const linkFiles = matchedLinks.flatMap((link) =>
      (Array.isArray(link?.files) ? link.files : []).map((file) => ({
        ...file,
        status: file.outcome || link.state,
      })),
    );

    return {
      key: download.id || `${download.torrent_id || "download"}-${index}`,
      label: download.qb_name || download.torrent_title || download.id || String(index + 1),
      state: download.state,
      episodeLabel: downloadEpisodeLabel(download),
      downloadRows: downloadArtifactRows(download),
      matchedLinks: matchedLinks.map((link, linkIndex) => ({
        key: link.id || `link-${linkIndex}`,
        label: link.target_dir || link.id || String(linkIndex + 1),
        rows: linkArtifactRows(link),
      })),
      allFiles: [...downloadFiles, ...linkFiles],
    };
  });

  const orphanLinks = linkList
    .filter((link) => !usedLinkIds.has(link.id || link.key))
    .map((link, index) => ({
      key: link.id || `orphan-link-${index}`,
      label: link.target_dir || link.id || String(index + 1),
      rows: linkArtifactRows(link),
      files: (Array.isArray(link?.files) ? link.files : []).map((file) => ({
        ...file,
        status: file.outcome || link.state,
      })),
    }));

  return { tasks, orphanLinks };
}

export function linkArtifactRows(link) {
  return [
    row("链接状态", pushStatusLabel(link.state)),
    row("目标目录", link.target_dir),
    row("源目录", link.source_path),
    row("检查时间", formatUnixSeconds(link.checked_at)),
    row("完成时间", formatUnixSeconds(link.completed_at)),
    row("下载任务", link.download_artifact_id),
  ].filter(Boolean);
}

function row(label, value, href = "") {
  if (value == null || String(value).trim() === "") return null;
  const text = String(value);
  const link = String(href || "").trim();
  return link ? { label, value: text, href: link } : { label, value: text };
}

export function pushStatusLabel(status) {
  return PUSH_STATUS_LABELS[normalizedStatus(status)] || status || "";
}
