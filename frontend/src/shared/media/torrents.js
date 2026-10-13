import { formatSize } from "../lib/formatters.js";

export function mteamTorrentWebUrl(torrentId) {
  const id = String(torrentId ?? "").trim();
  return id ? `https://kp.m-team.cc/detail/${encodeURIComponent(id)}` : "https://kp.m-team.cc/";
}

export function torrentStats(torrent) {
  const size = torrent?.size;
  const bytes = Number(size);
  const formattedSize =
    size == null || String(size).trim() === ""
      ? "—"
      : Number.isFinite(bytes) && bytes >= 0
        ? formatSize(bytes)
        : String(size);
  return [
    `大小 ${formattedSize}`,
    `做种 ${torrent?.seeders ?? "—"}`,
    `下载中 ${torrent?.leechers ?? "—"}`,
    torrent?.uploaded_at || torrent?.created_at
      ? `发布 ${torrent.uploaded_at || torrent.created_at}`
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
}
