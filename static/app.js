const IMG_BASE = "https://image.tmdb.org/t/p/w342";

/** @type {{ name?: string, base_url?: string, username?: string, password?: string, insecure_tls?: boolean }[] | null} */
let qbServersCache = null;
/** @type {{ name?: string, wanted_tag?: string, qb_category?: string, qb_save_dir_name?: string, download_dir?: string, link_target_dir?: string }[] | null} */
let subscriptionCategoriesCache = null;
/** @type {{ name?: string, priority?: number, mode?: string, title_keywords?: string[], resolution_keywords?: string[], source_keywords?: string[] }[] | null} */
let torrentMatchRulesCache = null;
let subscriptionStateCache = null;
let searchSource = "tmdb";
let currentView = "search";
let currentAppPage = "main";
let doubanQrTimer = null;
let doubanQrSessionId = "";
let doubanTagHistoryCache = null;
let doubanTagHistoryPromise = null;

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

function itemImageUrl(it) {
  return it.poster_url || it.cover_url || posterUrl(it.poster_path) || "";
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

async function resolveSubscriptionCategories() {
  if (subscriptionCategoriesCache) return subscriptionCategoriesCache;
  try {
    const c = await api("/api/config");
    subscriptionCategoriesCache = Array.isArray(c.subscription_categories) ? c.subscription_categories : [];
  } catch {
    subscriptionCategoriesCache = [];
  }
  return subscriptionCategoriesCache;
}

function setSearchLoading(on, text = "正在搜索…") {
  const overlay = $("#layout-loading");
  const btn = $("#btn-search");
  const q = $("#q");
  if (overlay) {
    overlay.classList.toggle("hidden", !on);
    overlay.setAttribute("aria-hidden", on ? "false" : "true");
    const loadingText = overlay.querySelector(".loading-text");
    if (loadingText) loadingText.textContent = text;
  }
  if (btn) btn.disabled = !!on;
  if (q) q.disabled = !!on;
  $("#main-layout")?.setAttribute("aria-busy", on ? "true" : "false");
}

function setSearchSource(source) {
  searchSource = source === "douban" ? "douban" : "tmdb";
  for (const btn of document.querySelectorAll("[data-search-source]")) {
    const active = btn.dataset.searchSource === searchSource;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-pressed", active ? "true" : "false");
  }
  const q = $("#q");
  if (q) {
    q.placeholder = searchSource === "douban" ? "搜索豆瓣影视标题…" : "搜索电影或剧集标题…";
  }
}

function setResultSectionsForSource(source) {
  $("#library-bar")?.classList.add("hidden");
  const moviesTitle = $("#movies-title");
  const tvSection = $("#tv-section");
  if (source === "douban") {
    if (moviesTitle) moviesTitle.textContent = "豆瓣影视";
    if (tvSection) tvSection.classList.add("hidden");
    $("#tv").innerHTML = "";
  } else {
    if (moviesTitle) moviesTitle.textContent = "电影";
    if (tvSection) tvSection.classList.remove("hidden");
  }
}

function setResultSectionsForLibrary() {
  currentView = "douban-library";
  $("#library-bar")?.classList.remove("hidden");
  const moviesTitle = $("#movies-title");
  const tvTitle = $("#tv-title");
  const tvSection = $("#tv-section");
  if (moviesTitle) moviesTitle.textContent = "想看";
  if (tvTitle) tvTitle.textContent = "看过";
  if (tvSection) tvSection.classList.remove("hidden");
}

function setAppPage(page) {
  currentAppPage = ["main", "settings", "subscriptions"].includes(page) ? page : "main";
  for (const section of document.querySelectorAll(".app-page")) {
    const active = section.id === `page-${currentAppPage}`;
    section.classList.toggle("hidden", !active);
    section.classList.toggle("is-active", active);
  }
  for (const btn of document.querySelectorAll("[data-app-page-target]")) {
    const active = btn.dataset.appPageTarget === currentAppPage;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-current", active ? "page" : "false");
  }
  if (currentAppPage === "settings") {
    $("#detail")?.classList.add("is-off");
  }
}

async function api(path, opts = {}) {
  let r;
  try {
    r = await fetch(path, {
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      ...opts,
    });
  } catch (err) {
    const detail = err instanceof Error && err.message ? err.message : String(err);
    throw new Error(`请求未收到服务端响应：${path}。请检查服务是否仍在运行；原始错误：${detail}`);
  }
  const text = await r.text();
  let data;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { raw: text };
  }
  if (!r.ok) {
    const msg = data?.error || r.statusText || "请求失败";
    throw new Error(`${msg}（HTTP ${r.status}）`);
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
    const mediaType = it.source === "douban" ? "douban" : it.media_type || type;
    div.dataset.type = mediaType;
    div.dataset.id = String(it.id ?? it.subject_id ?? "");
    if (mediaType === "douban" && Array.isArray(it.tags)) {
      div.dataset.doubanTags = normalizeDoubanTags(it.tags.join(" "));
    }
    const img = document.createElement("img");
    img.src = itemImageUrl(it) || "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    img.alt = "";
    img.loading = "lazy";
    const meta = document.createElement("div");
    meta.className = "meta";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = it.title || "(无标题)";
    const sub = document.createElement("div");
    sub.className = "subtle";
    if (mediaType === "douban") {
      const ratingValue = it.rating?.value ?? it.vote_average;
      const libraryBits = [it.date || it.abstract_text || it.abstract || it.abstract_2 || ""].filter(Boolean);
      const normalBits = [
        it.abstract_text || it.abstract || it.abstract_2 || "",
        ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : "",
      ].filter(Boolean);
      sub.textContent = (currentView === "douban-library" ? libraryBits : normalBits).join(" · ");
    } else {
      const d = it.release_date || it.first_air_date || "";
      const ratingValue = it.vote_average;
      sub.textContent = [
        d,
        ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : "",
      ]
        .filter(Boolean)
        .join(" · ");
    }
    meta.append(title, sub);
    div.append(img, meta);
    div.addEventListener("click", () => {
      const id = div.dataset.type === "douban" ? div.dataset.id : Number(div.dataset.id);
      openDetail(div.dataset.type, id, { doubanTags: div.dataset.doubanTags || "" });
    });
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

function renderDoubanMeta(d) {
  const rows = [];
  if (d.rating?.value != null) {
    const count = d.rating.count != null ? `（${Number(d.rating.count).toLocaleString()} 人）` : "";
    rows.push(["评分", `${Number(d.rating.value).toFixed(1)} / 10${count}`]);
  }
  if (d.date_published) rows.push(["发布日期", d.date_published]);
  if (d.duration) rows.push(["片长", d.duration]);
  if (Array.isArray(d.genres) && d.genres.length) rows.push(["类型", d.genres.join(" · ")]);
  if (Array.isArray(d.directors) && d.directors.length) rows.push(["导演", d.directors.join(" · ")]);
  if (Array.isArray(d.writers) && d.writers.length) rows.push(["编剧", d.writers.join(" · ")]);
  if (Array.isArray(d.actors) && d.actors.length) rows.push(["主演", d.actors.slice(0, 10).join(" · ")]);
  if (!rows.length) return "";
  const html = rows
    .map(
      ([k, v]) =>
        `<div class="detail-meta-row"><dt>${escapeHtml(k)}</dt><dd>${escapeHtml(String(v))}</dd></div>`,
    )
    .join("");
  return `<dl class="detail-meta">${html}</dl>`;
}

function renderDoubanInterestPanel(doubanId, currentInterest = "", currentRating = null, currentTags = "") {
  if (!doubanId) return "";
  const interest = currentInterest === "wish" || currentInterest === "collect" ? currentInterest : "";
  const rating = currentRating != null ? String(currentRating) : "";
  const tags = normalizeDoubanTags(currentTags);
  return `<section class="douban-mark-panel" data-douban-id="${escapeHtml(String(doubanId))}" data-interest="${escapeHtml(interest)}" data-current-tags="${escapeHtml(tags)}">
    <div class="douban-mark-head">
      <h4>豆瓣标记</h4>
      <span class="douban-mark-status subtle" aria-live="polite">${
        interest === "wish" ? "已想看" : interest === "collect" ? "已看过" : ""
      }</span>
    </div>
    <div class="douban-mark-controls">
      <div class="douban-mark-mode" role="group" aria-label="豆瓣标记状态">
        <button type="button" class="mteam-tab${interest === "wish" ? " is-active" : ""}" data-douban-interest="wish">想看</button>
        <button type="button" class="mteam-tab${interest === "collect" ? " is-active" : ""}" data-douban-interest="collect">看过</button>
      </div>
      <label class="douban-rating-select">
        <span>评分</span>
        <select data-douban-rating>
          <option value="">未评分</option>
          <option value="5"${rating === "5" ? " selected" : ""}>5 星</option>
          <option value="4"${rating === "4" ? " selected" : ""}>4 星</option>
          <option value="3"${rating === "3" ? " selected" : ""}>3 星</option>
          <option value="2"${rating === "2" ? " selected" : ""}>2 星</option>
          <option value="1"${rating === "1" ? " selected" : ""}>1 星</option>
        </select>
      </label>
      <button type="button" class="btn btn-mini primary" data-douban-mark-save>保存</button>
    </div>
    <label class="douban-tag-input">
      <span data-douban-tag-label>标签</span>
      <select data-douban-category></select>
      <input type="text" data-douban-tags autocomplete="off" spellcheck="false" placeholder="可选，例如：想补、冷门、家人一起看" value="${escapeHtml(tags)}" />
    </label>
    <div class="douban-tag-history hidden" data-douban-tag-history aria-live="polite"></div>
  </section>`;
}

function normalizeDoubanTags(value) {
  return String(value || "")
    .split(/\s+/)
    .map((x) => x.trim())
    .filter(Boolean)
    .join(" ");
}

function mergeDoubanTagText(current, tag) {
  const next = normalizeDoubanTags(tag);
  if (!next) return normalizeDoubanTags(current);
  const parts = normalizeDoubanTags(current).split(/\s+/).filter(Boolean);
  if (!parts.includes(next)) parts.push(next);
  return parts.join(" ");
}

async function loadDoubanTagHistory(forceRefresh = false) {
  if (doubanTagHistoryCache && !forceRefresh) return doubanTagHistoryCache;
  if (doubanTagHistoryPromise && !forceRefresh) return doubanTagHistoryPromise;
  const params = new URLSearchParams({ limit: "80" });
  if (forceRefresh) params.set("force_refresh", "true");
  doubanTagHistoryPromise = api(`/api/douban/tags?${params}`)
    .then((data) => {
      doubanTagHistoryCache = Array.isArray(data?.tags) ? data.tags.filter(Boolean) : [];
      return doubanTagHistoryCache;
    })
    .finally(() => {
      doubanTagHistoryPromise = null;
    });
  return doubanTagHistoryPromise;
}

function rememberDoubanTags(tagsText) {
  const tags = normalizeDoubanTags(tagsText).split(/\s+/).filter(Boolean);
  if (!tags.length) return;
  const allowed = new Set((subscriptionCategoriesCache || []).map((category) => String(category.wanted_tag || "").trim()).filter(Boolean));
  if (!allowed.size) return;
  const allowedTags = tags.filter((tag) => allowed.has(tag));
  if (!allowedTags.length) return;
  const existing = Array.isArray(doubanTagHistoryCache) ? doubanTagHistoryCache : [];
  doubanTagHistoryCache = [...allowedTags, ...existing.filter((tag) => !allowedTags.includes(tag))];
}

function renderDoubanTagHistory(root, tags) {
  const box = root?.querySelector("[data-douban-tag-history]");
  if (!box) return;
  const visible = Array.isArray(tags) ? tags.slice(0, 24) : [];
  if (!visible.length) {
    box.innerHTML = "";
    box.classList.add("hidden");
    return;
  }
  box.classList.remove("hidden");
  box.innerHTML = visible
    .map(
      (tag) =>
        `<button type="button" class="douban-tag-chip" data-douban-tag-suggestion="${escapeHtml(tag)}">${escapeHtml(tag)}</button>`,
    )
    .join("");
}

function setupDoubanTagHistory(root) {
  const box = root?.querySelector("[data-douban-tag-history]");
  const input = root?.querySelector("[data-douban-tags]");
  const select = root?.querySelector("[data-douban-category]");
  if (!box || (!input && !select)) return;
  box.addEventListener("click", (event) => {
    const btn = event.target.closest("[data-douban-tag-suggestion]");
    if (!btn) return;
    const tag = btn.dataset.doubanTagSuggestion || "";
    if (root.dataset.interest === "wish" && select) {
      select.value = tag;
      select.focus();
      applyDoubanInterestState(root, "wish");
    } else if (input) {
      input.value = mergeDoubanTagText(input.value, tag);
      input.focus();
    }
  });
  loadDoubanTagHistory()
    .then((tags) => renderDoubanTagHistory(root, tags))
    .catch(() => {
      box.innerHTML = "";
      box.classList.add("hidden");
    });
}

function renderDoubanCategorySelect(root, categories) {
  const select = root?.querySelector("[data-douban-category]");
  if (!select) return;
  const currentTags = normalizeDoubanTags(root.dataset.currentTags || "");
  const currentFirst = currentTags.split(/\s+/).filter(Boolean)[0] || "";
  const rows = Array.isArray(categories) ? categories.filter((category) => String(category.wanted_tag || "").trim()) : [];
  select.innerHTML = "";
  const blank = document.createElement("option");
  blank.value = "";
  blank.textContent = rows.length ? "选择订阅分类" : "未配置订阅分类";
  select.appendChild(blank);
  let hasCurrent = !currentFirst;
  for (const category of rows) {
    const tag = String(category.wanted_tag || "").trim();
    const opt = document.createElement("option");
    opt.value = tag;
    opt.textContent = `${category.name || tag} · ${tag}`;
    if (tag === currentFirst) hasCurrent = true;
    select.appendChild(opt);
  }
  if (!hasCurrent) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = `未配置：${currentFirst}`;
    opt.disabled = true;
    opt.selected = true;
    select.appendChild(opt);
  } else {
    select.value = currentFirst;
  }
}

function setupDoubanCategorySelect(root) {
  const select = root?.querySelector("[data-douban-category]");
  if (!select) return;
  select.addEventListener("change", () => applyDoubanInterestState(root, root.dataset.interest || ""));
  resolveSubscriptionCategories()
    .then((categories) => {
      renderDoubanCategorySelect(root, categories);
      applyDoubanInterestState(root, root.dataset.interest || "");
    })
    .catch(() => {
      renderDoubanCategorySelect(root, []);
      applyDoubanInterestState(root, root.dataset.interest || "");
    });
}

function applyDoubanInterestState(root, interest, currentRating = null) {
  if (!root) return;
  const normalized = interest === "wish" || interest === "collect" ? interest : "";
  const buttons = [...root.querySelectorAll("[data-douban-interest]")];
  const rating = root.querySelector("[data-douban-rating]");
  const ratingLabel = rating?.closest(".douban-rating-select");
  const tagLabel = root.querySelector("[data-douban-tag-label]");
  const tagInput = root.querySelector("[data-douban-tags]");
  const categorySelect = root.querySelector("[data-douban-category]");
  const save = root.querySelector("[data-douban-mark-save]");
  root.dataset.interest = normalized;
  for (const btn of buttons) {
    const active = btn.dataset.doubanInterest === normalized;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-pressed", active ? "true" : "false");
  }
  if (rating) {
    rating.disabled = normalized !== "collect";
    ratingLabel?.classList.toggle("hidden", normalized !== "collect");
    if (normalized === "collect" && currentRating != null) {
      rating.value = String(currentRating);
    } else if (normalized !== "collect") {
      rating.value = "";
    }
  }
  if (tagLabel) tagLabel.textContent = normalized === "wish" ? "订阅分类" : "标签";
  if (categorySelect) {
    const hasCategoryOption = [...categorySelect.options].some((option) => option.value && !option.disabled);
    categorySelect.classList.toggle("hidden", normalized !== "wish");
    categorySelect.disabled = normalized !== "wish" || !hasCategoryOption;
  }
  if (tagInput) {
    tagInput.classList.toggle("hidden", normalized === "wish");
    tagInput.disabled = normalized === "wish";
  }
  if (save) save.disabled = !normalized || (normalized === "wish" && !categorySelect?.value);
}

function setupDoubanInterestPanel(root) {
  if (!root) return;
  const buttons = [...root.querySelectorAll("[data-douban-interest]")];
  const rating = root.querySelector("[data-douban-rating]");
  const tags = root.querySelector("[data-douban-tags]");
  const category = root.querySelector("[data-douban-category]");
  const save = root.querySelector("[data-douban-mark-save]");
  const status = root.querySelector(".douban-mark-status");

  buttons.forEach((btn) => {
    btn.addEventListener("click", () => {
      applyDoubanInterestState(root, btn.dataset.doubanInterest || "");
      if (status) status.textContent = "";
    });
  });
  applyDoubanInterestState(root, root.dataset.interest || "", rating?.value || null);
  setupDoubanCategorySelect(root);
  setupDoubanTagHistory(root);

  save?.addEventListener("click", async () => {
    const doubanId = root.dataset.doubanId || "";
    const interest = root.dataset.interest || "";
    if (!doubanId) return;
    if (!interest) {
      if (status) status.textContent = "请选择想看或看过";
      return;
    }
    if (save) save.disabled = true;
    if (status) status.textContent = "保存中…";
    try {
      const ratingValue = rating && !rating.disabled && rating.value ? Number(rating.value) : undefined;
      const tagsValue = interest === "wish" ? normalizeDoubanTags(category?.value || "") : normalizeDoubanTags(tags?.value || "");
      if (interest === "wish" && !tagsValue) {
        throw new Error("请选择订阅分类");
      }
      await api(`/api/douban/subject/${encodeURIComponent(doubanId)}/interest`, {
        method: "POST",
        body: JSON.stringify({
          interest,
          rating: ratingValue,
          tags: tagsValue,
        }),
      });
      if (status) status.textContent = interest === "wish" ? "已标记想看" : "已标记看过";
      rememberDoubanTags(tagsValue);
      showToast(status?.textContent || "已保存", "ok");
      if (currentView === "douban-library") {
        if (status) status.textContent = "已保存，正在刷新列表…";
        loadDoubanLibrary(true).catch(() => {});
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (status) status.textContent = msg;
      showToast(msg, "err");
    } finally {
      if (save) save.disabled = false;
    }
  });
}

async function hydrateDoubanInterestPanel(root) {
  if (!root?.dataset?.doubanId) return;
  const status = root.querySelector(".douban-mark-status");
  if (status && !status.textContent) status.textContent = "读取豆瓣状态…";
  try {
    const data = await api(`/api/douban/subject/${encodeURIComponent(root.dataset.doubanId)}`);
    applyDoubanInterestState(root, data.user_interest || "", data.user_rating ?? null);
    if (status) {
      status.textContent =
        data.user_interest === "wish"
          ? "已想看"
          : data.user_interest === "collect"
            ? "已看过"
            : "";
    }
  } catch (err) {
    if (status && status.textContent === "读取豆瓣状态…") status.textContent = "";
  }
}

async function openDoubanDetail(id, options = {}) {
  clearErr();
  const drawer = $("#detail");
  const body = $("#detail-body");
  drawer.classList.remove("is-off");
  body.innerHTML = `
    <div class="detail-loading" role="status">
      <div class="spinner" aria-hidden="true"></div>
      <p>加载详情…</p>
    </div>`;

  let d;
  try {
    d = await api(`/api/douban/subject/${encodeURIComponent(id)}`);
  } catch (e) {
    body.innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(e.message)}</p>`;
    showErr(e.message);
    return;
  }

  const title = d.title || "";
  const doubanId = d.subject_id || d.id || id;
  const doubanUrl = d.url || `https://movie.douban.com/subject/${doubanId}/`;
  const metaHtml = renderDoubanMeta(d);
  const externalIdsHtml = `<div class="detail-external-ids"><a class="detail-ext-link tag" href="${escapeHtml(doubanUrl)}" target="_blank" rel="noopener noreferrer">豆瓣 · ${escapeHtml(String(doubanId))}</a></div>`;
  const currentTags =
    options.doubanTags ||
    (Array.isArray(d.tags) ? normalizeDoubanTags(d.tags.join(" ")) : normalizeDoubanTags(d.tags || ""));
  const doubanMarkHtml = renderDoubanInterestPanel(doubanId, d.user_interest || "", d.user_rating, currentTags);

  body.innerHTML = `
    <div class="d-head">
      ${
        d.poster_url || d.image
          ? `<img src="${escapeHtml(d.poster_url || d.image)}" alt="" />`
          : ""
      }
      <h3>${escapeHtml(title)}</h3>
      <div class="detail-type-line">
        <span class="tag">豆瓣</span>
      </div>
      ${externalIdsHtml}
      ${doubanMarkHtml}
      ${metaHtml}
      <p class="overview">${escapeHtml(d.summary || "")}</p>
      <div class="row-actions mteam-actions">
        <span class="mteam-actions-label subtle">M-Team</span>
        <div id="mteam-tablist" class="mteam-tablist" role="tablist" aria-label="M-Team 检索路径"></div>
      </div>
      <div id="torrent-box" class="torrent-list"></div>
    </div>`;

  setupDoubanInterestPanel(body.querySelector(".douban-mark-panel"));

  const tablist = $("#mteam-tablist");
  const box = $("#torrent-box");
  if (tablist && box) {
    setupMteamSourceTabs(tablist, box, { doubanId, keyword: title });
  }
}

async function openDetail(mediaType, id, options = {}) {
  if (mediaType === "douban") {
    await openDoubanDetail(id, options);
    return;
  }

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
    const doubanMarkHtml = renderDoubanInterestPanel(doubanId);
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
      ${doubanMarkHtml}
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

    const doubanPanel = body.querySelector(".douban-mark-panel");
    setupDoubanInterestPanel(doubanPanel);
    hydrateDoubanInterestPanel(doubanPanel).catch(() => {});

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

function subscriptionCategoryPayloadFromRow(row) {
  return {
    name: row.querySelector('[data-sub-cat="name"]')?.value?.trim() ?? "",
    wanted_tag: row.querySelector('[data-sub-cat="wanted_tag"]')?.value?.trim() ?? "",
    qb_category: row.querySelector('[data-sub-cat="qb_category"]')?.value?.trim() ?? "",
    qb_save_dir_name: row.querySelector('[data-sub-cat="qb_save_dir_name"]')?.value?.trim() ?? "",
    download_dir: row.querySelector('[data-sub-cat="download_dir"]')?.value?.trim() ?? "",
    link_target_dir: row.querySelector('[data-sub-cat="link_target_dir"]')?.value?.trim() ?? "",
  };
}

function categoryPayloadHasAnyValue(category) {
  return Object.values(category).some((value) => String(value || "").trim() !== "");
}

function makeSubscriptionCategoryRowEl(category = {}) {
  const row = document.createElement("div");
  row.className = "subscription-category-row";
  row.innerHTML = `
    <label>分类名<input type="text" data-sub-cat="name" placeholder="如 电影" /></label>
    <label>想看文本<input type="text" data-sub-cat="wanted_tag" placeholder="如 电影" /></label>
    <label>qB 下载分类<input type="text" data-sub-cat="qb_category" placeholder="如 movie" /></label>
    <label>qB 保存目录名<input type="text" data-sub-cat="qb_save_dir_name" placeholder="如 movies" /></label>
    <label>真实下载目录<input type="text" data-sub-cat="download_dir" placeholder="/downloads/movies" /></label>
    <label>硬链接目标目录<input type="text" data-sub-cat="link_target_dir" placeholder="/media/movies" /></label>
    <div class="subscription-category-actions">
      <p class="hint subtle">修改想看文本后，已有订阅记录可能仍保留旧文本；后续状态迁移需按订阅记录处理。</p>
      <button type="button" class="btn btn-mini subscription-category-remove">移除</button>
    </div>`;

  row.querySelector('[data-sub-cat="name"]').value = category.name ?? "";
  row.querySelector('[data-sub-cat="wanted_tag"]').value = category.wanted_tag ?? "";
  row.querySelector('[data-sub-cat="qb_category"]').value = category.qb_category ?? "";
  row.querySelector('[data-sub-cat="qb_save_dir_name"]').value = category.qb_save_dir_name ?? "";
  row.querySelector('[data-sub-cat="download_dir"]').value = category.download_dir ?? "";
  row.querySelector('[data-sub-cat="link_target_dir"]').value = category.link_target_dir ?? "";

  row.querySelector(".subscription-category-remove")?.addEventListener("click", () => {
    row.remove();
    const list = $("#subscription-categories-list");
    if (list && !list.querySelector(".subscription-category-row")) {
      list.innerHTML = '<p class="subtle subscription-category-empty">未配置订阅分类，可点下方「添加分类」</p>';
    }
  });
  return row;
}

function renderSubscriptionCategoriesEditor(categories) {
  const list = $("#subscription-categories-list");
  if (!list) return;
  list.innerHTML = "";
  const arr = Array.isArray(categories) ? categories : [];
  if (!arr.length) {
    list.innerHTML = '<p class="subtle subscription-category-empty">未配置订阅分类，可点下方「添加分类」</p>';
    return;
  }
  for (const category of arr) {
    list.appendChild(makeSubscriptionCategoryRowEl(category));
  }
}

function collectSubscriptionCategoriesFromDom() {
  const list = $("#subscription-categories-list");
  if (!list) return [];
  return [...list.querySelectorAll(".subscription-category-row")]
    .map((row) => subscriptionCategoryPayloadFromRow(row))
    .filter(categoryPayloadHasAnyValue);
}

function splitKeywordList(value) {
  return String(value || "")
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function joinKeywordList(values) {
  return Array.isArray(values) ? values.filter(Boolean).join(", ") : "";
}

function torrentRulePayloadFromRow(row) {
  return {
    name: row.querySelector('[data-torrent-rule="name"]')?.value?.trim() ?? "",
    priority: Number(row.querySelector('[data-torrent-rule="priority"]')?.value || 0) || 0,
    mode: row.querySelector('[data-torrent-rule="mode"]')?.value === "any" ? "any" : "all",
    title_keywords: splitKeywordList(row.querySelector('[data-torrent-rule="title_keywords"]')?.value),
    resolution_keywords: splitKeywordList(row.querySelector('[data-torrent-rule="resolution_keywords"]')?.value),
    source_keywords: splitKeywordList(row.querySelector('[data-torrent-rule="source_keywords"]')?.value),
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

function makeTorrentRuleRowEl(rule = {}) {
  const row = document.createElement("div");
  row.className = "torrent-rule-row";
  row.innerHTML = `
    <label>规则名<input type="text" data-torrent-rule="name" placeholder="如 优先 2160p BluRay" /></label>
    <label>优先级<input type="number" data-torrent-rule="priority" step="1" placeholder="100" /></label>
    <label>匹配模式
      <select data-torrent-rule="mode">
        <option value="all">全部满足</option>
        <option value="any">任一满足</option>
      </select>
    </label>
    <label>标题关键词<input type="text" data-torrent-rule="title_keywords" placeholder="2160p, 4K" /></label>
    <label>分辨率关键词<input type="text" data-torrent-rule="resolution_keywords" placeholder="1080p, 2160p" /></label>
    <label>版本/来源关键词<input type="text" data-torrent-rule="source_keywords" placeholder="BluRay, REMUX, WEB-DL" /></label>
    <div class="torrent-rule-actions">
      <p class="hint subtle">保存后自动订阅推送会按优先级生成可解释的候选匹配结果。</p>
      <button type="button" class="btn btn-mini torrent-rule-remove">移除</button>
    </div>`;

  row.querySelector('[data-torrent-rule="name"]').value = rule.name ?? "";
  row.querySelector('[data-torrent-rule="priority"]').value = Number.isFinite(Number(rule.priority))
    ? String(Number(rule.priority))
    : "0";
  row.querySelector('[data-torrent-rule="mode"]').value = rule.mode === "any" ? "any" : "all";
  row.querySelector('[data-torrent-rule="title_keywords"]').value = joinKeywordList(rule.title_keywords);
  row.querySelector('[data-torrent-rule="resolution_keywords"]').value = joinKeywordList(rule.resolution_keywords);
  row.querySelector('[data-torrent-rule="source_keywords"]').value = joinKeywordList(rule.source_keywords);

  row.querySelector(".torrent-rule-remove")?.addEventListener("click", () => {
    row.remove();
    const list = $("#torrent-rules-list");
    if (list && !list.querySelector(".torrent-rule-row")) {
      list.innerHTML = '<p class="subtle torrent-rule-empty">未配置规则；自动推送会使用首个候选种子。</p>';
    }
  });
  return row;
}

function renderTorrentRulesEditor(rules) {
  const list = $("#torrent-rules-list");
  if (!list) return;
  list.innerHTML = "";
  const arr = Array.isArray(rules) ? rules : [];
  if (!arr.length) {
    list.innerHTML = '<p class="subtle torrent-rule-empty">未配置规则；自动推送会使用首个候选种子。</p>';
    return;
  }
  for (const rule of arr) {
    list.appendChild(makeTorrentRuleRowEl(rule));
  }
}

function collectTorrentRulesFromDom() {
  const list = $("#torrent-rules-list");
  if (!list) return [];
  return [...list.querySelectorAll(".torrent-rule-row")]
    .map((row) => torrentRulePayloadFromRow(row))
    .filter(torrentRulePayloadHasAnyValue);
}

function clearDoubanQrTimer() {
  if (doubanQrTimer) {
    clearInterval(doubanQrTimer);
    doubanQrTimer = null;
  }
}

function resetDoubanQrUi() {
  clearDoubanQrTimer();
  doubanQrSessionId = "";
  const box = $("#douban-qr-box");
  const img = $("#douban-qr-img");
  const status = $("#douban-qr-status");
  if (box) box.classList.add("hidden");
  if (img) img.removeAttribute("src");
  if (status) status.textContent = "";
}

async function pollDoubanQrLogin() {
  if (!doubanQrSessionId) return;
  const statusEl = $("#douban-qr-status");
  try {
    const data = await api(
      `/api/douban/qr/poll?${new URLSearchParams({ session_id: doubanQrSessionId })}`,
    );
    if (statusEl) {
      statusEl.textContent = data.description || data.message || data.login_status || "等待扫码…";
    }
    if (data.done) {
      clearDoubanQrTimer();
      if (data.cookie_header) {
        $("#f-douban-cookie").value = data.cookie_header;
      }
      if (statusEl) statusEl.textContent = "已获取 Cookie";
      showToast("豆瓣 Cookie 已保存", "ok");
    }
  } catch (err) {
    clearDoubanQrTimer();
    const msg = err instanceof Error ? err.message : String(err);
    if (statusEl) statusEl.textContent = msg;
    showToast(msg, "err");
  }
}

async function startDoubanQrLogin() {
  clearErr();
  resetDoubanQrUi();
  const btn = $("#btn-douban-qr");
  const statusEl = $("#douban-qr-status");
  if (btn) btn.disabled = true;
  if (statusEl) statusEl.textContent = "正在生成二维码…";
  try {
    const data = await api("/api/douban/qr/start", { method: "POST", body: "{}" });
    doubanQrSessionId = data.session_id || "";
    if (!doubanQrSessionId || !data.image_url) {
      throw new Error("豆瓣 QR 登录响应缺少会话信息");
    }
    const box = $("#douban-qr-box");
    const img = $("#douban-qr-img");
    if (img) img.src = `${data.image_url}&t=${Date.now()}`;
    if (box) box.classList.remove("hidden");
    if (statusEl) statusEl.textContent = "等待扫码确认…";
    doubanQrTimer = setInterval(() => {
      pollDoubanQrLogin().catch(() => {});
    }, 2000);
    await pollDoubanQrLogin();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (statusEl) statusEl.textContent = msg;
    showToast(msg, "err");
  } finally {
    if (btn) btn.disabled = false;
  }
}

function formatUnixSeconds(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return "";
  return new Date(n * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function setLibraryCacheStatus(data) {
  const el = $("#library-cache-status");
  if (!el) return;
  const wishCount = Array.isArray(data?.wish?.items) ? data.wish.items.length : 0;
  const collectCount = Array.isArray(data?.collect?.items) ? data.collect.items.length : 0;
  const source = data?.cached ? "本地缓存" : "刚刚刷新";
  const fetched = formatUnixSeconds(data?.fetched_at);
  const ttl = Number(data?.ttl_seconds);
  const ttlText = Number.isFinite(ttl) && ttl > 0 ? `TTL ${Math.round(ttl / 3600)} 小时` : "";
  el.textContent = [
    source,
    fetched ? `抓取于 ${fetched}` : "",
    `想看 ${wishCount}`,
    `看过 ${collectCount}`,
    ttlText,
  ]
    .filter(Boolean)
    .join(" · ");
}

async function loadDoubanLibrary(forceRefresh = false) {
  clearErr();
  currentView = "douban-library";
  setResultSectionsForLibrary();
  setSearchLoading(true, forceRefresh ? "正在刷新豆瓣列表…" : "正在加载豆瓣列表…");
  const mainBtn = $("#btn-douban-library");
  const refreshBtn = $("#btn-douban-library-refresh");
  if (mainBtn) mainBtn.disabled = true;
  if (refreshBtn) refreshBtn.disabled = true;
  try {
    const params = new URLSearchParams({ limit: "200" });
    if (forceRefresh) params.set("force_refresh", "true");
    const data = await api(`/api/douban/library?${params}`);
    renderCards($("#movies"), data?.wish?.items || [], "douban");
    renderCards($("#tv"), data?.collect?.items || [], "douban");
    setLibraryCacheStatus(data);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    showErr(msg);
    $("#movies").innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(msg)}</p>`;
    $("#tv").innerHTML = "";
    const status = $("#library-cache-status");
    if (status) status.textContent = "";
  } finally {
    setSearchLoading(false);
    if (mainBtn) mainBtn.disabled = false;
    if (refreshBtn) refreshBtn.disabled = false;
  }
}

function subscriptionRecordsFromState(state) {
  const records = state?.records && typeof state.records === "object" ? Object.values(state.records) : [];
  return records.sort((a, b) => Number(b.updated_at || 0) - Number(a.updated_at || 0));
}

const SUB_STATUS_LABELS = {
  unprocessed: "待处理",
  matching: "匹配中",
  processing: "处理中",
  pushed: "下载中",
  downloading: "下载中",
  completed: "已完成",
  linked: "已链接",
  failed: "失败",
  skipped: "已跳过",
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
};

function normalizedStatus(value) {
  return String(value || "").trim().toLowerCase();
}

function subscriptionDisplayStatus(record) {
  const base = normalizedStatus(record?.status);
  const push = normalizedStatus(record?.last_push?.status);
  const completion = normalizedStatus(record?.last_completion?.status);
  if (base === "skipped") return { key: "skipped", text: SUB_STATUS_LABELS.skipped };
  if (base === "linked" || push === "linked" || completion === "completed") return { key: "linked", text: SUB_STATUS_LABELS.linked };
  if (base === "completed") return { key: "completed", text: SUB_STATUS_LABELS.completed };
  if (base === "failed" || push === "failed" || completion === "failed") return { key: "failed", text: SUB_STATUS_LABELS.failed };
  if (push === "downloaded") return { key: "downloaded", text: "已下载待链接" };
  if (push === "downloading" || base === "downloading" || base === "pushed") return { key: "pushed", text: SUB_STATUS_LABELS.pushed };
  if (base === "processing") return { key: "processing", text: SUB_STATUS_LABELS.processing };
  return { key: base || "unprocessed", text: SUB_STATUS_LABELS[base] || "待处理" };
}

function formatPercent(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return "";
  return `${Math.round(Math.max(0, Math.min(1, n)) * 100)}%`;
}

function formatBytes(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let idx = 0;
  while (v >= 1024 && idx < units.length - 1) {
    v /= 1024;
    idx += 1;
  }
  return `${v >= 10 || idx === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[idx]}`;
}

function subscriptionProgress(record) {
  const push = record?.last_push;
  if (!push) return null;
  const progress = Number(push.download_progress);
  if (Number.isFinite(progress)) return Math.max(0, Math.min(1, progress));
  if (normalizedStatus(record?.status) === "completed") return 1;
  return null;
}

function progressBarHtml(progress) {
  if (progress == null) return "";
  const pct = Math.max(0, Math.min(1, Number(progress)));
  return `<div class="subscription-progress" aria-label="下载进度 ${formatPercent(pct)}"><span style="width:${Math.round(pct * 100)}%"></span></div>`;
}

function renderSubscriptionSummary(state) {
  const el = $("#subscription-summary");
  if (!el) return;
  const records = subscriptionRecordsFromState(state);
  const counts = records.reduce((acc, record) => {
    const key = subscriptionDisplayStatus(record).key;
    acc[key] = (acc[key] || 0) + 1;
    return acc;
  }, {});
  const bits = [
    `总计 ${records.length}`,
    counts.unprocessed ? `待处理 ${counts.unprocessed}` : "",
    counts.pushed ? `下载中 ${counts.pushed}` : "",
    counts.downloaded ? `待链接 ${counts.downloaded}` : "",
    counts.completed ? `完成 ${counts.completed}` : "",
    counts.failed ? `失败 ${counts.failed}` : "",
    counts.skipped ? `跳过 ${counts.skipped}` : "",
    state?.last_poll_at ? `上次轮询 ${formatUnixSeconds(state.last_poll_at)}` : "",
  ].filter(Boolean);
  el.textContent = bits.join(" · ");
}

function renderSubscriptionCards(state) {
  const mount = $("#subscription-list");
  if (!mount) return;
  const records = subscriptionRecordsFromState(state);
  renderSubscriptionSummary(state);
  if (!records.length) {
    mount.innerHTML = '<p class="empty-hint">暂无订阅记录</p>';
    return;
  }
  mount.innerHTML = records
    .map((record) => {
      const status = subscriptionDisplayStatus(record);
      const progress = subscriptionProgress(record);
      const push = record.last_push || {};
      const completion = record.last_completion || {};
      const meta = [
        record.release_year || "",
        record.category_text || "",
        push.qb_category ? `qB ${push.qb_category}` : "",
        push.download_state || "",
        record.updated_at ? `更新 ${formatUnixSeconds(record.updated_at)}` : "",
      ].filter(Boolean);
      const sub = completion.error || push.error || record.last_error || record.skip_reason || "";
      const episodeCount = Array.isArray(push.episodes) && push.episodes.length
        ? `<span class="subscription-episode-count">${push.episodes.length} 集</span>`
        : "";
      return `<article class="subscription-card" data-subscription-id="${escapeHtml(record.subject_id)}">
        <div class="subscription-card-head">
          <h2>${escapeHtml(record.title || record.subject_id)}</h2>
          <span class="subscription-status subscription-status-${escapeHtml(status.key)}">${escapeHtml(status.text)}</span>
        </div>
        <div class="subscription-card-meta">${meta.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}${episodeCount}</div>
        ${progressBarHtml(progress)}
        ${progress != null ? `<div class="subscription-card-progress">${escapeHtml(formatPercent(progress))}</div>` : ""}
        ${sub ? `<p class="subscription-card-note">${escapeHtml(sub)}</p>` : ""}
      </article>`;
    })
    .join("");
}

function subscriptionPollToast(outcome) {
  if (!outcome || typeof outcome !== "object") return "订阅轮询完成";
  const parts = [
    `新增待处理 ${Number(outcome.created_unprocessed || 0)}`,
    `跳过旧想看 ${Number(outcome.created_skipped || 0)}`,
    `更新已有 ${Number(outcome.updated_existing || 0)}`,
  ];
  return `订阅轮询完成：${parts.join(" · ")}`;
}

async function loadSubscriptions({ poll = false, silent = false } = {}) {
  clearErr();
  const refreshBtn = $("#btn-subscription-refresh");
  const pollBtn = $("#btn-subscription-poll");
  if (refreshBtn) refreshBtn.disabled = true;
  if (pollBtn) pollBtn.disabled = true;
  try {
    let pollOutcome = null;
    if (poll) {
      pollOutcome = await api("/api/subscriptions/wanted/poll", { method: "POST", body: "{}" });
    }
    const state = await api("/api/subscriptions/wanted");
    subscriptionStateCache = state;
    renderSubscriptionCards(state);
    if (!silent) showToast(poll ? subscriptionPollToast(pollOutcome) : "本地订阅列表已刷新", "ok");
    return state;
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    const detail = `${poll ? "轮询想看失败" : "刷新本地订阅列表失败"}：${msg}`;
    showErr(detail);
    const mount = $("#subscription-list");
    if (mount && !subscriptionRecordsFromState(subscriptionStateCache).length) {
      mount.innerHTML = `<p class="empty-hint">加载失败：${escapeHtml(detail)}</p>`;
    }
    throw new Error(detail);
  } finally {
    if (refreshBtn) refreshBtn.disabled = false;
    if (pollBtn) pollBtn.disabled = false;
  }
}

function findCachedSubscriptionRecord(id) {
  const records = subscriptionRecordsFromState(subscriptionStateCache);
  return records.find((record) => String(record.subject_id) === String(id));
}

function detailRow(label, value) {
  if (value == null || String(value).trim() === "") return "";
  return `<div class="detail-meta-row"><dt>${escapeHtml(label)}</dt><dd>${escapeHtml(String(value))}</dd></div>`;
}

function renderSubscriptionEpisodeRows(episodes = []) {
  if (!Array.isArray(episodes) || !episodes.length) return "";
  return `<section class="subscription-detail-section">
    <h4>分集</h4>
    <div class="subscription-episode-list">
      ${episodes
        .map(
          (ep) => `<div class="subscription-episode-row">
            <span class="subscription-episode-title">${escapeHtml(ep.label || "未识别分集")}</span>
            <span class="subscription-episode-state">${escapeHtml(PUSH_STATUS_LABELS[normalizedStatus(ep.status)] || ep.status || "")}</span>
            ${progressBarHtml(ep.progress)}
            <span class="subscription-episode-files">${escapeHtml(`${ep.completed_file_count || ep.linked_file_count || 0}/${ep.file_count || 0}`)}</span>
          </div>`,
        )
        .join("")}
    </div>
  </section>`;
}

function renderSubscriptionFileRows(files = [], mode = "progress") {
  if (!Array.isArray(files) || !files.length) return "";
  return `<section class="subscription-detail-section">
    <h4>${mode === "links" ? "硬链接结果" : "文件"}</h4>
    <div class="subscription-file-list">
      ${files
        .slice(0, 80)
        .map((file) => {
          const progress = mode === "links" ? null : Number(file.progress);
          const status = mode === "links" ? file.status : `${formatPercent(progress)}${file.episode_label ? ` · ${file.episode_label}` : ""}`;
          const name = mode === "links" ? file.target_path || file.source_path : file.name;
          const note = file.error || (mode === "links" ? file.source_path : formatBytes(file.size));
          return `<div class="subscription-file-row">
            <div class="subscription-file-main">
              <span class="subscription-file-name">${escapeHtml(name || "")}</span>
              ${note ? `<span class="subscription-file-note">${escapeHtml(note)}</span>` : ""}
            </div>
            <span class="subscription-file-status">${escapeHtml(status || "")}</span>
          </div>`;
        })
        .join("")}
    </div>
  </section>`;
}

function renderSubscriptionDetail(record) {
  const status = subscriptionDisplayStatus(record);
  const push = record.last_push || null;
  const completion = record.last_completion || null;
  const progress = subscriptionProgress(record);
  const metaRows = [
    detailRow("豆瓣 ID", record.subject_id),
    detailRow("分类文本", record.category_text),
    detailRow("上映年份", record.release_year),
    detailRow("状态", status.text),
    detailRow("重试", `${record.retry_count || 0}/${record.max_retries || 0}`),
    detailRow("首次看到", formatUnixSeconds(record.first_seen_at)),
    detailRow("最近更新", formatUnixSeconds(record.updated_at)),
  ].join("");
  const pushRows = push
    ? [
        detailRow("种子", push.torrent_title),
        detailRow("qB", push.qb_server),
        detailRow("分类", push.qb_category),
        detailRow("保存目录", push.qb_save_dir_name),
        detailRow("qB 状态", push.download_state || PUSH_STATUS_LABELS[normalizedStatus(push.status)] || push.status),
        detailRow("qB hash", push.qb_hash),
        detailRow("文件", push.total_file_count != null ? `${push.completed_file_count || 0}/${push.total_file_count}` : ""),
        detailRow("大小", formatBytes(push.total_size)),
        detailRow("检查时间", formatUnixSeconds(push.checked_at)),
      ].join("")
    : "";
  const downloadSection = pushRows
    ? `<section class="subscription-detail-section"><h4>下载</h4><dl class="detail-meta">${pushRows}</dl></section>`
    : '<section class="subscription-detail-section"><h4>下载</h4><p class="empty-hint">尚未推送，暂无下载进度</p></section>';
  const completionRows = completion
    ? [
        detailRow("链接状态", PUSH_STATUS_LABELS[normalizedStatus(completion.status)] || completion.status),
        detailRow("目标目录", completion.target_dir),
        detailRow("源目录", completion.source_path),
        detailRow("完成时间", formatUnixSeconds(completion.completed_at)),
        detailRow("错误", completion.error),
      ].join("")
    : "";
  return `<article class="subscription-detail" data-subscription-detail-id="${escapeHtml(record.subject_id)}">
    <div class="subscription-detail-head">
      <h3>${escapeHtml(record.title || record.subject_id)}</h3>
      <span class="subscription-status subscription-status-${escapeHtml(status.key)}">${escapeHtml(status.text)}</span>
    </div>
    ${progressBarHtml(progress)}
    <dl class="detail-meta">${metaRows}</dl>
    <div class="row-actions">
      ${push ? `<button type="button" class="btn secondary" data-sub-action="progress" data-subscription-id="${escapeHtml(record.subject_id)}">刷新下载进度</button>` : ""}
      ${push ? `<button type="button" class="btn primary" data-sub-action="completion" data-subscription-id="${escapeHtml(record.subject_id)}">检查完成并硬链接</button>` : ""}
    </div>
    ${record.last_error ? `<p class="subscription-detail-error">${escapeHtml(record.last_error)}</p>` : ""}
    ${downloadSection}
    ${renderSubscriptionEpisodeRows((completion?.episodes?.length ? completion.episodes : push?.episodes) || [])}
    ${completionRows ? `<section class="subscription-detail-section"><h4>硬链接</h4><dl class="detail-meta">${completionRows}</dl></section>` : ""}
    ${renderSubscriptionFileRows(push?.files || [])}
    ${renderSubscriptionFileRows(completion?.linked_files || push?.linked_files || [], "links")}
  </article>`;
}

function openSubscriptionDetail(record) {
  const detail = $("#detail");
  const body = $("#detail-body");
  if (!detail || !body) return;
  body.innerHTML = renderSubscriptionDetail(record);
  detail.classList.remove("is-off");
}

async function refreshSubscriptionProgress(id) {
  const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/progress`, {
    method: "POST",
    body: "{}",
  });
  subscriptionStateCache = await api("/api/subscriptions/wanted");
  renderSubscriptionCards(subscriptionStateCache);
  openSubscriptionDetail(data.record || findCachedSubscriptionRecord(id));
  showToast("下载进度已刷新", "ok");
}

async function checkSubscriptionCompletion(id) {
  const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/completion`, {
    method: "POST",
    body: JSON.stringify({ dry_run: false }),
  });
  subscriptionStateCache = await api("/api/subscriptions/wanted");
  renderSubscriptionCards(subscriptionStateCache);
  openSubscriptionDetail(data.record || findCachedSubscriptionRecord(id));
  showToast(data.completed ? "硬链接完成" : "下载尚未完成", "ok");
}

async function runSearch() {
  clearErr();
  currentView = "search";
  const q = $("#q").value.trim();
  if (!q) {
    showErr("请输入搜索词");
    return;
  }
  const source = searchSource;
  setResultSectionsForSource(source);
  setSearchLoading(true, source === "douban" ? "正在搜索豆瓣…" : "正在搜索 TMDB…");
  try {
    if (source === "douban") {
      const data = await api(`/api/douban/search?${new URLSearchParams({ q, limit: "20" })}`);
      renderCards($("#movies"), data.items || data.movies || [], "douban");
    } else {
      const data = await api(`/api/search?${new URLSearchParams({ q })}`);
      renderCards($("#movies"), data.movies || [], "movie");
      renderCards($("#tv"), data.tv || [], "tv");
    }
  } catch (e) {
    showErr(e.message);
  } finally {
    setSearchLoading(false);
  }
}

function openSettings() {
  setAppPage("settings");
}

function closeSettings() {
  resetDoubanQrUi();
  setAppPage("main");
}

async function loadSettings() {
  try {
    const c = await api("/api/config");
    $("#f-tmdb").value = c.tmdb_api_key || "";
    $("#f-mteam").value = c.mteam_api_key || "";
    $("#f-douban-cookie").value = c.douban_cookie || "";
    renderSubscriptionCategoriesEditor(c.subscription_categories);
    subscriptionCategoriesCache = Array.isArray(c.subscription_categories) ? c.subscription_categories : [];
    renderTorrentRulesEditor(c.torrent_match_rules);
    torrentMatchRulesCache = Array.isArray(c.torrent_match_rules) ? c.torrent_match_rules : [];
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
$("#btn-douban-library")?.addEventListener("click", () => {
  setAppPage("main");
  loadDoubanLibrary(true).catch(() => {});
});
$("#btn-douban-library-refresh")?.addEventListener("click", () => {
  loadDoubanLibrary(true).catch(() => {});
});
document.querySelectorAll("[data-app-page-target]").forEach((btn) => {
  btn.addEventListener("click", async () => {
    if (btn.dataset.appPageTarget === "settings") {
      await loadSettings();
      openSettings();
    } else if (btn.dataset.appPageTarget === "subscriptions") {
      resetDoubanQrUi();
      setAppPage("subscriptions");
      loadSubscriptions({ silent: true }).catch(() => {});
    } else {
      closeSettings();
    }
  });
});
$("#btn-cancel-settings").addEventListener("click", closeSettings);

$("#btn-subscription-refresh")?.addEventListener("click", () => {
  loadSubscriptions().catch(() => {});
});

$("#btn-subscription-poll")?.addEventListener("click", () => {
  loadSubscriptions({ poll: true }).catch(() => {});
});

$("#subscription-list")?.addEventListener("click", (event) => {
  const card = event.target.closest("[data-subscription-id]");
  if (!card) return;
  const record = findCachedSubscriptionRecord(card.dataset.subscriptionId || "");
  if (record) openSubscriptionDetail(record);
});

$("#detail-body")?.addEventListener("click", (event) => {
  const action = event.target.closest("[data-sub-action]");
  if (!action) return;
  const id = action.dataset.subscriptionId || "";
  if (!id) return;
  const buttons = [...document.querySelectorAll("[data-sub-action]")];
  buttons.forEach((btn) => (btn.disabled = true));
  const run =
    action.dataset.subAction === "progress"
      ? refreshSubscriptionProgress(id)
      : checkSubscriptionCompletion(id);
  run
    .catch((err) => showToast(err instanceof Error ? err.message : String(err), "err"))
    .finally(() => {
      buttons.forEach((btn) => (btn.disabled = false));
    });
});

$("#btn-qb-add")?.addEventListener("click", () => {
  const list = $("#qb-servers-list");
  if (!list) return;
  list.querySelector(".qb-empty")?.remove();
  list.appendChild(makeQbServerRowEl({}));
});

$("#btn-subscription-category-add")?.addEventListener("click", () => {
  const list = $("#subscription-categories-list");
  if (!list) return;
  list.querySelector(".subscription-category-empty")?.remove();
  list.appendChild(makeSubscriptionCategoryRowEl({}));
});

$("#btn-torrent-rule-add")?.addEventListener("click", () => {
  const list = $("#torrent-rules-list");
  if (!list) return;
  list.querySelector(".torrent-rule-empty")?.remove();
  list.appendChild(makeTorrentRuleRowEl({}));
});

document.querySelectorAll("[data-search-source]").forEach((btn) => {
  btn.addEventListener("click", () => setSearchSource(btn.dataset.searchSource));
});

$("#btn-douban-qr")?.addEventListener("click", () => {
  startDoubanQrLogin().catch(() => {});
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
  const doubanCookie = $("#f-douban-cookie").value;
  const qbServers = collectQbServersFromDom();
  const subscriptionCategories = collectSubscriptionCategoriesFromDom();
  const torrentMatchRules = collectTorrentRulesFromDom();
  const submitBtn = e.target.querySelector('button[type="submit"]');
  if (submitBtn) submitBtn.disabled = true;
  try {
    await api("/api/config", {
      method: "PUT",
      body: JSON.stringify({
        tmdb_api_key: tmdb,
        mteam_api_key: mteam,
        douban_cookie: doubanCookie,
        qb_servers: qbServers,
        subscription_categories: subscriptionCategories,
        torrent_match_rules: torrentMatchRules,
      }),
    });
    qbServersCache = qbServers;
    subscriptionCategoriesCache = subscriptionCategories;
    torrentMatchRulesCache = torrentMatchRules;
    doubanTagHistoryCache = null;
    closeSettings();
  } catch (err) {
    showErr(err.message);
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
});

setSearchSource("tmdb");
loadSettings();
