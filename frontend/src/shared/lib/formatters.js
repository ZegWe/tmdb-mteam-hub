export function joinNames(values, key = "name") {
  return Array.isArray(values)
    ? values
        .map((value) => (typeof value === "string" ? value : value?.[key]))
        .filter(Boolean)
        .join(" · ")
    : "";
}

export function joinDetailList(value) {
  return Array.isArray(value) ? value.filter(Boolean).join(" · ") : "";
}

export function normalizedStatus(value) {
  return String(value || "")
    .trim()
    .toLowerCase();
}

export function normalizeDoubanTags(value) {
  return String(value || "")
    .split(/\s+/)
    .map((item) => item.trim())
    .filter(Boolean)
    .join(" ");
}

export function mergeDoubanTagText(current, tag) {
  const next = normalizeDoubanTags(tag);
  if (!next) return normalizeDoubanTags(current);
  const parts = normalizeDoubanTags(current).split(/\s+/).filter(Boolean);
  if (!parts.includes(next)) parts.push(next);
  return parts.join(" ");
}

export function splitKeywordList(value) {
  return String(value || "")
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function joinKeywordList(values) {
  return Array.isArray(values) ? values.filter(Boolean).join(", ") : "";
}

export function formatUnixSeconds(ts) {
  const value = Number(ts);
  if (!Number.isFinite(value) || value <= 0) return "";
  return new Date(value * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatPercent(value) {
  const normalized = Number(value);
  if (!Number.isFinite(normalized)) return "";
  return `${Math.round(Math.max(0, Math.min(1, normalized)) * 100)}%`;
}

export function formatSize(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let index = 0;
  let value = bytes;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(index ? 2 : 0)} ${units[index]}`;
}

export function formatBytes(value) {
  const normalized = Number(value);
  if (!Number.isFinite(normalized) || normalized <= 0) return "";
  return formatSize(normalized);
}
