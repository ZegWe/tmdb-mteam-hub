function normalizedText(value) {
  return String(value ?? "").trim();
}

export function qbPushPayload({ serverId, torrentId, category, savepath } = {}) {
  const payload = {
    server_id: normalizedText(serverId),
    torrent_id: normalizedText(torrentId),
  };
  const normalizedCategory = normalizedText(category);
  const normalizedSavepath = normalizedText(savepath);
  if (normalizedCategory) payload.category = normalizedCategory;
  if (normalizedSavepath) payload.savepath = normalizedSavepath;
  return payload;
}
