const IMG_BASE = "https://image.tmdb.org/t/p/w342";

/** @type {{ name?: string, base_url?: string, username?: string, password?: string, insecure_tls?: boolean }[] | null} */
let qbServersCache = null;

const $ = (sel) => document.querySelector(sel);

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

function posterUrl(path) {
  if (!path) return "";
  return `${IMG_BASE}${path}`;
}

function showErr(msg) {
  const el = $("#err");
  el.textContent = msg;
  el.classList.remove("hidden");
}

function clearErr() {
  const el = $("#err");
  el.classList.add("hidden");
  el.textContent = "";
}

function mteamTorrentWebUrl(torrentId) {
  const id = String(torrentId ?? "").trim();
  if (!id) return "https://kp.m-team.cc/";
  return `https://kp.m-team.cc/detail/${encodeURIComponent(id)}`;
}

let toastTimer;

function hideToast() {
  const el = $("#toast");
  if (el) el.classList.add("hidden");
}

function showToast(message, kind = "ok") {
  let el = $("#toast");
  if (!el) {
    if (kind === "err") showErr(message);
    return;
  }
  el.textContent = message;
  el.classList.remove("hidden", "toast-ok", "toast-err");
  el.classList.add(kind === "err" ? "toast-err" : "toast-ok");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(hideToast, 3800);
}

async function resolveQbServers() {
  if (qbServersCache) return qbServersCache;
  try {
    const c = await api("/api/config");
    qbServersCache = Array.isArray(c.qb_servers) ? c.qb_servers : [];
  } catch {
    qbServersCache = [];
  }
  return qbServersCache;
}

function setSearchLoading(on) {
  const overlay = $("#layout-loading");
  const btn = $("#btn-search");
  const q = $("#q");
  if (overlay) {
    overlay.classList.toggle("hidden", !on);
    overlay.setAttribute("aria-hidden", on ? "false" : "true");
  }
  if (btn) btn.disabled = !!on;
  if (q) q.disabled = !!on;
  $("#main-layout")?.setAttribute("aria-busy", on ? "true" : "false");
}

async function api(path, opts = {}) {
  const r = await fetch(path, {
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    ...opts,
  });
  const text = await r.text();
  let data;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { raw: text };
  }
  if (!r.ok) {
    const msg = data?.error || r.statusText || "请求失败";
    throw new Error(msg);
  }
  return data;
}

function renderCards(container, items, type) {
  container.innerHTML = "";
  if (!items.length) {
    container.innerHTML = '<p class="empty-hint">无结果</p>';
    return;
  }
  for (const it of items) {
    const div = document.createElement("div");
    div.className = "card";
    div.dataset.type = it.media_type || type;
    div.dataset.id = String(it.id);
    const img = document.createElement("img");
    img.src = posterUrl(it.poster_path) || "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    img.alt = "";
    img.loading = "lazy";
    const meta = document.createElement("div");
    meta.className = "meta";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = it.title || "(无标题)";
    const sub = document.createElement("div");
    sub.className = "subtle";
    const d = it.release_date || it.first_air_date || "";
    sub.textContent = [d, it.vote_average != null ? `★ ${it.vote_average.toFixed(1)}` : ""]
      .filter(Boolean)
      .join(" · ");
    meta.append(title, sub);
    div.append(img, meta);
    div.addEventListener("click", () => openDetail(div.dataset.type, Number(div.dataset.id)));
    container.append(div);
  }
}

function imdbFromDetail(d) {
  if (d.imdb_id) return d.imdb_id;
  const ext = d.external_ids;
  if (ext?.imdb_id) return ext.imdb_id;
  return null;
}

function doubanFromDetail(d) {
  const asDigitsId = (v) => {
    if (v == null) return null;
    const s = String(v).trim();
    if (!s) return null;
    if (/^\d+$/.test(s)) return s;
    return null;
  };

  let id = asDigitsId(d.douban_id);
  const ext = d.external_ids;
  if (!id && ext) {
    id = asDigitsId(ext.douban_id);
    if (!id && ext.douban != null) id = asDigitsId(ext.douban);
  }
  if (!id && d.douban_url) {
    const m = String(d.douban_url).trim().match(/douban\.com\/subject\/(\d+)/i);
    if (m) id = m[1];
  }
  return id;
}

function doubanUrlFromDetail(d) {
  const id = doubanFromDetail(d);
  if (d.douban_url && String(d.douban_url).trim()) return String(d.douban_url).trim();
  if (id) return `https://movie.douban.com/subject/${id}/`;
  return null;
}

function translateMovieStatus(s) {
  if (!s) return "";
  return MOVIE_STATUS_ZH[s] || s;
}

function translateTvStatus(s) {
  if (!s) return "";
  return TV_STATUS_ZH[s] || s;
}

function formatMoneyUsd(n) {
  if (n == null || n <= 0) return null;
  if (n >= 1e9) return `约 $${(n / 1e9).toFixed(2)}B`;
  if (n >= 1e6) return `约 $${(n / 1e6).toFixed(1)}M`;
  return `$${n.toLocaleString()}`;
}

function joinNames(arr, key = "name") {
  if (!Array.isArray(arr)) return "";
  return arr
    .map((x) => (typeof x === "string" ? x : x[key]))
    .filter(Boolean)
    .join(" · ");
}

function imdbHref(imdbRaw) {
  if (imdbRaw == null) return null;
  const s = String(imdbRaw).trim();
  if (!s) return null;
  const id = s.startsWith("tt") ? s : `tt${s}`;
  return `https://www.imdb.com/title/${id}/`;
}

function renderExternalIdsBar(d, mediaType, numericId) {
  const parts = [];
  if (numericId != null && String(numericId).trim() !== "") {
    const p = mediaType === "tv" ? "tv" : "movie";
    const u = `https://www.themoviedb.org/${p}/${numericId}`;
    parts.push(
      `<a class="detail-ext-link tag" href="${escapeHtml(u)}" target="_blank" rel="noopener noreferrer">TMDB · ${escapeHtml(String(numericId))}</a>`,
    );
  }
  const imdb = imdbFromDetail(d);
  const ih = imdbHref(imdb);
  if (imdb && ih) {
    parts.push(
      `<a class="detail-ext-link tag" href="${escapeHtml(ih)}" target="_blank" rel="noopener noreferrer">IMDb · ${escapeHtml(String(imdb))}</a>`,
    );
  }
  const wd = d.external_ids?.wikidata_id;
  if (wd && String(wd).trim()) {
    const wid = escapeHtml(String(wd).trim());
    parts.push(
      `<a class="detail-ext-link tag" href="https://www.wikidata.org/wiki/${wid}" target="_blank" rel="noopener noreferrer">Wikidata · ${wid}</a>`,
    );
  }
  const dubId = doubanFromDetail(d);
  const dubUrl = doubanUrlFromDetail(d);
  if (dubId && dubUrl) {
    parts.push(
      `<a class="detail-ext-link tag" href="${escapeHtml(dubUrl)}" target="_blank" rel="noopener noreferrer">豆瓣 · ${escapeHtml(dubId)}</a>`,
    );
  }
  if (!parts.length) return "";
  return `<div class="detail-external-ids">${parts.join('<span class="id-separator" aria-hidden="true"> · </span>')}</div>`;
}

function renderDetailMeta(d, mediaType) {
  const rows = [];

  if (mediaType === "tv") {
    if (d.number_of_seasons != null) rows.push(["季数", `${d.number_of_seasons} 季`]);
    if (d.number_of_episodes != null) rows.push(["总集数", `${d.number_of_episodes} 集`]);
    if (d.status) rows.push(["更新状态", translateTvStatus(d.status)]);
    if (d.first_air_date) rows.push(["首播", d.first_air_date]);
    if (d.last_air_date) rows.push(["最近播出", d.last_air_date]);
    const ne = d.next_episode_to_air;
    if (ne && (ne.air_date || ne.name)) {
      const se =
        ne.season_number != null && ne.episode_number != null
          ? `S${ne.season_number}E${ne.episode_number}`
          : "";
      const bits = [ne.air_date, se, ne.name].filter(Boolean);
      if (bits.length) rows.push(["下一集", bits.join(" · ")]);
    }
    const le = d.last_episode_to_air;
    if (le && (le.air_date || le.name)) {
      const se =
        le.season_number != null && le.episode_number != null
          ? `S${le.season_number}E${le.episode_number}`
          : "";
      const bits = [le.air_date, se, le.name].filter(Boolean);
      if (bits.length) rows.push(["最近一集", bits.join(" · ")]);
    }
    const ert = d.episode_run_time;
    if (Array.isArray(ert) && ert.length) {
      const m = ert[0];
      rows.push(["单集时长", ert.length > 1 ? `${Math.min(...ert)}–${Math.max(...ert)} 分钟` : `${m} 分钟`]);
    }
    const nets = joinNames(d.networks, "name");
    if (nets) rows.push(["电视网", nets]);
    if (d.type) rows.push(["作品形态", d.type]);
  } else {
    if (d.runtime) rows.push(["片长", `${d.runtime} 分钟`]);
    if (d.status) rows.push(["状态", translateMovieStatus(d.status)]);
    if (d.release_date) rows.push(["上映日期", d.release_date]);
    const bud = formatMoneyUsd(d.budget);
    const rev = formatMoneyUsd(d.revenue);
    if (bud) rows.push(["预算", bud]);
    if (rev) rows.push(["票房", rev]);
  }

  const orig = d.original_title || d.original_name;
  const loc = d.title || d.name;
  if (orig && loc && orig !== loc) rows.push(["原名", orig]);

  if (d.vote_average != null) {
    const vc = d.vote_count != null ? `（${d.vote_count.toLocaleString()} 人）` : "";
    rows.push(["评分", `${Number(d.vote_average).toFixed(1)} / 10${vc}`]);
  }

  const genres = joinNames(d.genres, "name");
  if (genres) rows.push(["类型", genres]);

  const countries =
    joinNames(d.production_countries, "name") ||
    (Array.isArray(d.origin_country) ? d.origin_country.join(" · ") : "");
  if (countries) rows.push(["国家/地区", countries]);

  const langs = joinNames(d.spoken_languages, "english_name") || joinNames(d.spoken_languages, "name");
  if (langs) rows.push(["语言", langs]);

  if (!rows.length) return "";
  const html = rows
    .map(
      ([k, v]) =>
        `<div class="detail-meta-row"><dt>${escapeHtml(k)}</dt><dd>${escapeHtml(String(v))}</dd></div>`
    )
    .join("");
  return `<dl class="detail-meta">${html}</dl>`;
}

async function openDetail(mediaType, id) {
  clearErr();
  const drawer = $("#detail");
  const body = $("#detail-body");
  drawer.classList.remove("is-off");
  body.innerHTML = `
    <div class="detail-loading" role="status">
      <div class="spinner" aria-hidden="true"></div>
      <p>加载详情…</p>
    </div>`;

  const path = mediaType === "tv" ? `/api/tmdb/tv/${id}` : `/api/tmdb/movie/${id}`;
  let d;
  try {
    d = await api(path);
  } catch (e) {
    body.innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(e.message)}</p>`;
    showErr(e.message);
    return;
  }

  try {
    const imdb = imdbFromDetail(d);
    const doubanId = doubanFromDetail(d);
    const doubanUrl = doubanUrlFromDetail(d);
    const torrentKeywordTitle =
      (mediaType === "tv"
        ? (d.original_name || "").trim()
        : (d.original_title || "").trim()) || "";
    const canMteam = !!(imdb || doubanId || torrentKeywordTitle);
    const title = d.title || d.name || "";
    const date = d.release_date || d.first_air_date || "";
    const metaHtml = renderDetailMeta(d, mediaType);
    const externalIdsHtml = renderExternalIdsBar(d, mediaType, id);
    const tagline = d.tagline ? `<p class="tagline-block">${escapeHtml(d.tagline)}</p>` : "";
    const tvEpMount = mediaType === "tv" ? '<div id="tv-seasons-mount" class="tv-seasons-mount"></div>' : "";

    body.innerHTML = `
    <div class="d-head">
      ${
        d.poster_url
          ? `<img src="${escapeHtml(d.poster_url)}" alt="" />`
          : d.poster_path
            ? `<img src="${escapeHtml(posterUrl(d.poster_path))}" alt="" />`
            : ""
      }
      <h3>${escapeHtml(title)}</h3>
      <div class="detail-type-line">
        <span class="tag">${mediaType === "tv" ? "剧集" : "电影"}</span>
        ${date ? `<span class="tag">${escapeHtml(date)}</span>` : ""}
      </div>
      ${externalIdsHtml}
      ${tagline}
      ${metaHtml}
      ${tvEpMount}
      <p class="overview">${escapeHtml(d.overview || "")}</p>
      <div class="row-actions mteam-actions">
        ${
          canMteam
            ? `<span class="mteam-actions-label subtle">M-Team</span>
        <div id="mteam-tablist" class="mteam-tablist" role="tablist" aria-label="M-Team 检索路径"></div>`
            : `<span class="subtle">缺少 IMDb / 豆瓣 ID，且无原标题，无法在 M-Team 检索</span>`
        }
      </div>
      <div id="torrent-box" class="torrent-list"></div>
    </div>`;

    if (mediaType === "tv") {
      mountTvEpisodesBrowser(id, d);
    }

    if (canMteam) {
      const tablist = $("#mteam-tablist");
      const box = $("#torrent-box");
      if (tablist && box) {
        setupMteamSourceTabs(tablist, box, { imdb, doubanId, keyword: torrentKeywordTitle });
      }
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    body.innerHTML = `<p class="empty-hint">页面渲染失败：${escapeHtml(msg)}</p>`;
    showErr(msg);
  }
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function renderEpisodeList(seasonJson) {
  const eps = Array.isArray(seasonJson?.episodes) ? seasonJson.episodes : [];
  if (!eps.length) {
    return '<p class="subtle">本季暂无分集数据</p>';
  }
  const rows = [];
  for (const ep of eps) {
    const num = ep.episode_number;
    const title = ep.name || (num != null ? `第 ${num} 集` : "（无标题）");
    const air =
      ep.air_date && String(ep.air_date).trim()
        ? `<span class="tv-ep-air">${escapeHtml(String(ep.air_date).trim())}</span>`
        : "";
    let still =
      ep.still_url && String(ep.still_url).trim() ? String(ep.still_url).trim() : "";
    if (!still && ep.still_path && String(ep.still_path).trim()) {
      still = `https://image.tmdb.org/t/p/w185${String(ep.still_path).trim()}`;
    }
    const img = still
      ? `<img class="tv-ep-still" src="${escapeHtml(still)}" alt="" loading="lazy" />`
      : `<div class="tv-ep-still tv-ep-still-placeholder" aria-hidden="true"></div>`;
    const ov =
      ep.overview && String(ep.overview).trim()
        ? `<p class="tv-ep-overview">${escapeHtml(String(ep.overview).trim())}</p>`
        : "";
    const numLabel = num != null ? `E${num}` : "E—";
    rows.push(`<div class="tv-episode-row">
      <div class="tv-ep-thumb">${img}</div>
      <div class="tv-ep-main">
        <div class="tv-ep-title-line">
          <span class="tv-ep-num">${escapeHtml(numLabel)}</span>
          <span class="tv-ep-title">${escapeHtml(title)}</span>
          ${air}
        </div>
        ${ov}
      </div>
    </div>`);
  }
  return rows.join("");
}

function mountTvEpisodesBrowser(tvId, showDetail) {
  const mount = document.getElementById("tv-seasons-mount");
  if (!mount) return;

  const seasons = Array.isArray(showDetail?.seasons) ? [...showDetail.seasons] : [];
  if (!seasons.length) {
    mount.innerHTML = '<p class="subtle">暂无分季信息</p>';
    return;
  }

  seasons.sort((a, b) => (a.season_number ?? 0) - (b.season_number ?? 0));

  mount.innerHTML =
    '<h4 class="tv-episodes-heading">分集</h4><div class="tv-seasons-list" role="list"></div>';
  const list = mount.querySelector(".tv-seasons-list");

  for (const s of seasons) {
    const sn = s.season_number;
    if (sn == null) continue;

    const det = document.createElement("details");
    det.className = "tv-season-block";
    det.dataset.loaded = "0";

    const sum = document.createElement("summary");
    sum.className = "tv-season-summary";
    const ec = s.episode_count != null ? `${s.episode_count} 集` : "";
    const sname = s.name && String(s.name).trim() ? ` · ${escapeHtml(String(s.name).trim())}` : "";
    sum.innerHTML = `<span class="tv-season-label">第 ${escapeHtml(String(sn))} 季${sname}</span>${
      ec ? `<span class="tv-season-meta subtle">${escapeHtml(ec)}</span>` : ""
    }`;

    const body = document.createElement("div");
    body.className = "tv-season-body";
    body.innerHTML = '<p class="subtle tv-season-placeholder">展开以加载本季分集…</p>';

    det.append(sum, body);

    det.addEventListener("toggle", async () => {
      if (!det.open || det.dataset.loaded === "1" || det.dataset.loading === "1") return;
      det.dataset.loading = "1";
      body.innerHTML =
        '<div class="inline-loading tv-season-loading" role="status"><div class="spinner spinner-sm" aria-hidden="true"></div><span>加载中…</span></div>';
      try {
        const seasonJson = await api(`/api/tmdb/tv/${tvId}/season/${sn}`);
        body.innerHTML = renderEpisodeList(seasonJson);
        det.dataset.loaded = "1";
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        body.innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(msg)}</p>`;
      } finally {
        det.dataset.loading = "0";
      }
    });

    list.append(det);
  }
}

function setupMteamSourceTabs(tablistEl, torrentBoxEl, opts) {
  const imdb = opts.imdb ?? null;
  const doubanId = opts.doubanId ?? null;
  const keyword = opts.keyword ?? "";
  /** @type {{ source: string, label: string }[]} */
  const sources = [];
  if (imdb) sources.push({ source: "imdb", label: "IMDb" });
  if (doubanId) sources.push({ source: "douban", label: "豆瓣 ID" });
  if (keyword) sources.push({ source: "keyword", label: "原标题" });
  if (!sources.length) return;

  /** @type {Record<string, unknown>} */
  const cache = Object.create(null);

  function paramsFor(source) {
    const p = new URLSearchParams({ source });
    if (source === "imdb") p.set("imdb_id", imdb);
    else if (source === "douban") p.set("douban_id", doubanId);
    else p.set("keyword", keyword);
    return p;
  }

  async function selectSource(source) {
    for (const btn of tablistEl.querySelectorAll('[role="tab"]')) {
      const on = btn.dataset.source === source;
      btn.classList.toggle("is-active", !!on);
      btn.setAttribute("aria-selected", on ? "true" : "false");
      btn.tabIndex = on ? 0 : -1;
    }

    const box = torrentBoxEl;
    if (cache[source] !== undefined) {
      await renderTorrents(box, cache[source]);
      return;
    }
    box.innerHTML = `<div class="inline-loading" role="status">
      <div class="spinner spinner-sm" aria-hidden="true"></div>
      <span>正在加载 M-Team…</span>
    </div>`;
    try {
      const res = await api(`/api/mteam/torrents?${paramsFor(source)}`);
      cache[source] = res;
      await renderTorrents(box, res);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showErr(msg);
      box.innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(msg)}</p>`;
    }
  }

  tablistEl.innerHTML = "";
  for (const s of sources) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "mteam-tab";
    b.dataset.source = s.source;
    b.setAttribute("role", "tab");
    b.textContent = s.label;
    b.tabIndex = -1;
    b.addEventListener("click", () => {
      selectSource(s.source).catch(() => {});
    });
    tablistEl.appendChild(b);
  }

  selectSource(sources[0].source).catch(() => {});
}

function extractTorrentRows(res) {
  if (!res || typeof res !== "object") return [];
  const d = res.data;
  if (Array.isArray(d)) return d;
  if (d && typeof d === "object") {
    if (Array.isArray(d.data)) return d.data;
    if (Array.isArray(d.list)) return d.list;
  }
  if (Array.isArray(res.results)) return res.results;
  return [];
}

async function renderTorrents(container, res) {
  const rows = extractTorrentRows(res);
  if (!rows.length) {
    container.innerHTML = `<p class="empty-hint">未返回种子列表（请检查 M-Team 返回结构或账号权限）。</p><pre style="font-size:0.7rem;overflow:auto;max-height:180px;color:var(--muted)">${escapeHtml(JSON.stringify(res, null, 2))}</pre>`;
    return;
  }
  container.innerHTML = '<h4 class="torrent-list-title">M-Team 种子</h4><div class="torrent-cards"></div>';
  const mount = container.querySelector(".torrent-cards");
  if (!mount) return;

  for (const t of rows) {
    const tid = t.id != null ? String(t.id).trim() : "";
    const st = t.status || {};

    const card = document.createElement("article");
    card.className = "torrent-card";
    if (tid) card.dataset.torrentId = tid;

    const inner = document.createElement("div");
    inner.className = "torrent-card-inner";

    const link = document.createElement("a");
    link.className = "torrent-card-link";
    link.href = mteamTorrentWebUrl(tid || "0");
    link.target = "_blank";
    link.rel = "noopener noreferrer";

    const name = document.createElement("div");
    name.className = "torrent-name";
    name.textContent = t.name || t.title || tid || "(无标题)";

    const stats = document.createElement("div");
    stats.className = "torrent-stats";
    const size = t.size != null ? formatSize(Number(t.size)) : "";
    stats.textContent = [
      size,
      `做种 ${st.seeders ?? t.seeders ?? "—"}`,
      `下载 ${st.leechers ?? t.leechers ?? "—"}`,
      t.createdDate || t.created_date || "",
    ]
      .filter(Boolean)
      .join(" · ");

    const desc = document.createElement("div");
    desc.className = "torrent-desc";
    desc.textContent = t.smallDescr || t.small_descr || "";

    link.append(name, stats, desc);

    const actions = document.createElement("div");
    actions.className = "torrent-card-actions";
    actions.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
    });

    if (!tid) {
      actions.innerHTML = `<span class="subtle torrent-push-hint" title="无种子 ID，无法推送">—</span>`;
    } else {
      const pushBtn = document.createElement("button");
      pushBtn.type = "button";
      pushBtn.className = "btn btn-mini primary torrent-push-trigger";
      pushBtn.textContent = "推送 qB";
      pushBtn.title = "推送到 qBittorrent";
      const titleStr = String(name.textContent || "").trim();
      pushBtn.addEventListener("click", (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        openQbPushDialog(tid, titleStr).catch(() => {});
      });
      actions.appendChild(pushBtn);
    }

    inner.append(link, actions);
    card.appendChild(inner);
    mount.appendChild(card);
  }
}

const qbPushContext = { torrentId: "", title: "" };

async function openQbPushDialog(torrentId, title) {
  qbPushContext.torrentId = torrentId;
  qbPushContext.title = title || "";
  const dlg = document.getElementById("qb-push");
  const lbl = document.getElementById("qb-push-torrent-label");
  if (lbl) {
    lbl.textContent = qbPushContext.title
      ? `${qbPushContext.title}（${qbPushContext.torrentId}）`
      : `种子 ID · ${qbPushContext.torrentId}`;
  }
  const catIn = document.getElementById("qb-push-category");
  const pathIn = document.getElementById("qb-push-savepath");
  if (catIn) catIn.value = "";
  if (pathIn) pathIn.value = "";

  const sel = document.getElementById("qb-push-server");
  if (!sel || !dlg) return;

  sel.innerHTML = "";
  const servers = await resolveQbServers();
  if (!servers.length) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = "未配置 qB（请打开 API 设置）";
    sel.appendChild(opt);
    sel.disabled = true;
  } else {
    sel.disabled = false;
    servers.forEach((s, i) => {
      const opt = document.createElement("option");
      opt.value = String(i);
      opt.textContent = (s.name || "").trim() || s.base_url?.trim() || `服务器 ${i + 1}`;
      sel.appendChild(opt);
    });
  }

  dlg.showModal();
}

function formatSize(bytes) {
  if (!bytes) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 2 : 0)} ${u[i]}`;
}

function qbPayloadFromServerRow(row) {
  return {
    name: row.querySelector('[data-qb="name"]')?.value?.trim() ?? "",
    base_url: row.querySelector('[data-qb="base_url"]')?.value?.trim() ?? "",
    username: row.querySelector('[data-qb="username"]')?.value?.trim() ?? "",
    password: row.querySelector('[data-qb="password"]')?.value ?? "",
    insecure_tls: !!row.querySelector('[data-qb="insecure_tls"]')?.checked,
  };
}

function makeQbServerRowEl(s = {}) {
  const row = document.createElement("div");
  row.className = "qb-server-row";
  row.innerHTML = `
    <label>显示名<input type="text" data-qb="name" placeholder="如 家用 NAS" /></label>
    <label>Web UI 根地址<input type="text" data-qb="base_url" placeholder="http://127.0.0.1:8080" /></label>
    <label>用户名<input type="text" data-qb="username" autocomplete="off" /></label>
    <label>密码<input type="password" data-qb="password" autocomplete="off" /></label>
    <div class="qb-row-actions">
      <label class="qb-insecure"><input type="checkbox" data-qb="insecure_tls" /> 忽略 HTTPS 证书错误</label>
      <div class="qb-row-tail">
        <button type="button" class="btn btn-mini secondary qb-test-qb-server">测试连接</button>
        <span class="qb-test-msg subtle" aria-live="polite"></span>
        <button type="button" class="btn btn-mini qb-remove-server">移除</button>
      </div>
    </div>`;

  row.querySelector('[data-qb="name"]').value = s.name ?? "";
  row.querySelector('[data-qb="base_url"]').value = s.base_url ?? "";
  row.querySelector('[data-qb="username"]').value = s.username ?? "";
  row.querySelector('[data-qb="password"]').value = s.password ?? "";
  if (s.insecure_tls) {
    const icb = row.querySelector('[data-qb="insecure_tls"]');
    if (icb) icb.checked = true;
  }

  row.querySelector(".qb-test-qb-server")?.addEventListener("click", async () => {
    const btn = row.querySelector(".qb-test-qb-server");
    const msgEl = row.querySelector(".qb-test-msg");
    const payload = qbPayloadFromServerRow(row);
    if (!payload.base_url) {
      if (msgEl) {
        msgEl.textContent = "请先填写 Web UI 根地址";
        msgEl.classList.remove("subtle");
        msgEl.classList.add("qb-test-msg-error");
        msgEl.classList.remove("qb-test-msg-ok");
      }
      return;
    }
    if (btn) btn.disabled = true;
    if (msgEl) {
      msgEl.textContent = "正在测试…";
      msgEl.classList.add("subtle");
      msgEl.classList.remove("qb-test-msg-error", "qb-test-msg-ok");
    }
    try {
      const r = await api("/api/qb/test", { method: "POST", body: JSON.stringify(payload) });
      const v = r.version ? ` · ${r.version}` : "";
      if (msgEl) {
        msgEl.textContent = `可连通${v}`;
        msgEl.classList.remove("subtle");
        msgEl.classList.add("qb-test-msg-ok");
      }
    } catch (err) {
      if (msgEl) {
        msgEl.textContent = err.message || String(err);
        msgEl.classList.remove("subtle");
        msgEl.classList.add("qb-test-msg-error");
      }
    } finally {
      if (btn) btn.disabled = false;
    }
  });

  row.querySelector(".qb-remove-server")?.addEventListener("click", () => {
    row.remove();
    const list = $("#qb-servers-list");
    if (list && !list.querySelector(".qb-server-row")) {
      list.innerHTML = '<p class="subtle qb-empty">未配置 qB 服务器，可点下方「添加」</p>';
    }
  });
  return row;
}

function renderQbServersEditor(servers) {
  const list = $("#qb-servers-list");
  if (!list) return;
  list.innerHTML = "";
  const arr = Array.isArray(servers) ? servers : [];
  if (!arr.length) {
    list.innerHTML = '<p class="subtle qb-empty">未配置 qB 服务器，可点下方「添加」</p>';
    return;
  }
  for (const s of arr) {
    list.appendChild(makeQbServerRowEl(s));
  }
}

function collectQbServersFromDom() {
  const list = $("#qb-servers-list");
  if (!list) return [];
  return [...list.querySelectorAll(".qb-server-row")]
    .map((row) => qbPayloadFromServerRow(row))
    .filter((x) => x.base_url !== "");
}

async function runSearch() {
  clearErr();
  const q = $("#q").value.trim();
  if (!q) {
    showErr("请输入搜索词");
    return;
  }
  setSearchLoading(true);
  try {
    const data = await api(`/api/search?${new URLSearchParams({ q })}`);
    renderCards($("#movies"), data.movies || [], "movie");
    renderCards($("#tv"), data.tv || [], "tv");
  } catch (e) {
    showErr(e.message);
  } finally {
    setSearchLoading(false);
  }
}

function openSettings() {
  $("#modal-bg").classList.remove("hidden");
  $("#settings").showModal?.();
  if (!document.getElementById("settings").open) {
    document.getElementById("settings").setAttribute("open", "");
  }
}

function closeSettings() {
  $("#modal-bg").classList.add("hidden");
  const d = $("#settings");
  if (d.close) d.close();
  else d.removeAttribute("open");
}

async function loadSettings() {
  try {
    const c = await api("/api/config");
    $("#f-tmdb").value = c.tmdb_api_key || "";
    $("#f-mteam").value = c.mteam_api_key || "";
    renderQbServersEditor(c.qb_servers);
    qbServersCache = Array.isArray(c.qb_servers) ? c.qb_servers : [];
  } catch {
    /* ignore */
  }
}

$("#btn-search").addEventListener("click", runSearch);
$("#q").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runSearch();
});
$("#btn-settings").addEventListener("click", async () => {
  await loadSettings();
  openSettings();
});
$("#modal-bg").addEventListener("click", closeSettings);
$("#btn-cancel-settings").addEventListener("click", closeSettings);

$("#btn-qb-add")?.addEventListener("click", () => {
  const list = $("#qb-servers-list");
  if (!list) return;
  list.querySelector(".qb-empty")?.remove();
  list.appendChild(makeQbServerRowEl({}));
});

$("#detail-close").addEventListener("click", () => $("#detail").classList.add("is-off"));

$("#qb-push-cancel")?.addEventListener("click", () => {
  $("#qb-push")?.close?.();
});

$("#qb-push-form")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  const servers = await resolveQbServers();
  if (!servers.length) {
    showToast("请先在 API 设置中配置 qB 服务器", "err");
    return;
  }
  const sel = $("#qb-push-server");
  if (!sel?.options?.length || sel.disabled) {
    showToast("没有可用的 qB 服务器", "err");
    return;
  }
  const idx = Number(sel.value);
  const server = servers[idx];
  if (!server?.base_url?.trim()) {
    showToast("所选 qB 服务器无效", "err");
    return;
  }
  const submitBtn = $("#qb-push-submit");
  if (submitBtn) submitBtn.disabled = true;
  try {
    await api("/api/qb/push-mteam", {
      method: "POST",
      body: JSON.stringify({
        server,
        torrent_id: qbPushContext.torrentId,
        category: $("#qb-push-category").value.trim() || undefined,
        savepath: $("#qb-push-savepath").value.trim() || undefined,
      }),
    });
    const sn = (server.name || "").trim() || server.base_url || "qB";
    showToast(`已推送到 ${sn}`, "ok");
    $("#qb-push")?.close?.();
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
});

$("#settings-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  clearErr();
  const tmdb = $("#f-tmdb").value;
  const mteam = $("#f-mteam").value;
  const qbServers = collectQbServersFromDom();
  const submitBtn = e.target.querySelector('button[type="submit"]');
  if (submitBtn) submitBtn.disabled = true;
  try {
    await api("/api/config", {
      method: "PUT",
      body: JSON.stringify({
        tmdb_api_key: tmdb,
        mteam_api_key: mteam,
        qb_servers: qbServers,
      }),
    });
    qbServersCache = qbServers;
    closeSettings();
  } catch (err) {
    showErr(err.message);
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
});

loadSettings();
