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
