<template>
  <div class="app-shell">
    <aside class="side-nav menu bg-base-100 text-base-content" aria-label="主导航">
      <div class="brand">
        <h1>影视检索</h1>
        <p class="sub">TMDB / 豆瓣 / M-Team</p>
      </div>
      <nav class="nav-list" aria-label="页面">
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': page === 'main' }"
          @click="go('main')"
        >
          主功能
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': page === 'subscriptions' }"
          @click="go('subscriptions')"
        >
          订阅
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': page === 'settings' }"
          @click="go('settings')"
        >
          设置
        </button>
      </nav>
    </aside>

    <div class="app-content">
      <div v-if="error" id="err" class="banner err alert alert-error" role="alert">{{ error }}</div>
      <div
        v-if="toast.message"
        id="toast"
        class="toast alert"
        :class="toast.kind === 'err' ? 'toast-err alert-error' : 'toast-ok alert-success'"
        role="status"
        aria-live="polite"
      >
        {{ toast.message }}
      </div>

      <section
        v-show="page === 'main'"
        id="page-main"
        class="app-page"
        :class="{ 'is-active': page === 'main' }"
      >
        <header class="top">
          <h1>影视检索</h1>
          <p class="sub">TMDB / 豆瓣资料与 M-Team 种子</p>
          <div class="actions">
            <button type="button" class="btn btn-secondary" @click="loadDoubanLibrary(true)">
              豆瓣列表
            </button>
          </div>
        </header>

        <section class="search-bar">
          <div class="source-switch tabs tabs-boxed" role="group" aria-label="搜索来源">
            <button
              type="button"
              class="source-pill tab"
              :class="{ 'tab-active is-active': searchSource === 'tmdb' }"
              @click="setSearchSource('tmdb')"
            >
              TMDB
            </button>
            <button
              type="button"
              class="source-pill tab"
              :class="{ 'tab-active is-active': searchSource === 'douban' }"
              @click="setSearchSource('douban')"
            >
              豆瓣
            </button>
          </div>
          <input
            v-model.trim="query"
            type="search"
            class="input input-bordered"
            :placeholder="searchSource === 'douban' ? '搜索豆瓣影视标题…' : '搜索电影或剧集标题…'"
            autocomplete="off"
            @keydown.enter="runSearch"
          />
          <button
            type="button"
            class="btn btn-primary"
            :disabled="searchLoading"
            @click="runSearch"
          >
            搜索
          </button>
        </section>

        <section
          v-if="currentView === 'douban-library'"
          id="library-bar"
          class="library-bar"
          aria-live="polite"
        >
          <div>
            <h2>豆瓣列表</h2>
            <p class="hint">{{ libraryCacheStatus }}</p>
          </div>
          <button
            type="button"
            class="btn btn-secondary"
            :disabled="searchLoading"
            @click="loadDoubanLibrary(true)"
          >
            刷新缓存
          </button>
        </section>

        <main
          class="layout"
          id="main-layout"
          aria-live="polite"
          :aria-busy="searchLoading ? 'true' : 'false'"
        >
          <div v-if="searchLoading" class="layout-loading" role="status">
            <div class="spinner" aria-hidden="true"></div>
            <span class="loading-text">{{ searchLoadingText }}</span>
          </div>
          <section id="movies-section">
            <h2 id="movies-title">
              {{
                currentView === "douban-library"
                  ? "想看"
                  : searchSource === "douban"
                    ? "豆瓣影视"
                    : "电影"
              }}
            </h2>
            <div class="grid">
              <p v-if="!movies.length" class="empty-hint">无结果</p>
              <article
                v-for="item in movies"
                :key="cardKey(item, 'movie')"
                class="card media-card bg-base-100 border border-base-300 shadow-sm"
                @click="openCardDetail(item, 'movie')"
              >
                <img :src="itemImageUrl(item) || transparentPixel" alt="" loading="lazy" />
                <div class="meta">
                  <div class="title">{{ item.title || "(无标题)" }}</div>
                  <div class="subtle">{{ cardSubtitle(item) }}</div>
                </div>
              </article>
            </div>
          </section>
          <section
            v-show="searchSource !== 'douban' || currentView === 'douban-library'"
            id="tv-section"
          >
            <h2 id="tv-title">{{ currentView === "douban-library" ? "看过" : "剧集" }}</h2>
            <div class="grid">
              <p v-if="!tv.length" class="empty-hint">无结果</p>
              <article
                v-for="item in tv"
                :key="cardKey(item, 'tv')"
                class="card media-card bg-base-100 border border-base-300 shadow-sm"
                @click="openCardDetail(item, 'tv')"
              >
                <img :src="itemImageUrl(item) || transparentPixel" alt="" loading="lazy" />
                <div class="meta">
                  <div class="title">{{ item.title || item.name || "(无标题)" }}</div>
                  <div class="subtle">{{ cardSubtitle(item) }}</div>
                </div>
              </article>
            </div>
          </section>
        </main>
      </section>

      <section
        v-show="page === 'subscriptions'"
        id="page-subscriptions"
        class="app-page"
        :class="{ 'is-active': page === 'subscriptions' }"
      >
        <header class="top subscriptions-top">
          <h1>订阅</h1>
          <p class="sub">想看订阅、下载进度与硬链接结果</p>
          <div class="actions">
            <button
              type="button"
              class="btn btn-secondary"
              :disabled="subscriptionsLoading"
              @click="loadSubscriptions({ poll: true })"
            >
              轮询想看
            </button>
            <button
              type="button"
              class="btn btn-secondary"
              :disabled="subscriptionsLoading"
              title="只重新读取本地已保存的订阅状态"
              @click="loadSubscriptions()"
            >
              刷新本地列表
            </button>
          </div>
        </header>
        <section class="subscription-toolbar" aria-live="polite">
          <p class="hint">{{ subscriptionSummary }}</p>
        </section>
        <section id="subscription-list" class="subscription-list" aria-live="polite">
          <p v-if="!subscriptionRecords.length" class="empty-hint">暂无订阅记录</p>
          <article
            v-for="record in subscriptionRecords"
            :key="record.subject_id"
            class="subscription-card card bg-base-100 border border-base-300 shadow-sm"
            @click="openSubscriptionDetail(record)"
          >
            <div class="subscription-card-head">
              <h2>{{ record.title || record.subject_id }}</h2>
              <span
                class="subscription-status badge"
                :class="`subscription-status-${subscriptionDisplayStatus(record).key}`"
                >{{ subscriptionDisplayStatus(record).text }}</span
              >
            </div>
            <div class="subscription-card-meta">
              <span v-for="meta in subscriptionCardMeta(record)" :key="meta">{{ meta }}</span>
              <span v-if="record.last_push?.episodes?.length" class="subscription-episode-count"
                >{{ record.last_push.episodes.length }} 集</span
              >
            </div>
            <div
              v-if="subscriptionProgress(record) != null"
              class="subscription-progress"
              :aria-label="`下载进度 ${formatPercent(subscriptionProgress(record))}`"
            >
              <span :style="{ width: `${Math.round(subscriptionProgress(record) * 100)}%` }"></span>
            </div>
            <div v-if="subscriptionProgress(record) != null" class="subscription-card-progress">
              {{ formatPercent(subscriptionProgress(record)) }}
            </div>
            <p v-if="subscriptionCardNote(record)" class="subscription-card-note">
              {{ subscriptionCardNote(record) }}
            </p>
          </article>
        </section>
      </section>

      <section
        v-show="page === 'settings'"
        id="page-settings"
        class="app-page"
        :class="{ 'is-active': page === 'settings' }"
      >
        <header class="top settings-top">
          <h1>设置</h1>
          <p class="sub">API、豆瓣登录、订阅分类与 qBittorrent</p>
        </header>

        <form id="settings-form" class="settings-page-form" @submit.prevent="saveSettings">
          <section class="settings-section card bg-base-100 border border-base-300">
            <h2>API 密钥</h2>
            <p class="hint">
              将写入运行目录下的 <code>config.toml</code>（或通过环境变量
              <code>CONFIG_PATH</code> 指定路径）。
            </p>
            <label
              >TMDB API Key<input
                v-model="settings.tmdb_api_key"
                type="password"
                class="input input-bordered"
                autocomplete="off"
            /></label>
            <label
              >M-Team OpenAPI Key<input
                v-model="settings.mteam_api_key"
                type="password"
                class="input input-bordered"
                autocomplete="off"
            /></label>
            <label
              >豆瓣 Cookie<textarea
                v-model="settings.douban_cookie"
                class="textarea textarea-bordered"
                rows="3"
                autocomplete="off"
                spellcheck="false"
                placeholder="dbcl2=...; ck=..."
              ></textarea>
            </label>
            <div class="douban-login-tools">
              <button
                type="button"
                class="btn btn-secondary"
                :disabled="qrLoading"
                @click="startDoubanQrLogin"
              >
                QR 登录获取 Cookie
              </button>
              <span class="hint subtle" aria-live="polite">{{ doubanQrStatus }}</span>
            </div>
            <div v-if="doubanQrImage" class="douban-qr-box">
              <img :src="doubanQrImage" alt="豆瓣登录二维码" />
            </div>
          </section>

          <section
            class="settings-section subscription-categories-fieldset card bg-base-100 border border-base-300"
          >
            <h2>订阅分类</h2>
            <p class="hint">
              “想看”只能选择这里配置的文本；分类保存后会写入配置文件，后续自动下载与硬链接使用同一组字段。
            </p>
            <div class="subscription-categories-list">
              <p
                v-if="!settings.subscription_categories.length"
                class="subtle subscription-category-empty"
              >
                未配置订阅分类，可点下方「添加分类」
              </p>
              <div
                v-for="(category, idx) in settings.subscription_categories"
                :key="idx"
                class="subscription-category-row"
              >
                <label
                  >分类名<input
                    v-model="category.name"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 电影"
                /></label>
                <label
                  >想看文本<input
                    v-model="category.wanted_tag"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 电影"
                /></label>
                <label
                  >qB 下载分类<input
                    v-model="category.qb_category"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 movie"
                /></label>
                <label
                  >qB 保存目录名<input
                    v-model="category.qb_save_dir_name"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 movies"
                /></label>
                <label
                  >真实下载目录<input
                    v-model="category.download_dir"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="/downloads/movies"
                /></label>
                <label
                  >硬链接目标目录<input
                    v-model="category.link_target_dir"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="/media/movies"
                /></label>
                <div class="subscription-category-actions">
                  <p class="hint subtle">
                    修改想看文本后，已有订阅记录可能仍保留旧文本；后续状态迁移需按订阅记录处理。
                  </p>
                  <button
                    type="button"
                    class="btn btn-sm btn-ghost"
                    @click="settings.subscription_categories.splice(idx, 1)"
                  >
                    移除
                  </button>
                </div>
              </div>
            </div>
            <button type="button" class="btn btn-secondary" @click="addSubscriptionCategory">
              添加分类
            </button>
          </section>

          <section
            class="settings-section torrent-rules-fieldset card bg-base-100 border border-base-300"
          >
            <h2>种子匹配规则</h2>
            <p class="hint">
              数字越大越先尝试；高优先级没有候选命中时才尝试低优先级。关键词用逗号分隔。
            </p>
            <div class="torrent-rules-list">
              <p v-if="!settings.torrent_match_rules.length" class="subtle torrent-rule-empty">
                未配置规则；自动推送会使用首个候选种子。
              </p>
              <div
                v-for="(rule, idx) in settings.torrent_match_rules"
                :key="idx"
                class="torrent-rule-row"
              >
                <label
                  >规则名<input
                    v-model="rule.name"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 优先 2160p BluRay"
                /></label>
                <label
                  >优先级<input
                    v-model.number="rule.priority"
                    type="number"
                    class="input input-bordered input-sm"
                    step="1"
                    placeholder="100"
                /></label>
                <label
                  >匹配模式<select v-model="rule.mode" class="select select-bordered select-sm">
                    <option value="all">全部满足</option>
                    <option value="any">任一满足</option>
                  </select></label
                >
                <label
                  >标题关键词<input
                    v-model="rule.title_keywords_text"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="2160p, 4K"
                /></label>
                <label
                  >分辨率关键词<input
                    v-model="rule.resolution_keywords_text"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="1080p, 2160p"
                /></label>
                <label
                  >版本/来源关键词<input
                    v-model="rule.source_keywords_text"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="BluRay, REMUX, WEB-DL"
                /></label>
                <div class="torrent-rule-actions">
                  <p class="hint subtle">保存后自动订阅推送会按优先级生成可解释的候选匹配结果。</p>
                  <button
                    type="button"
                    class="btn btn-sm btn-ghost"
                    @click="settings.torrent_match_rules.splice(idx, 1)"
                  >
                    移除
                  </button>
                </div>
              </div>
            </div>
            <button type="button" class="btn btn-secondary" @click="addTorrentRule">
              添加规则
            </button>
          </section>

          <section
            class="settings-section qb-servers-fieldset card bg-base-100 border border-base-300"
          >
            <h2>qBittorrent</h2>
            <p class="hint">
              在本机可访问的 qB Web UI；保存后会写入配置文件，下次打开设置会从此处加载。
            </p>
            <div class="qb-servers-list">
              <p v-if="!settings.qb_servers.length" class="subtle qb-empty">
                未配置 qB 服务器，可点下方「添加」
              </p>
              <div v-for="(server, idx) in settings.qb_servers" :key="idx" class="qb-server-row">
                <label
                  >显示名<input
                    v-model="server.name"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="如 家用 NAS"
                /></label>
                <label
                  >Web UI 根地址<input
                    v-model="server.base_url"
                    type="text"
                    class="input input-bordered input-sm"
                    placeholder="http://127.0.0.1:8080"
                /></label>
                <label
                  >用户名<input
                    v-model="server.username"
                    type="text"
                    class="input input-bordered input-sm"
                    autocomplete="off"
                /></label>
                <label
                  >密码<input
                    v-model="server.password"
                    type="password"
                    class="input input-bordered input-sm"
                    autocomplete="off"
                /></label>
                <div class="qb-row-actions">
                  <label class="qb-insecure"
                    ><input
                      v-model="server.insecure_tls"
                      type="checkbox"
                      class="checkbox checkbox-sm"
                    />
                    忽略 HTTPS 证书错误</label
                  >
                  <div class="qb-row-tail">
                    <button
                      type="button"
                      class="btn btn-sm btn-secondary"
                      :disabled="server.testing"
                      @click="testQbServer(server)"
                    >
                      测试连接
                    </button>
                    <span
                      class="qb-test-msg"
                      :class="
                        server.testKind === 'err'
                          ? 'qb-test-msg-error'
                          : server.testKind === 'ok'
                            ? 'qb-test-msg-ok'
                            : 'subtle'
                      "
                      aria-live="polite"
                      >{{ server.testMessage }}</span
                    >
                    <button
                      type="button"
                      class="btn btn-sm btn-ghost"
                      @click="settings.qb_servers.splice(idx, 1)"
                    >
                      移除
                    </button>
                  </div>
                </div>
              </div>
            </div>
            <button type="button" class="btn btn-secondary" @click="addQbServer">添加服务器</button>
          </section>

          <div class="form-actions">
            <p
              id="settings-save-status"
              class="form-status"
              :class="settingsStatus.kind ? `is-${settingsStatus.kind}` : ''"
              role="status"
              aria-live="polite"
            >
              {{ settingsStatus.message }}
            </p>
            <button type="submit" class="btn btn-primary" :disabled="savingSettings">
              保存设置
            </button>
          </div>
        </form>
      </section>
    </div>
  </div>

  <aside id="detail" class="drawer" :class="{ 'is-off': !detailOpen }">
    <button type="button" class="close" aria-label="关闭" @click="closeDetail">×</button>
    <div id="detail-body">
      <div v-if="detailLoading" class="detail-loading" role="status">
        <div class="spinner" aria-hidden="true"></div>
        <p>加载详情…</p>
      </div>
      <p v-else-if="detailError" class="empty-hint">加载失败：{{ detailError }}</p>

      <article v-else-if="detailKind === 'media' && detailData" class="d-head">
        <img v-if="detailPoster" :src="detailPoster" alt="" />
        <h3>{{ detailTitle }}</h3>
        <div class="detail-type-line">
          <span class="tag">{{
            detailMediaType === "douban" ? "豆瓣" : detailMediaType === "tv" ? "剧集" : "电影"
          }}</span>
          <span v-if="detailDate" class="tag">{{ detailDate }}</span>
        </div>
        <div v-if="externalLinks.length" class="detail-external-ids">
          <a
            v-for="link in externalLinks"
            :key="link.href"
            class="detail-ext-link tag"
            :href="link.href"
            target="_blank"
            rel="noopener noreferrer"
            >{{ link.label }}</a
          >
        </div>
        <section v-if="detailDoubanId" class="douban-mark-panel">
          <div class="douban-mark-head">
            <h4>豆瓣标记</h4>
            <span class="douban-mark-status subtle" aria-live="polite">{{
              doubanInterestStatus
            }}</span>
          </div>
          <div class="douban-mark-controls">
            <div class="douban-mark-mode tabs tabs-boxed" role="group" aria-label="豆瓣标记状态">
              <button
                type="button"
                class="mteam-tab tab"
                :class="{ 'is-active tab-active': doubanMark.interest === 'wish' }"
                @click="setDoubanInterest('wish')"
              >
                想看
              </button>
              <button
                type="button"
                class="mteam-tab tab"
                :class="{ 'is-active tab-active': doubanMark.interest === 'collect' }"
                @click="setDoubanInterest('collect')"
              >
                看过
              </button>
            </div>
            <label v-if="doubanMark.interest === 'collect'" class="douban-rating-select">
              <span>评分</span>
              <select v-model="doubanMark.rating" class="select select-bordered select-sm">
                <option value="">未评分</option>
                <option value="5">5 星</option>
                <option value="4">4 星</option>
                <option value="3">3 星</option>
                <option value="2">2 星</option>
                <option value="1">1 星</option>
              </select>
            </label>
            <button
              type="button"
              class="btn btn-sm btn-primary"
              :disabled="doubanSaveDisabled"
              @click="saveDoubanInterest"
            >
              保存
            </button>
          </div>
          <label class="douban-tag-input">
            <span>{{ doubanMark.interest === "wish" ? "订阅分类" : "标签" }}</span>
            <select
              v-if="doubanMark.interest === 'wish'"
              v-model="doubanMark.category"
              class="select select-bordered select-sm"
            >
              <option value="">
                {{ subscriptionCategoriesCache.length ? "选择订阅分类" : "未配置订阅分类" }}
              </option>
              <option
                v-for="category in subscriptionCategoriesCache"
                :key="category.wanted_tag"
                :value="category.wanted_tag"
              >
                {{ category.name || category.wanted_tag }} · {{ category.wanted_tag }}
              </option>
            </select>
            <input
              v-else
              v-model="doubanMark.tags"
              type="text"
              class="input input-bordered input-sm"
              autocomplete="off"
              spellcheck="false"
              placeholder="可选，例如：想补、冷门、家人一起看"
            />
          </label>
          <div v-if="doubanTagHistory.length" class="douban-tag-history" aria-live="polite">
            <button
              v-for="tag in doubanTagHistory.slice(0, 24)"
              :key="tag"
              type="button"
              class="douban-tag-chip"
              @click="applyDoubanTagSuggestion(tag)"
            >
              {{ tag }}
            </button>
          </div>
        </section>

        <p v-if="detailData.tagline" class="tagline-block">{{ detailData.tagline }}</p>
        <dl v-if="detailMetaRows.length" class="detail-meta">
          <div v-for="row in detailMetaRows" :key="row.label" class="detail-meta-row">
            <dt>{{ row.label }}</dt>
            <dd>{{ row.value }}</dd>
          </div>
        </dl>

        <div v-if="detailMediaType === 'tv'" class="tv-seasons-mount">
          <h4 class="tv-episodes-heading">分集</h4>
          <p v-if="!detailSeasons.length" class="subtle">暂无分季信息</p>
          <div v-else class="tv-seasons-list" role="list">
            <details
              v-for="season in detailSeasons"
              :key="season.season_number"
              class="tv-season-block"
              @toggle="loadSeasonEpisodes($event, season.season_number)"
            >
              <summary class="tv-season-summary">
                <span class="tv-season-label"
                  >第 {{ season.season_number }} 季{{
                    season.name ? ` · ${season.name}` : ""
                  }}</span
                >
                <span v-if="season.episode_count != null" class="tv-season-meta subtle"
                  >{{ season.episode_count }} 集</span
                >
              </summary>
              <div class="tv-season-body">
                <div
                  v-if="seasonLoading[season.season_number]"
                  class="inline-loading tv-season-loading"
                  role="status"
                >
                  <div class="spinner spinner-sm"></div>
                  <span>加载中…</span>
                </div>
                <p v-else-if="seasonErrors[season.season_number]" class="empty-hint">
                  加载失败：{{ seasonErrors[season.season_number] }}
                </p>
                <p
                  v-else-if="!seasonEpisodes[season.season_number]"
                  class="subtle tv-season-placeholder"
                >
                  展开以加载本季分集…
                </p>
                <template v-else>
                  <p v-if="!seasonEpisodes[season.season_number].length" class="subtle">
                    本季暂无分集数据
                  </p>
                  <div
                    v-for="ep in seasonEpisodes[season.season_number]"
                    :key="ep.episode_number"
                    class="tv-episode-row"
                  >
                    <div class="tv-ep-thumb">
                      <img
                        v-if="episodeStill(ep)"
                        class="tv-ep-still"
                        :src="episodeStill(ep)"
                        alt=""
                        loading="lazy"
                      />
                      <div
                        v-else
                        class="tv-ep-still tv-ep-still-placeholder"
                        aria-hidden="true"
                      ></div>
                    </div>
                    <div class="tv-ep-main">
                      <div class="tv-ep-title-line">
                        <span class="tv-ep-num">E{{ ep.episode_number ?? "—" }}</span>
                        <span class="tv-ep-title">{{
                          ep.name || `第 ${ep.episode_number} 集`
                        }}</span>
                        <span v-if="ep.air_date" class="tv-ep-air">{{ ep.air_date }}</span>
                      </div>
                      <p v-if="ep.overview" class="tv-ep-overview">{{ ep.overview }}</p>
                    </div>
                  </div>
                </template>
              </div>
            </details>
          </div>
        </div>

        <p class="overview">{{ detailOverview }}</p>
        <div class="row-actions mteam-actions">
          <template v-if="mteamSources.length">
            <span class="mteam-actions-label subtle">M-Team</span>
            <div class="mteam-tablist tabs tabs-boxed" role="tablist" aria-label="M-Team 检索路径">
              <button
                v-for="source in mteamSources"
                :key="source.source"
                type="button"
                class="mteam-tab tab"
                :class="{ 'is-active tab-active': activeTorrentSource === source.source }"
                role="tab"
                @click="selectTorrentSource(source.source)"
              >
                {{ source.label }}
              </button>
            </div>
          </template>
          <span v-else class="subtle">缺少 IMDb / 豆瓣 ID，且无原标题，无法在 M-Team 检索</span>
        </div>
        <div class="torrent-list">
          <div v-if="torrentsLoading" class="inline-loading" role="status">
            <div class="spinner spinner-sm"></div>
            <span>正在加载 M-Team…</span>
          </div>
          <p v-else-if="torrentError" class="empty-hint">加载失败：{{ torrentError }}</p>
          <template v-else-if="torrentRows.length">
            <h4 class="torrent-list-title">M-Team 种子</h4>
            <div class="torrent-cards">
              <article
                v-for="torrent in torrentRows"
                :key="torrent.id || torrent.name"
                class="torrent-card"
              >
                <div class="torrent-card-inner">
                  <a
                    class="torrent-card-link"
                    :href="mteamTorrentWebUrl(torrent.id)"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    <div class="torrent-name">
                      {{ torrent.name || torrent.title || torrent.id || "(无标题)" }}
                    </div>
                    <div class="torrent-stats">{{ torrentStats(torrent) }}</div>
                    <div class="torrent-desc">
                      {{ torrent.smallDescr || torrent.small_descr || "" }}
                    </div>
                  </a>
                  <div class="torrent-card-actions">
                    <button
                      v-if="torrent.id"
                      type="button"
                      class="btn btn-sm btn-primary torrent-push-trigger"
                      @click.prevent.stop="openQbPushDialog(torrent)"
                    >
                      推送 qB
                    </button>
                    <span v-else class="subtle torrent-push-hint" title="无种子 ID，无法推送"
                      >—</span
                    >
                  </div>
                </div>
              </article>
            </div>
          </template>
          <p v-else-if="activeTorrentSource" class="empty-hint">
            未返回种子列表（请检查 M-Team 返回结构或账号权限）。
          </p>
        </div>
      </article>

      <article
        v-else-if="detailKind === 'subscription' && selectedSubscription"
        class="subscription-detail"
        :data-subscription-detail-id="selectedSubscription.subject_id"
      >
        <div class="subscription-detail-head">
          <h3>{{ selectedSubscription.title || selectedSubscription.subject_id }}</h3>
          <span
            class="subscription-status badge"
            :class="`subscription-status-${subscriptionDisplayStatus(selectedSubscription).key}`"
            >{{ subscriptionDisplayStatus(selectedSubscription).text }}</span
          >
        </div>
        <div
          v-if="subscriptionProgress(selectedSubscription) != null"
          class="subscription-progress"
          :aria-label="`下载进度 ${formatPercent(subscriptionProgress(selectedSubscription))}`"
        >
          <span
            :style="{ width: `${Math.round(subscriptionProgress(selectedSubscription) * 100)}%` }"
          ></span>
        </div>
        <dl class="detail-meta">
          <div
            v-for="row in subscriptionDetailRows(selectedSubscription)"
            :key="row.label"
            class="detail-meta-row"
          >
            <dt>{{ row.label }}</dt>
            <dd>{{ row.value }}</dd>
          </div>
        </dl>
        <div class="row-actions">
          <button
            v-if="selectedSubscription.last_push"
            type="button"
            class="btn btn-secondary"
            :disabled="subscriptionActionLoading"
            @click="refreshSubscriptionProgress(selectedSubscription.subject_id)"
          >
            刷新下载进度
          </button>
          <button
            v-if="selectedSubscription.last_push"
            type="button"
            class="btn btn-primary"
            :disabled="subscriptionActionLoading"
            @click="checkSubscriptionCompletion(selectedSubscription.subject_id)"
          >
            检查完成并硬链接
          </button>
        </div>
        <p v-if="selectedSubscription.last_error" class="subscription-detail-error">
          {{ selectedSubscription.last_error }}
        </p>
        <section class="subscription-detail-section">
          <h4>下载</h4>
          <dl v-if="selectedSubscription.last_push" class="detail-meta">
            <div
              v-for="row in pushRows(selectedSubscription.last_push)"
              :key="row.label"
              class="detail-meta-row"
            >
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            </div>
          </dl>
          <p v-else class="empty-hint">尚未推送，暂无下载进度</p>
        </section>
        <section v-if="subscriptionEpisodes.length" class="subscription-detail-section">
          <h4>分集</h4>
          <div class="subscription-episode-list">
            <div
              v-for="ep in subscriptionEpisodes"
              :key="ep.label || ep.episode_number"
              class="subscription-episode-row"
            >
              <span class="subscription-episode-title">{{ ep.label || "未识别分集" }}</span>
              <span class="subscription-episode-state">{{ pushStatusLabel(ep.status) }}</span>
              <div v-if="ep.progress != null" class="subscription-progress">
                <span :style="{ width: `${Math.round(Number(ep.progress) * 100)}%` }"></span>
              </div>
              <span class="subscription-episode-files"
                >{{ ep.completed_file_count || ep.linked_file_count || 0 }}/{{
                  ep.file_count || 0
                }}</span
              >
            </div>
          </div>
        </section>
        <section v-if="selectedSubscription.last_completion" class="subscription-detail-section">
          <h4>硬链接</h4>
          <dl class="detail-meta">
            <div
              v-for="row in completionRows(selectedSubscription.last_completion)"
              :key="row.label"
              class="detail-meta-row"
            >
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            </div>
          </dl>
        </section>
        <section v-if="subscriptionFiles.length" class="subscription-detail-section">
          <h4>文件</h4>
          <div class="subscription-file-list">
            <div
              v-for="file in subscriptionFiles"
              :key="file.name || file.target_path || file.source_path"
              class="subscription-file-row"
            >
              <div class="subscription-file-main">
                <span class="subscription-file-name">{{
                  file.name || file.target_path || file.source_path
                }}</span>
                <span
                  v-if="file.error || file.source_path || file.size"
                  class="subscription-file-note"
                  >{{ file.error || file.source_path || formatBytes(file.size) }}</span
                >
              </div>
              <span class="subscription-file-status">{{
                file.status || (file.progress != null ? formatPercent(file.progress) : "")
              }}</span>
            </div>
          </div>
        </section>
      </article>
    </div>
  </aside>

  <dialog class="modal" :open="qbDialogOpen">
    <form class="modal-box" @submit.prevent="submitQbPush">
      <h2>推送到 qBittorrent</h2>
      <p class="hint subtle">{{ qbPushLabel }}</p>
      <label
        >qB 服务器<select
          v-model="qbPush.serverIndex"
          class="select select-bordered"
          :disabled="!qbServersCache.length"
        >
          <option v-if="!qbServersCache.length" value="">未配置 qB（请打开 API 设置）</option>
          <option v-for="(server, idx) in qbServersCache" :key="idx" :value="String(idx)">
            {{ server.name || server.base_url || `服务器 ${idx + 1}` }}
          </option>
        </select></label
      >
      <label
        >分类（可选）<input
          v-model.trim="qbPush.category"
          type="text"
          class="input input-bordered"
          autocomplete="off"
          placeholder="留空则用 qB 默认"
      /></label>
      <label
        >保存路径（可选）<input
          v-model.trim="qbPush.savepath"
          type="text"
          class="input input-bordered"
          autocomplete="off"
          placeholder="留空则用 qB 默认保存目录"
      /></label>
      <div class="form-actions">
        <button type="button" class="btn btn-secondary" @click="qbDialogOpen = false">取消</button>
        <button type="submit" class="btn btn-primary" :disabled="qbPushLoading">确认推送</button>
      </div>
    </form>
  </dialog>
</template>

<script setup>
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";

const IMG_BASE = "https://image.tmdb.org/t/p/w342";
const transparentPixel =
  "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

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

const SUB_SKIP_REASON_LABELS = {
  initial_bootstrap_existing_wish: "历史想看，首次同步跳过",
};

const route = useRoute();
const router = useRouter();
const routeToPage = { main: "main", subscriptions: "subscriptions", settings: "settings" };

const page = computed(() => routeToPage[route.name] || "main");
const error = ref("");
const toast = reactive({ message: "", kind: "ok", timer: 0 });

const searchSource = ref("tmdb");
const query = ref("");
const currentView = ref("search");
const searchLoading = ref(false);
const searchLoadingText = ref("正在搜索 TMDB…");
const movies = ref([]);
const tv = ref([]);
const libraryCacheStatus = ref("");

const detailOpen = ref(false);
const detailLoading = ref(false);
const detailError = ref("");
const detailKind = ref("");
const detailMediaType = ref("");
const detailData = ref(null);
const selectedSubscription = ref(null);
const detailDoubanId = ref("");
const detailNumericId = ref("");
const seasonEpisodes = reactive({});
const seasonLoading = reactive({});
const seasonErrors = reactive({});

const mteamSources = ref([]);
const activeTorrentSource = ref("");
const torrentCache = reactive({});
const torrentRows = ref([]);
const torrentsLoading = ref(false);
const torrentError = ref("");

const qbDialogOpen = ref(false);
const qbPushLoading = ref(false);
const qbPush = reactive({ torrentId: "", title: "", serverIndex: "", category: "", savepath: "" });

const settings = reactive({
  tmdb_api_key: "",
  mteam_api_key: "",
  douban_cookie: "",
  qb_servers: [],
  subscription_categories: [],
  torrent_match_rules: [],
});
const settingsLoaded = ref(false);
const savingSettings = ref(false);
const settingsStatus = reactive({ message: "", kind: "" });
const qbServersCache = ref([]);
const subscriptionCategoriesCache = ref([]);
const doubanTagHistory = ref([]);
const doubanTagHistoryPromise = ref(null);
const doubanQrStatus = ref("");
const doubanQrImage = ref("");
const doubanQrSessionId = ref("");
const qrLoading = ref(false);
let doubanQrTimer = 0;

const subscriptionsLoading = ref(false);
const subscriptionState = ref(null);
const subscriptionActionLoading = ref(false);

const doubanMark = reactive({ interest: "", rating: "", tags: "", category: "", status: "" });

watch(
  () => page.value,
  (next) => {
    clearError();
    if (next === "settings") loadSettings();
    if (next === "subscriptions") loadSubscriptions({ silent: true });
    if (next !== "settings") resetDoubanQrUi();
    if (next === "settings") closeDetail();
  },
  { immediate: true },
);

onMounted(() => {
  loadSettings();
});

function go(target) {
  const path =
    target === "subscriptions" ? "/subscriptions" : target === "settings" ? "/settings" : "/";
  router.push(path);
}

async function api(path, opts = {}) {
  let response;
  try {
    response = await fetch(path, {
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      ...opts,
    });
  } catch (err) {
    const detail = err instanceof Error && err.message ? err.message : String(err);
    throw new Error(`请求未收到服务端响应：${path}。请检查服务是否仍在运行；原始错误：${detail}`);
  }
  const text = await response.text();
  let data;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { raw: text };
  }
  if (!response.ok) {
    const msg = data?.error || response.statusText || "请求失败";
    throw new Error(`${msg}（HTTP ${response.status}）`);
  }
  return data;
}

function showToast(message, kind = "ok") {
  toast.message = message;
  toast.kind = kind;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => {
    toast.message = "";
  }, 3800);
}

function showError(message) {
  error.value = message;
}

function clearError() {
  error.value = "";
}

function posterUrl(path) {
  return path ? `${IMG_BASE}${path}` : "";
}

function itemImageUrl(item) {
  return item?.poster_url || item?.cover_url || posterUrl(item?.poster_path) || "";
}

function cardKey(item, fallback) {
  return `${item.source || item.media_type || fallback}-${item.id ?? item.subject_id ?? item.title}`;
}

function cardSubtitle(item) {
  if ((item.source || item.media_type) === "douban") {
    const ratingValue = item.rating?.value ?? item.vote_average;
    const bits =
      currentView.value === "douban-library"
        ? [item.date || item.abstract_text || item.abstract || item.abstract_2 || ""]
        : [
            item.abstract_text || item.abstract || item.abstract_2 || "",
            ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : "",
          ];
    return bits.filter(Boolean).join(" · ");
  }
  const date = item.release_date || item.first_air_date || "";
  const ratingValue = item.vote_average;
  return [date, ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : ""]
    .filter(Boolean)
    .join(" · ");
}

function setSearchSource(source) {
  searchSource.value = source === "douban" ? "douban" : "tmdb";
}

function setSearchLoading(on, text = "正在搜索…") {
  searchLoading.value = on;
  searchLoadingText.value = text;
}

async function runSearch() {
  clearError();
  currentView.value = "search";
  const q = query.value.trim();
  if (!q) {
    showError("请输入搜索词");
    return;
  }
  setSearchLoading(true, searchSource.value === "douban" ? "正在搜索豆瓣…" : "正在搜索 TMDB…");
  try {
    if (searchSource.value === "douban") {
      const data = await api(`/api/douban/search?${new URLSearchParams({ q, limit: "20" })}`);
      movies.value = data.items || data.movies || [];
      tv.value = [];
    } else {
      const data = await api(`/api/search?${new URLSearchParams({ q })}`);
      movies.value = data.movies || [];
      tv.value = data.tv || [];
    }
  } catch (err) {
    showError(err instanceof Error ? err.message : String(err));
  } finally {
    setSearchLoading(false);
  }
}

function setLibraryCacheStatus(data) {
  const wishCount = Array.isArray(data?.wish?.items) ? data.wish.items.length : 0;
  const collectCount = Array.isArray(data?.collect?.items) ? data.collect.items.length : 0;
  const source = data?.cached ? "本地缓存" : "刚刚刷新";
  const fetched = formatUnixSeconds(data?.fetched_at);
  const ttl = Number(data?.ttl_seconds);
  const ttlText = Number.isFinite(ttl) && ttl > 0 ? `TTL ${Math.round(ttl / 3600)} 小时` : "";
  libraryCacheStatus.value = [
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
  clearError();
  currentView.value = "douban-library";
  setSearchLoading(true, forceRefresh ? "正在刷新豆瓣列表…" : "正在加载豆瓣列表…");
  try {
    const params = new URLSearchParams({ limit: "200" });
    if (forceRefresh) params.set("force_refresh", "true");
    const data = await api(`/api/douban/library?${params}`);
    movies.value = data?.wish?.items || [];
    tv.value = data?.collect?.items || [];
    setLibraryCacheStatus(data);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    showError(msg);
    movies.value = [];
    tv.value = [];
    libraryCacheStatus.value = "";
  } finally {
    setSearchLoading(false);
  }
}

function openCardDetail(item, fallbackType) {
  const type = item.source === "douban" ? "douban" : item.media_type || fallbackType;
  const id = type === "douban" ? (item.id ?? item.subject_id) : Number(item.id);
  const tags = Array.isArray(item.tags)
    ? normalizeDoubanTags(item.tags.join(" "))
    : normalizeDoubanTags(item.tags || "");
  openDetail(type, id, { doubanTags: tags });
}

async function openDetail(mediaType, id, options = {}) {
  clearError();
  resetDetail();
  detailOpen.value = true;
  detailLoading.value = true;
  detailKind.value = "media";
  detailMediaType.value = mediaType;
  detailNumericId.value = String(id || "");
  try {
    const path =
      mediaType === "douban"
        ? `/api/douban/subject/${encodeURIComponent(id)}`
        : mediaType === "tv"
          ? `/api/tmdb/tv/${id}`
          : `/api/tmdb/movie/${id}`;
    const data = await api(path);
    detailData.value = data;
    detailDoubanId.value =
      mediaType === "douban"
        ? String(data.subject_id || data.id || id)
        : doubanFromDetail(data) || "";
    const tags =
      options.doubanTags ||
      normalizeDoubanTags(Array.isArray(data.tags) ? data.tags.join(" ") : data.tags || "");
    initDoubanMark(data, tags);
    await setupMteamSources();
    if (detailDoubanId.value) hydrateDoubanInterestPanel();
  } catch (err) {
    detailError.value = err instanceof Error ? err.message : String(err);
    showError(detailError.value);
  } finally {
    detailLoading.value = false;
  }
}

function resetDetail() {
  detailError.value = "";
  detailKind.value = "";
  detailData.value = null;
  selectedSubscription.value = null;
  detailDoubanId.value = "";
  detailNumericId.value = "";
  mteamSources.value = [];
  activeTorrentSource.value = "";
  torrentRows.value = [];
  torrentError.value = "";
  for (const key of Object.keys(seasonEpisodes)) delete seasonEpisodes[key];
  for (const key of Object.keys(seasonLoading)) delete seasonLoading[key];
  for (const key of Object.keys(seasonErrors)) delete seasonErrors[key];
}

function closeDetail() {
  detailOpen.value = false;
}

const detailTitle = computed(() => detailData.value?.title || detailData.value?.name || "");
const detailDate = computed(
  () =>
    detailData.value?.release_date ||
    detailData.value?.first_air_date ||
    detailData.value?.date_published ||
    "",
);
const detailOverview = computed(
  () => detailData.value?.overview || detailData.value?.summary || "",
);
const detailPoster = computed(
  () =>
    detailData.value?.poster_url ||
    detailData.value?.image ||
    posterUrl(detailData.value?.poster_path),
);
const detailSeasons = computed(() =>
  Array.isArray(detailData.value?.seasons)
    ? [...detailData.value.seasons].sort((a, b) => (a.season_number ?? 0) - (b.season_number ?? 0))
    : [],
);

const detailMetaRows = computed(() => {
  const data = detailData.value;
  if (!data) return [];
  if (detailMediaType.value === "douban") return doubanMetaRows(data);
  return tmdbMetaRows(data, detailMediaType.value);
});

const externalLinks = computed(() => {
  const data = detailData.value;
  if (!data) return [];
  const links = [];
  if (detailMediaType.value !== "douban" && detailNumericId.value) {
    const p = detailMediaType.value === "tv" ? "tv" : "movie";
    links.push({
      href: `https://www.themoviedb.org/${p}/${detailNumericId.value}`,
      label: `TMDB · ${detailNumericId.value}`,
    });
  }
  const imdb = imdbFromDetail(data);
  const ih = imdbHref(imdb);
  if (imdb && ih) links.push({ href: ih, label: `IMDb · ${imdb}` });
  const dubId = detailMediaType.value === "douban" ? detailDoubanId.value : doubanFromDetail(data);
  const dubUrl =
    detailMediaType.value === "douban"
      ? data.url || `https://movie.douban.com/subject/${dubId}/`
      : doubanUrlFromDetail(data);
  if (dubId && dubUrl) links.push({ href: dubUrl, label: `豆瓣 · ${dubId}` });
  return links;
});

function tmdbMetaRows(data, mediaType) {
  const rows = [];
  if (mediaType === "tv") {
    if (data.number_of_seasons != null) rows.push(["季数", `${data.number_of_seasons} 季`]);
    if (data.number_of_episodes != null) rows.push(["总集数", `${data.number_of_episodes} 集`]);
    if (data.status) rows.push(["更新状态", TV_STATUS_ZH[data.status] || data.status]);
    if (data.first_air_date) rows.push(["首播", data.first_air_date]);
    if (data.last_air_date) rows.push(["最近播出", data.last_air_date]);
    if (Array.isArray(data.episode_run_time) && data.episode_run_time.length)
      rows.push(["单集时长", `${data.episode_run_time[0]} 分钟`]);
    const networks = joinNames(data.networks);
    if (networks) rows.push(["电视网", networks]);
    if (data.type) rows.push(["作品形态", data.type]);
  } else {
    if (data.runtime) rows.push(["片长", `${data.runtime} 分钟`]);
    if (data.status) rows.push(["状态", MOVIE_STATUS_ZH[data.status] || data.status]);
    if (data.release_date) rows.push(["上映日期", data.release_date]);
  }
  const orig = data.original_title || data.original_name;
  const loc = data.title || data.name;
  if (orig && loc && orig !== loc) rows.push(["原名", orig]);
  if (data.vote_average != null)
    rows.push([
      "评分",
      `${Number(data.vote_average).toFixed(1)} / 10${data.vote_count != null ? `（${Number(data.vote_count).toLocaleString()} 人）` : ""}`,
    ]);
  const genres = joinNames(data.genres);
  if (genres) rows.push(["类型", genres]);
  const countries =
    joinNames(data.production_countries) ||
    (Array.isArray(data.origin_country) ? data.origin_country.join(" · ") : "");
  if (countries) rows.push(["国家/地区", countries]);
  const langs =
    joinNames(data.spoken_languages, "english_name") || joinNames(data.spoken_languages, "name");
  if (langs) rows.push(["语言", langs]);
  return rows.map(([label, value]) => ({ label, value }));
}

function doubanMetaRows(data) {
  const rows = [];
  if (data.rating?.value != null)
    rows.push([
      "评分",
      `${Number(data.rating.value).toFixed(1)} / 10${data.rating.count != null ? `（${Number(data.rating.count).toLocaleString()} 人）` : ""}`,
    ]);
  if (data.date_published) rows.push(["发布日期", data.date_published]);
  if (data.duration) rows.push(["片长", data.duration]);
  if (Array.isArray(data.genres) && data.genres.length)
    rows.push(["类型", data.genres.join(" · ")]);
  if (Array.isArray(data.directors) && data.directors.length)
    rows.push(["导演", data.directors.join(" · ")]);
  if (Array.isArray(data.writers) && data.writers.length)
    rows.push(["编剧", data.writers.join(" · ")]);
  if (Array.isArray(data.actors) && data.actors.length)
    rows.push(["主演", data.actors.slice(0, 10).join(" · ")]);
  return rows.map(([label, value]) => ({ label, value }));
}

function joinNames(arr, key = "name") {
  return Array.isArray(arr)
    ? arr
        .map((x) => (typeof x === "string" ? x : x?.[key]))
        .filter(Boolean)
        .join(" · ")
    : "";
}

function imdbFromDetail(data) {
  return data?.imdb_id || data?.external_ids?.imdb_id || null;
}

function imdbHref(imdbRaw) {
  if (imdbRaw == null) return null;
  const s = String(imdbRaw).trim();
  if (!s) return null;
  const id = s.startsWith("tt") ? s : `tt${s}`;
  return `https://www.imdb.com/title/${id}/`;
}

function doubanFromDetail(data) {
  const asDigitsId = (value) => {
    if (value == null) return null;
    const s = String(value).trim();
    return /^\d+$/.test(s) ? s : null;
  };
  let id = asDigitsId(data?.douban_id);
  const ext = data?.external_ids;
  if (!id && ext) id = asDigitsId(ext.douban_id) || asDigitsId(ext.douban);
  if (!id && data?.douban_url) {
    const match = String(data.douban_url).match(/douban\.com\/subject\/(\d+)/i);
    if (match) id = match[1];
  }
  return id;
}

function doubanUrlFromDetail(data) {
  const id = doubanFromDetail(data);
  if (data?.douban_url && String(data.douban_url).trim()) return String(data.douban_url).trim();
  return id ? `https://movie.douban.com/subject/${id}/` : null;
}

async function loadSeasonEpisodes(event, seasonNumber) {
  if (!event.target.open || seasonEpisodes[seasonNumber] || seasonLoading[seasonNumber]) return;
  seasonLoading[seasonNumber] = true;
  seasonErrors[seasonNumber] = "";
  try {
    const data = await api(`/api/tmdb/tv/${detailNumericId.value}/season/${seasonNumber}`);
    seasonEpisodes[seasonNumber] = Array.isArray(data?.episodes) ? data.episodes : [];
  } catch (err) {
    seasonErrors[seasonNumber] = err instanceof Error ? err.message : String(err);
  } finally {
    seasonLoading[seasonNumber] = false;
  }
}

function episodeStill(ep) {
  if (ep?.still_url && String(ep.still_url).trim()) return String(ep.still_url).trim();
  if (ep?.still_path && String(ep.still_path).trim())
    return `https://image.tmdb.org/t/p/w185${String(ep.still_path).trim()}`;
  return "";
}

function initDoubanMark(data, tags) {
  doubanMark.interest =
    data?.user_interest === "wish" || data?.user_interest === "collect" ? data.user_interest : "";
  doubanMark.rating = data?.user_rating != null ? String(data.user_rating) : "";
  doubanMark.tags = normalizeDoubanTags(tags);
  doubanMark.category = normalizeDoubanTags(tags).split(/\s+/).filter(Boolean)[0] || "";
  doubanMark.status =
    doubanMark.interest === "wish" ? "已想看" : doubanMark.interest === "collect" ? "已看过" : "";
  loadDoubanTagHistory().catch(() => {});
}

const doubanInterestStatus = computed(() => doubanMark.status);
const doubanSaveDisabled = computed(
  () => !doubanMark.interest || (doubanMark.interest === "wish" && !doubanMark.category),
);

function setDoubanInterest(interest) {
  doubanMark.interest = interest === "wish" || interest === "collect" ? interest : "";
  doubanMark.status = "";
}

async function hydrateDoubanInterestPanel() {
  if (!detailDoubanId.value) return;
  doubanMark.status = doubanMark.status || "读取豆瓣状态…";
  try {
    const data = await api(`/api/douban/subject/${encodeURIComponent(detailDoubanId.value)}`);
    if (data.user_interest === "wish" || data.user_interest === "collect")
      doubanMark.interest = data.user_interest;
    doubanMark.rating = data.user_rating != null ? String(data.user_rating) : "";
    doubanMark.status =
      data.user_interest === "wish" ? "已想看" : data.user_interest === "collect" ? "已看过" : "";
  } catch {
    if (doubanMark.status === "读取豆瓣状态…") doubanMark.status = "";
  }
}

async function saveDoubanInterest() {
  if (!detailDoubanId.value || !doubanMark.interest) return;
  try {
    doubanMark.status = "保存中…";
    const tags =
      doubanMark.interest === "wish"
        ? normalizeDoubanTags(doubanMark.category)
        : normalizeDoubanTags(doubanMark.tags);
    if (doubanMark.interest === "wish" && !tags) throw new Error("请选择订阅分类");
    await api(`/api/douban/subject/${encodeURIComponent(detailDoubanId.value)}/interest`, {
      method: "POST",
      body: JSON.stringify({
        interest: doubanMark.interest,
        rating:
          doubanMark.interest === "collect" && doubanMark.rating
            ? Number(doubanMark.rating)
            : undefined,
        tags,
      }),
    });
    doubanMark.status = doubanMark.interest === "wish" ? "已标记想看" : "已标记看过";
    rememberDoubanTags(tags);
    showToast(doubanMark.status, "ok");
    if (currentView.value === "douban-library") loadDoubanLibrary(true).catch(() => {});
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    doubanMark.status = msg;
    showToast(msg, "err");
  }
}

function applyDoubanTagSuggestion(tag) {
  if (doubanMark.interest === "wish") {
    doubanMark.category = tag;
  } else {
    doubanMark.tags = mergeDoubanTagText(doubanMark.tags, tag);
  }
}

async function loadDoubanTagHistory(forceRefresh = false) {
  if (doubanTagHistory.value.length && !forceRefresh) return doubanTagHistory.value;
  if (doubanTagHistoryPromise.value && !forceRefresh) return doubanTagHistoryPromise.value;
  const params = new URLSearchParams({ limit: "80" });
  if (forceRefresh) params.set("force_refresh", "true");
  doubanTagHistoryPromise.value = api(`/api/douban/tags?${params}`)
    .then((data) => {
      doubanTagHistory.value = Array.isArray(data?.tags) ? data.tags.filter(Boolean) : [];
      return doubanTagHistory.value;
    })
    .finally(() => {
      doubanTagHistoryPromise.value = null;
    });
  return doubanTagHistoryPromise.value;
}

function rememberDoubanTags(tagsText) {
  const tags = normalizeDoubanTags(tagsText).split(/\s+/).filter(Boolean);
  if (!tags.length) return;
  const allowed = new Set(
    subscriptionCategoriesCache.value
      .map((category) => String(category.wanted_tag || "").trim())
      .filter(Boolean),
  );
  const allowedTags = tags.filter((tag) => allowed.has(tag));
  if (!allowedTags.length) return;
  doubanTagHistory.value = [
    ...allowedTags,
    ...doubanTagHistory.value.filter((tag) => !allowedTags.includes(tag)),
  ];
}

async function setupMteamSources() {
  const data = detailData.value;
  const imdb = imdbFromDetail(data);
  const doubanId = detailDoubanId.value;
  const keyword =
    detailMediaType.value === "douban"
      ? data?.title || ""
      : (detailMediaType.value === "tv" ? data?.original_name : data?.original_title) || "";
  const sources = [];
  if (imdb) sources.push({ source: "imdb", label: "IMDb", params: { imdb_id: imdb } });
  if (doubanId)
    sources.push({ source: "douban", label: "豆瓣 ID", params: { douban_id: doubanId } });
  if (keyword) sources.push({ source: "keyword", label: "原标题", params: { keyword } });
  mteamSources.value = sources;
  for (const key of Object.keys(torrentCache)) delete torrentCache[key];
  if (sources.length) await nextTick(() => selectTorrentSource(sources[0].source));
}

async function selectTorrentSource(source) {
  activeTorrentSource.value = source;
  torrentError.value = "";
  if (torrentCache[source]) {
    torrentRows.value = torrentCache[source];
    return;
  }
  const selected = mteamSources.value.find((item) => item.source === source);
  if (!selected) return;
  torrentsLoading.value = true;
  try {
    const params = new URLSearchParams({ source, ...selected.params });
    const data = await api(`/api/mteam/torrents?${params}`);
    const rows = extractTorrentRows(data);
    torrentCache[source] = rows;
    torrentRows.value = rows;
  } catch (err) {
    torrentError.value = err instanceof Error ? err.message : String(err);
    showError(torrentError.value);
    torrentRows.value = [];
  } finally {
    torrentsLoading.value = false;
  }
}

function extractTorrentRows(res) {
  if (!res || typeof res !== "object") return [];
  const data = res.data;
  if (Array.isArray(data)) return data;
  if (data && typeof data === "object") {
    if (Array.isArray(data.data)) return data.data;
    if (Array.isArray(data.list)) return data.list;
  }
  if (Array.isArray(res.results)) return res.results;
  return [];
}

function torrentStats(torrent) {
  const status = torrent.status || {};
  return [
    torrent.size != null ? formatSize(Number(torrent.size)) : "",
    `做种 ${status.seeders ?? torrent.seeders ?? "—"}`,
    `下载 ${status.leechers ?? torrent.leechers ?? "—"}`,
    torrent.createdDate || torrent.created_date || "",
  ]
    .filter(Boolean)
    .join(" · ");
}

function mteamTorrentWebUrl(torrentId) {
  const id = String(torrentId ?? "").trim();
  return id ? `https://kp.m-team.cc/detail/${encodeURIComponent(id)}` : "https://kp.m-team.cc/";
}

async function openQbPushDialog(torrent) {
  await resolveQbServers();
  qbPush.torrentId = String(torrent.id || "");
  qbPush.title = String(torrent.name || torrent.title || "").trim();
  qbPush.serverIndex = qbServersCache.value.length ? "0" : "";
  qbPush.category = "";
  qbPush.savepath = "";
  qbDialogOpen.value = true;
}

const qbPushLabel = computed(() =>
  qbPush.title
    ? `${qbPush.title}（${qbPush.torrentId}）`
    : qbPush.torrentId
      ? `种子 ID · ${qbPush.torrentId}`
      : "",
);

async function submitQbPush() {
  if (!qbServersCache.value.length) {
    showToast("请先在 API 设置中配置 qB 服务器", "err");
    return;
  }
  const server = qbServersCache.value[Number(qbPush.serverIndex)];
  if (!server?.base_url?.trim()) {
    showToast("所选 qB 服务器无效", "err");
    return;
  }
  qbPushLoading.value = true;
  try {
    await api("/api/qb/push-mteam", {
      method: "POST",
      body: JSON.stringify({
        server,
        torrent_id: qbPush.torrentId,
        category: qbPush.category || undefined,
        savepath: qbPush.savepath || undefined,
      }),
    });
    showToast(`已推送到 ${(server.name || "").trim() || server.base_url || "qB"}`, "ok");
    qbDialogOpen.value = false;
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    qbPushLoading.value = false;
  }
}

async function resolveQbServers() {
  if (qbServersCache.value.length) return qbServersCache.value;
  try {
    const data = await api("/api/config");
    qbServersCache.value = Array.isArray(data.qb_servers) ? data.qb_servers : [];
  } catch {
    qbServersCache.value = [];
  }
  return qbServersCache.value;
}

async function loadSettings() {
  try {
    settingsStatus.message = "";
    settingsStatus.kind = "";
    const data = await api("/api/config");
    settings.tmdb_api_key = data.tmdb_api_key || "";
    settings.mteam_api_key = data.mteam_api_key || "";
    settings.douban_cookie = data.douban_cookie || "";
    settings.qb_servers = (Array.isArray(data.qb_servers) ? data.qb_servers : []).map((server) => ({
      ...server,
      testMessage: "",
      testKind: "",
      testing: false,
    }));
    settings.subscription_categories = (
      Array.isArray(data.subscription_categories) ? data.subscription_categories : []
    ).map((category) => ({ ...category }));
    settings.torrent_match_rules = (
      Array.isArray(data.torrent_match_rules) ? data.torrent_match_rules : []
    ).map(ruleToForm);
    qbServersCache.value = settings.qb_servers;
    subscriptionCategoriesCache.value = settings.subscription_categories;
    settingsLoaded.value = true;
  } catch {
    settingsLoaded.value = false;
  }
}

function ruleToForm(rule = {}) {
  return {
    name: rule.name || "",
    priority: Number.isFinite(Number(rule.priority)) ? Number(rule.priority) : 0,
    mode: rule.mode === "any" ? "any" : "all",
    title_keywords_text: joinKeywordList(rule.title_keywords),
    resolution_keywords_text: joinKeywordList(rule.resolution_keywords),
    source_keywords_text: joinKeywordList(rule.source_keywords),
  };
}

function ruleFromForm(rule) {
  return {
    name: String(rule.name || "").trim(),
    priority: Number(rule.priority || 0) || 0,
    mode: rule.mode === "any" ? "any" : "all",
    title_keywords: splitKeywordList(rule.title_keywords_text),
    resolution_keywords: splitKeywordList(rule.resolution_keywords_text),
    source_keywords: splitKeywordList(rule.source_keywords_text),
  };
}

function addSubscriptionCategory() {
  settings.subscription_categories.push({
    name: "",
    wanted_tag: "",
    qb_category: "",
    qb_save_dir_name: "",
    download_dir: "",
    link_target_dir: "",
  });
}

function addTorrentRule() {
  settings.torrent_match_rules.push(ruleToForm({ mode: "all", priority: 0 }));
}

function addQbServer() {
  settings.qb_servers.push({
    name: "",
    base_url: "",
    username: "",
    password: "",
    insecure_tls: false,
    testMessage: "",
    testKind: "",
    testing: false,
  });
}

async function testQbServer(server) {
  if (!server.base_url) {
    server.testMessage = "请先填写 Web UI 根地址";
    server.testKind = "err";
    return;
  }
  server.testing = true;
  server.testMessage = "正在测试…";
  server.testKind = "";
  try {
    const data = await api("/api/qb/test", {
      method: "POST",
      body: JSON.stringify(qbPayload(server)),
    });
    server.testMessage = `可连通${data.version ? ` · ${data.version}` : ""}`;
    server.testKind = "ok";
  } catch (err) {
    server.testMessage = err instanceof Error ? err.message : String(err);
    server.testKind = "err";
  } finally {
    server.testing = false;
  }
}

function qbPayload(server) {
  return {
    name: String(server.name || "").trim(),
    base_url: String(server.base_url || "").trim(),
    username: String(server.username || "").trim(),
    password: server.password || "",
    insecure_tls: !!server.insecure_tls,
  };
}

async function saveSettings() {
  clearError();
  savingSettings.value = true;
  settingsStatus.message = "正在保存设置…";
  settingsStatus.kind = "pending";
  const qbServers = settings.qb_servers.map(qbPayload).filter((server) => server.base_url);
  const categories = settings.subscription_categories
    .map(subscriptionCategoryPayload)
    .filter(categoryPayloadHasAnyValue);
  const rules = settings.torrent_match_rules
    .map(ruleFromForm)
    .filter(torrentRulePayloadHasAnyValue);
  try {
    await api("/api/config", {
      method: "PUT",
      body: JSON.stringify({
        tmdb_api_key: settings.tmdb_api_key,
        mteam_api_key: settings.mteam_api_key,
        douban_cookie: settings.douban_cookie,
        qb_servers: qbServers,
        subscription_categories: categories,
        torrent_match_rules: rules,
      }),
    });
    qbServersCache.value = qbServers;
    subscriptionCategoriesCache.value = categories;
    doubanTagHistory.value = [];
    settingsStatus.message = "设置已保存";
    settingsStatus.kind = "ok";
    showToast("设置已保存", "ok");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    settingsStatus.message = `保存失败：${msg}`;
    settingsStatus.kind = "err";
    showError(`保存设置失败：${msg}`);
    showToast(`保存失败：${msg}`, "err");
  } finally {
    savingSettings.value = false;
  }
}

function subscriptionCategoryPayload(category) {
  return {
    name: String(category.name || "").trim(),
    wanted_tag: String(category.wanted_tag || "").trim(),
    qb_category: String(category.qb_category || "").trim(),
    qb_save_dir_name: String(category.qb_save_dir_name || "").trim(),
    download_dir: String(category.download_dir || "").trim(),
    link_target_dir: String(category.link_target_dir || "").trim(),
  };
}

function categoryPayloadHasAnyValue(category) {
  return Object.values(category).some((value) => String(value || "").trim() !== "");
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

function clearDoubanQrTimer() {
  if (doubanQrTimer) {
    clearInterval(doubanQrTimer);
    doubanQrTimer = 0;
  }
}

function resetDoubanQrUi() {
  clearDoubanQrTimer();
  doubanQrSessionId.value = "";
  doubanQrImage.value = "";
  doubanQrStatus.value = "";
}

async function startDoubanQrLogin() {
  clearError();
  resetDoubanQrUi();
  qrLoading.value = true;
  doubanQrStatus.value = "正在生成二维码…";
  try {
    const data = await api("/api/douban/qr/start", { method: "POST", body: "{}" });
    if (!data.session_id || !data.image_url) throw new Error("豆瓣 QR 登录响应缺少会话信息");
    doubanQrSessionId.value = data.session_id;
    doubanQrImage.value = `${data.image_url}&t=${Date.now()}`;
    doubanQrStatus.value = "等待扫码确认…";
    doubanQrTimer = setInterval(() => pollDoubanQrLogin().catch(() => {}), 2000);
    await pollDoubanQrLogin();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    doubanQrStatus.value = msg;
    showToast(msg, "err");
  } finally {
    qrLoading.value = false;
  }
}

async function pollDoubanQrLogin() {
  if (!doubanQrSessionId.value) return;
  const data = await api(
    `/api/douban/qr/poll?${new URLSearchParams({ session_id: doubanQrSessionId.value })}`,
  );
  doubanQrStatus.value = data.description || data.message || data.login_status || "等待扫码…";
  if (data.done) {
    clearDoubanQrTimer();
    if (data.cookie_header) settings.douban_cookie = data.cookie_header;
    doubanQrStatus.value = "已获取 Cookie";
    showToast("豆瓣 Cookie 已保存", "ok");
  }
}

const subscriptionRecords = computed(() => {
  const records =
    subscriptionState.value?.records && typeof subscriptionState.value.records === "object"
      ? Object.values(subscriptionState.value.records)
      : [];
  return records.sort((a, b) => Number(b.updated_at || 0) - Number(a.updated_at || 0));
});

const subscriptionSummary = computed(() => {
  const records = subscriptionRecords.value;
  if (!records.length) return "尚未加载订阅状态";
  const counts = records.reduce((acc, record) => {
    const key = subscriptionDisplayStatus(record).key;
    acc[key] = (acc[key] || 0) + 1;
    return acc;
  }, {});
  return [
    `总计 ${records.length}`,
    counts.unprocessed ? `待处理 ${counts.unprocessed}` : "",
    counts.pushed ? `下载中 ${counts.pushed}` : "",
    counts.downloaded ? `待链接 ${counts.downloaded}` : "",
    counts.completed ? `完成 ${counts.completed}` : "",
    counts.failed ? `失败 ${counts.failed}` : "",
    counts.skipped ? `跳过 ${counts.skipped}` : "",
    subscriptionState.value?.last_poll_at
      ? `上次轮询 ${formatUnixSeconds(subscriptionState.value.last_poll_at)}`
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
});

async function loadSubscriptions({ poll = false, silent = false } = {}) {
  clearError();
  subscriptionsLoading.value = true;
  try {
    let pollOutcome = null;
    if (poll)
      pollOutcome = await api("/api/subscriptions/wanted/poll", { method: "POST", body: "{}" });
    subscriptionState.value = await api("/api/subscriptions/wanted");
    if (!silent) showToast(poll ? subscriptionPollToast(pollOutcome) : "本地订阅列表已刷新", "ok");
    return subscriptionState.value;
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    const detail = `${poll ? "轮询想看失败" : "刷新本地订阅列表失败"}：${msg}`;
    showError(detail);
    if (!silent) showToast(detail, "err");
    return null;
  } finally {
    subscriptionsLoading.value = false;
  }
}

function subscriptionPollToast(outcome) {
  if (!outcome || typeof outcome !== "object") return "订阅轮询完成";
  return `订阅轮询完成：新增待处理 ${Number(outcome.created_unprocessed || 0)} · 跳过旧想看 ${Number(outcome.created_skipped || 0)} · 更新已有 ${Number(outcome.updated_existing || 0)}`;
}

function normalizedStatus(value) {
  return String(value || "")
    .trim()
    .toLowerCase();
}

function formatSubscriptionSkipReason(value) {
  const raw = String(value || "").trim();
  if (!raw) return "";
  const mapped = SUB_SKIP_REASON_LABELS[raw] || SUB_SKIP_REASON_LABELS[raw.toLowerCase()];
  if (mapped) return mapped;
  if (/^[a-z0-9_-]+$/i.test(raw)) return `跳过原因：${raw.replace(/[_-]+/g, " ")}`;
  return raw;
}

function subscriptionDisplayStatus(record) {
  const base = normalizedStatus(record?.status);
  const push = normalizedStatus(record?.last_push?.status);
  const completion = normalizedStatus(record?.last_completion?.status);
  if (base === "skipped") return { key: "skipped", text: SUB_STATUS_LABELS.skipped };
  if (base === "linked" || push === "linked" || completion === "completed")
    return { key: "linked", text: SUB_STATUS_LABELS.linked };
  if (base === "completed") return { key: "completed", text: SUB_STATUS_LABELS.completed };
  if (base === "failed" || push === "failed" || completion === "failed")
    return { key: "failed", text: SUB_STATUS_LABELS.failed };
  if (push === "downloaded") return { key: "downloaded", text: "已下载待链接" };
  if (push === "downloading" || base === "downloading" || base === "pushed")
    return { key: "pushed", text: SUB_STATUS_LABELS.pushed };
  if (base === "processing") return { key: "processing", text: SUB_STATUS_LABELS.processing };
  return { key: base || "unprocessed", text: SUB_STATUS_LABELS[base] || "待处理" };
}

function subscriptionProgress(record) {
  const progress = Number(record?.last_push?.download_progress);
  if (Number.isFinite(progress)) return Math.max(0, Math.min(1, progress));
  if (normalizedStatus(record?.status) === "completed") return 1;
  return null;
}

function subscriptionCardMeta(record) {
  const push = record.last_push || {};
  return [
    record.release_year || "",
    record.category_text || "",
    push.qb_category ? `qB ${push.qb_category}` : "",
    push.download_state || "",
    record.updated_at ? `更新 ${formatUnixSeconds(record.updated_at)}` : "",
  ].filter(Boolean);
}

function subscriptionCardNote(record) {
  const push = record.last_push || {};
  const completion = record.last_completion || {};
  return (
    completion.error ||
    push.error ||
    record.last_error ||
    formatSubscriptionSkipReason(record.skip_reason) ||
    ""
  );
}

function openSubscriptionDetail(record) {
  detailOpen.value = true;
  detailLoading.value = false;
  detailError.value = "";
  detailKind.value = "subscription";
  selectedSubscription.value = record;
}

const subscriptionEpisodes = computed(() => {
  const record = selectedSubscription.value;
  return (
    (record?.last_completion?.episodes?.length
      ? record.last_completion.episodes
      : record?.last_push?.episodes) || []
  );
});

const subscriptionFiles = computed(() => {
  const record = selectedSubscription.value;
  return [
    ...(record?.last_push?.files || []),
    ...(record?.last_completion?.linked_files || record?.last_push?.linked_files || []),
  ].slice(0, 120);
});

function subscriptionDetailRows(record) {
  return [
    row("豆瓣 ID", record.subject_id),
    row("分类文本", record.category_text),
    row("上映年份", record.release_year),
    row("状态", subscriptionDisplayStatus(record).text),
    row("跳过原因", formatSubscriptionSkipReason(record.skip_reason)),
    row("重试", `${record.retry_count || 0}/${record.max_retries || 0}`),
    row("首次看到", formatUnixSeconds(record.first_seen_at)),
    row("最近更新", formatUnixSeconds(record.updated_at)),
  ].filter(Boolean);
}

function pushRows(push) {
  return [
    row("种子", push.torrent_title),
    row("qB", push.qb_server),
    row("分类", push.qb_category),
    row("保存目录", push.qb_save_dir_name),
    row("qB 状态", push.download_state || pushStatusLabel(push.status)),
    row("qB hash", push.qb_hash),
    row(
      "文件",
      push.total_file_count != null
        ? `${push.completed_file_count || 0}/${push.total_file_count}`
        : "",
    ),
    row("大小", formatBytes(push.total_size)),
    row("检查时间", formatUnixSeconds(push.checked_at)),
  ].filter(Boolean);
}

function completionRows(completion) {
  return [
    row("链接状态", pushStatusLabel(completion.status)),
    row("目标目录", completion.target_dir),
    row("源目录", completion.source_path),
    row("完成时间", formatUnixSeconds(completion.completed_at)),
    row("错误", completion.error),
  ].filter(Boolean);
}

function row(label, value) {
  return value == null || String(value).trim() === "" ? null : { label, value: String(value) };
}

async function refreshSubscriptionProgress(id) {
  subscriptionActionLoading.value = true;
  try {
    const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/progress`, {
      method: "POST",
      body: "{}",
    });
    subscriptionState.value = await api("/api/subscriptions/wanted");
    selectedSubscription.value =
      data.record ||
      subscriptionRecords.value.find((record) => String(record.subject_id) === String(id));
    showToast("下载进度已刷新", "ok");
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    subscriptionActionLoading.value = false;
  }
}

async function checkSubscriptionCompletion(id) {
  subscriptionActionLoading.value = true;
  try {
    const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/completion`, {
      method: "POST",
      body: JSON.stringify({ dry_run: false }),
    });
    subscriptionState.value = await api("/api/subscriptions/wanted");
    selectedSubscription.value =
      data.record ||
      subscriptionRecords.value.find((record) => String(record.subject_id) === String(id));
    showToast(data.completed ? "硬链接完成" : "下载尚未完成", "ok");
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    subscriptionActionLoading.value = false;
  }
}

function pushStatusLabel(status) {
  return PUSH_STATUS_LABELS[normalizedStatus(status)] || status || "";
}

function normalizeDoubanTags(value) {
  return String(value || "")
    .split(/\s+/)
    .map((item) => item.trim())
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

function splitKeywordList(value) {
  return String(value || "")
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function joinKeywordList(values) {
  return Array.isArray(values) ? values.filter(Boolean).join(", ") : "";
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

function formatPercent(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return "";
  return `${Math.round(Math.max(0, Math.min(1, n)) * 100)}%`;
}

function formatSize(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let idx = 0;
  let value = bytes;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value.toFixed(idx ? 2 : 0)} ${units[idx]}`;
}

function formatBytes(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  return formatSize(n);
}
</script>
