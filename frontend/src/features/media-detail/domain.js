import { formatSize, joinNames } from "../../shared/lib/formatters.js";

const TV_STATUS_ZH = {
  "Returning Series": "连载中",
  Ended: "已完结",
  Canceled: "已取消",
  "In Production": "制作中",
  Planned: "计划中",
  Pilot: "试播",
};

const MOVIE_STATUS_ZH = {
  Released: "已上映",
  PostProduction: "后期制作",
  Rumored: "传闻",
  Planned: "计划中",
  "In Production": "制作中",
  Canceled: "已取消",
};

export function tmdbMetaRows(data, mediaType) {
  const rows = [];
  if (mediaType === "tv") {
    if (data.number_of_seasons != null) rows.push(["季数", `${data.number_of_seasons} 季`]);
    if (data.number_of_episodes != null) rows.push(["总集数", `${data.number_of_episodes} 集`]);
    if (data.status) rows.push(["更新状态", TV_STATUS_ZH[data.status] || data.status]);
    if (data.first_air_date) rows.push(["首播", data.first_air_date]);
    if (data.last_air_date) rows.push(["最近播出", data.last_air_date]);
    if (Array.isArray(data.episode_run_time) && data.episode_run_time.length) {
      rows.push(["单集时长", `${data.episode_run_time[0]} 分钟`]);
    }
    const networks = joinNames(data.networks);
    if (networks) rows.push(["电视网", networks]);
    if (data.series_type) rows.push(["作品形态", data.series_type]);
  } else {
    if (data.runtime) rows.push(["片长", `${data.runtime} 分钟`]);
    if (data.status) rows.push(["状态", MOVIE_STATUS_ZH[data.status] || data.status]);
    if (data.release_date) rows.push(["上映日期", data.release_date]);
  }
  const originalTitle = data.original_title;
  const localizedTitle = data.title;
  if (originalTitle && localizedTitle && originalTitle !== localizedTitle) {
    rows.push(["原名", originalTitle]);
  }
  if (data.vote_average != null) {
    rows.push([
      "评分",
      `${Number(data.vote_average).toFixed(1)} / 10${data.vote_count != null ? `（${Number(data.vote_count).toLocaleString()} 人）` : ""}`,
    ]);
  }
  const genres = joinNames(data.genres);
  if (genres) rows.push(["类型", genres]);
  const countries =
    joinNames(data.production_countries) ||
    (Array.isArray(data.origin_country) ? data.origin_country.join(" · ") : "");
  if (countries) rows.push(["国家/地区", countries]);
  const languages =
    joinNames(data.spoken_languages, "english_name") || joinNames(data.spoken_languages, "name");
  if (languages) rows.push(["语言", languages]);
  return rows.map(([label, value]) => ({ label, value }));
}

export function doubanMetaRows(data) {
  const rows = [];
  const originalTitle = String(data.original_title || "").trim();
  const localizedTitle = String(data.title || "").trim();
  if (originalTitle && originalTitle !== localizedTitle) rows.push(["原名", originalTitle]);
  const alternateTitles = joinNames(data.aka);
  if (alternateTitles) rows.push(["又名", alternateTitles]);
  const countries = joinNames(data.countries);
  if (countries) rows.push(["国家/地区", countries]);
  const languages = joinNames(data.languages);
  if (languages) rows.push(["语言", languages]);
  if (data.rating?.value != null) {
    rows.push([
      "评分",
      `${Number(data.rating.value).toFixed(1)} / 10${data.rating.count != null ? `（${Number(data.rating.count).toLocaleString()} 人）` : ""}`,
    ]);
  }
  if (data.date_published) rows.push(["发布日期", data.date_published]);
  if (data.duration) rows.push(["片长", data.duration]);
  if (Array.isArray(data.genres) && data.genres.length)
    rows.push(["类型", data.genres.join(" · ")]);
  if (Array.isArray(data.directors) && data.directors.length) {
    rows.push(["导演", data.directors.join(" · ")]);
  }
  if (Array.isArray(data.writers) && data.writers.length) {
    rows.push(["编剧", data.writers.join(" · ")]);
  }
  if (Array.isArray(data.actors) && data.actors.length) {
    rows.push(["主演", data.actors.slice(0, 10).join(" · ")]);
  }
  return rows.map(([label, value]) => ({ label, value }));
}

export function imdbFromDetail(data) {
  return data?.imdb_id || null;
}

export function imdbHref(imdbRaw) {
  if (imdbRaw == null) return null;
  const value = String(imdbRaw).trim();
  if (!value) return null;
  const id = value.startsWith("tt") ? value : `tt${value}`;
  return `https://www.imdb.com/title/${id}/`;
}

export function doubanFromDetail(data) {
  const asDigitsId = (value) => {
    if (value == null) return null;
    const normalized = String(value).trim();
    return /^\d+$/.test(normalized) ? normalized : null;
  };
  let id = asDigitsId(data?.douban_id);
  if (!id && data?.douban_url) {
    const match = String(data.douban_url).match(/douban\.com\/subject\/(\d+)/i);
    if (match) id = match[1];
  }
  return id;
}

export function doubanUrlFromDetail(data) {
  const id = doubanFromDetail(data);
  if (data?.douban_url && String(data.douban_url).trim()) return String(data.douban_url).trim();
  return id ? `https://movie.douban.com/subject/${id}/` : null;
}

export function episodeStill(episode) {
  if (episode?.still_url && String(episode.still_url).trim()) {
    return String(episode.still_url).trim();
  }
  return "";
}

// Conservatively recognizes the coverage advertised by a TV torrent title.
// Unknown titles are never treated as packs automatically.
export function classifyTvTorrentTitle(title, expectedSeason = null, episodeTotal = null) {
  const raw = String(title || "").trim();
  const normalized = raw.toLowerCase().replace(/[–—]/g, "-");
  const season = positiveInteger(expectedSeason);
  const total = positiveInteger(episodeTotal);

  const seasonEpisode = normalized.match(
    /(?:^|[^a-z0-9])s(\d{1,2})[ ._-]*e(\d{1,3})(?:[ ._-]*(?:e)?(\d{1,3}))?(?=$|[^a-z0-9])/i,
  );
  if (seasonEpisode) {
    return episodeCoverage(
      Number(seasonEpisode[1]),
      Number(seasonEpisode[2]),
      Number(seasonEpisode[3] || seasonEpisode[2]),
      season,
      total,
    );
  }

  const bareEpisode = normalized.match(
    /(?:^|[^a-z0-9])(?:episode|ep|e)(\d{1,3})(?:[ ._-]*(?:(?:episode|ep|e)[ ._-]*)?(\d{1,3}))?(?=$|[^a-z0-9])/i,
  );
  if (bareEpisode) {
    return episodeCoverage(
      season,
      Number(bareEpisode[1]),
      Number(bareEpisode[2] || bareEpisode[1]),
      season,
      total,
    );
  }

  const chineseEpisode = raw.match(
    /第\s*(\d{1,3})\s*集?(?:\s*[-~至到]\s*(?:第\s*)?(\d{1,3})\s*集)?/,
  );
  if (chineseEpisode) {
    return episodeCoverage(
      season,
      Number(chineseEpisode[1]),
      Number(chineseEpisode[2] || chineseEpisode[1]),
      season,
      total,
    );
  }

  const plainRange = season
    ? normalized.match(/(?:^|[^a-z0-9])(\d{1,3})\s*[-~]\s*(\d{1,3})(?=$|[^a-z0-9])/)
    : null;
  if (plainRange) {
    return episodeCoverage(season, Number(plainRange[1]), Number(plainRange[2]), season, total);
  }

  const seasonMarker = normalized.match(
    /(?:^|[^a-z0-9])s(?:eason[ ._-]*)?(\d{1,2})(?=$|[^a-z0-9])/i,
  );
  const wordSeason = normalized.match(/season[ ._-]*(\d{1,2})(?=$|[^a-z0-9])/i);
  const packSeason = Number(seasonMarker?.[1] || wordSeason?.[1] || season || 0) || null;
  const packKeyword =
    /\b(?:complete|full[ ._-]*season|season[ ._-]*pack|batch|collection)\b/i.test(normalized) ||
    /全季|全集|合集|整季/.test(raw);
  if (packSeason && (packKeyword || seasonMarker)) {
    const compatible = !season || packSeason === season;
    return {
      kind: "season_pack",
      seasonNumber: packSeason,
      episodeStart: 1,
      episodeEnd: total,
      compatible,
      label: "S" + pad2(packSeason) + " 整季合集",
    };
  }

  return {
    kind: "unknown",
    seasonNumber: null,
    episodeStart: null,
    episodeEnd: null,
    compatible: false,
    label: "未识别集数",
  };
}

function episodeCoverage(foundSeason, start, end, expectedSeason, total) {
  const normalizedSeason = positiveInteger(foundSeason) || expectedSeason;
  const normalizedStart = positiveInteger(start);
  const normalizedEnd = positiveInteger(end);
  if (!normalizedStart || !normalizedEnd || normalizedEnd < normalizedStart) {
    return {
      kind: "unknown",
      seasonNumber: normalizedSeason,
      episodeStart: null,
      episodeEnd: null,
      compatible: false,
      label: "未识别集数",
    };
  }
  const compatible =
    (!expectedSeason || !normalizedSeason || normalizedSeason === expectedSeason) &&
    (!total || normalizedEnd <= total);
  const kind = normalizedEnd > normalizedStart ? "partial_pack" : "episode";
  const seasonLabel = normalizedSeason ? "S" + pad2(normalizedSeason) : "";
  const episodeLabel =
    kind === "episode"
      ? "E" + pad2(normalizedStart)
      : "E" + pad2(normalizedStart) + "-E" + pad2(normalizedEnd);
  return {
    kind,
    seasonNumber: normalizedSeason,
    episodeStart: normalizedStart,
    episodeEnd: normalizedEnd,
    compatible,
    label: (seasonLabel + episodeLabel + " " + (kind === "episode" ? "单集" : "部分合集")).trim(),
  };
}

function positiveInteger(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : null;
}

function pad2(value) {
  return String(value).padStart(2, "0");
}

export function mteamTorrentWebUrl(torrentId) {
  const id = String(torrentId ?? "").trim();
  return id ? `https://kp.m-team.cc/detail/${encodeURIComponent(id)}` : "https://kp.m-team.cc/";
}

export function torrentStats(torrent) {
  return [
    torrent?.size != null ? formatSize(Number(torrent.size)) : "",
    `做种 ${torrent?.seeders ?? "—"}`,
    `下载 ${torrent?.leechers ?? "—"}`,
    torrent?.created_at || "",
  ]
    .filter(Boolean)
    .join(" · ");
}
