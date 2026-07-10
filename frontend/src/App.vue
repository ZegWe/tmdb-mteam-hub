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
          :class="{ 'btn-active is-active': navPage === 'main' }"
          @click="go('main')"
        >
          主功能
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'subscriptions' }"
          @click="go('subscriptions')"
        >
          订阅
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'logs' }"
          @click="go('logs')"
        >
          日志
        </button>
        <button
          type="button"
          class="nav-item btn btn-ghost justify-start"
          :class="{ 'btn-active is-active': navPage === 'settings' }"
          @click="go('settings')"
        >
          设置
        </button>
      </nav>
      <button
        type="button"
        class="theme-toggle"
        :aria-label="themeToggleLabel"
        :title="themeToggleLabel"
        @click="cycleThemeMode"
      >
        <svg
          v-if="themeMode === 'system'"
          viewBox="0 0 24 24"
          aria-hidden="true"
          class="theme-toggle-icon"
        >
          <path d="M4 5.5h16v10H4z" />
          <path d="M9 19h6" />
          <path d="M12 15.5V19" />
        </svg>
        <svg
          v-else-if="themeMode === 'dark'"
          viewBox="0 0 24 24"
          aria-hidden="true"
          class="theme-toggle-icon"
        >
          <path d="M20 14.5A7.5 7.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z" />
        </svg>
        <svg v-else viewBox="0 0 24 24" aria-hidden="true" class="theme-toggle-icon">
          <path d="M12 4V2" />
          <path d="M12 22v-2" />
          <path d="M4.93 4.93 3.51 3.51" />
          <path d="m20.49 20.49-1.42-1.42" />
          <path d="M4 12H2" />
          <path d="M22 12h-2" />
          <path d="m4.93 19.07-1.42 1.42" />
          <path d="m20.49 3.51-1.42 1.42" />
          <circle cx="12" cy="12" r="4" />
        </svg>
      </button>
    </aside>

    <div class="app-content">
      <div v-if="error" id="err" class="banner err alert alert-error" role="alert">{{ error }}</div>
      <div v-if="toast.message" id="toast" class="app-toast" role="status" aria-live="polite">
        <div class="app-toast-message" :class="toast.kind === 'err' ? 'toast-err' : 'toast-ok'">
          {{ toast.message }}
        </div>
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

        <main
          class="layout search-layout"
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
              {{ searchSource === "douban" ? "豆瓣影视" : "电影" }}
            </h2>
            <div class="grid">
              <p v-if="!movies.length" class="empty-hint">无结果</p>
              <article
                v-for="item in movies"
                :key="cardKey(item, 'movie')"
                class="card media-card media-card-search bg-base-100 border border-base-300 shadow-sm"
                @click="openCardDetail(item, 'movie')"
              >
                <img :src="itemImageUrl(item) || transparentPixel" alt="" loading="lazy" />
                <div class="meta">
                  <div class="title">{{ item.title || "(无标题)" }}</div>
                  <div class="subtle">{{ cardSubtitle(item) }}</div>
                </div>
              </article>
            </div>
            <div v-if="showDoubanSearchPager" class="search-pager" aria-label="豆瓣搜索分页">
              <button
                type="button"
                class="btn btn-secondary"
                :disabled="searchLoading || doubanSearchPage.page <= 1"
                @click="loadDoubanSearchPage(doubanSearchPage.page - 1)"
              >
                上一页
              </button>
              <span class="search-pager-status">{{ doubanSearchPagerText }}</span>
              <button
                type="button"
                class="btn btn-secondary"
                :disabled="searchLoading || !doubanSearchPage.has_more"
                @click="loadDoubanSearchPage(doubanSearchPage.page + 1)"
              >
                下一页
              </button>
            </div>
          </section>
          <section v-show="searchSource !== 'douban'" id="tv-section">
            <h2 id="tv-title">剧集</h2>
            <div class="grid">
              <p v-if="!tv.length" class="empty-hint">无结果</p>
              <article
                v-for="item in tv"
                :key="cardKey(item, 'tv')"
                class="card media-card media-card-search bg-base-100 border border-base-300 shadow-sm"
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
              title="从豆瓣获取最新想看订阅"
              @click="loadSubscriptions({ poll: true })"
            >
              刷新
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
            <div class="subscription-card-cover">
              <img :src="itemImageUrl(record) || transparentPixel" alt="" loading="lazy" />
              <span
                class="subscription-status badge"
                :class="`subscription-status-${subscriptionDisplayStatus(record).key}`"
                >{{ subscriptionDisplayStatus(record).text }}</span
              >
            </div>
            <div class="meta subscription-card-meta">
              <div class="title">{{ record.title || record.subject_id }}</div>
              <div class="subtle">{{ subscriptionCardSubtitle(record) }}</div>
            </div>
          </article>
        </section>
      </section>

      <section
        v-show="page === 'logs'"
        id="page-logs"
        class="app-page"
        :class="{ 'is-active': page === 'logs' }"
      >
        <header class="top logs-top">
          <h1>日志</h1>
          <p class="sub">操作事件、结果状态与关联对象</p>
          <div class="actions">
            <button
              type="button"
              class="btn btn-secondary"
              :disabled="operationLogsLoading"
              @click="loadOperationLogs({ page: 1 })"
            >
              刷新
            </button>
          </div>
        </header>

        <section class="operation-log-filters" aria-label="日志筛选">
          <label>
            分类
            <select
              v-model="operationLogFilters.category"
              class="select select-bordered"
              @change="loadOperationLogs({ page: 1 })"
            >
              <option value="">全部分类</option>
              <option v-for="item in operationLogCategories" :key="item.value" :value="item.value">
                {{ item.label }}
              </option>
            </select>
          </label>
          <label>
            状态
            <select
              v-model="operationLogFilters.status"
              class="select select-bordered"
              @change="loadOperationLogs({ page: 1 })"
            >
              <option value="">全部状态</option>
              <option v-for="item in operationLogStatuses" :key="item.value" :value="item.value">
                {{ item.label }}
              </option>
            </select>
          </label>
          <label class="operation-log-search">
            关键词
            <input
              v-model.trim="operationLogFilters.q"
              type="search"
              class="input input-bordered"
              placeholder="搜索说明、对象、错误"
              @keydown.enter="loadOperationLogs({ page: 1 })"
            />
          </label>
          <div class="operation-log-filter-actions">
            <button
              type="button"
              class="btn btn-primary"
              :disabled="operationLogsLoading"
              @click="loadOperationLogs({ page: 1 })"
            >
              筛选
            </button>
            <button type="button" class="btn btn-ghost" @click="resetOperationLogFilters">
              重置
            </button>
          </div>
        </section>

        <section class="operation-log-summary" aria-live="polite">
          <p class="hint">{{ operationLogSummary }}</p>
        </section>

        <section class="operation-log-list" aria-live="polite">
          <div v-if="operationLogsLoading && !operationLogs.length" class="inline-loading">
            <div class="spinner spinner-sm" aria-hidden="true"></div>
            <span>加载日志…</span>
          </div>
          <p v-else-if="!operationLogs.length" class="empty-hint">暂无日志</p>
          <article
            v-for="entry in operationLogs"
            :key="entry.id"
            class="operation-log-card card bg-base-100 border border-base-300 shadow-sm"
          >
            <div class="operation-log-main">
              <div class="operation-log-head">
                <span class="operation-log-time">{{ formatUnixSeconds(entry.created_at) }}</span>
                <span class="operation-log-category badge">{{
                  operationLogCategoryLabel(entry.category)
                }}</span>
                <span
                  class="operation-log-status badge"
                  :class="`operation-log-status-${normalizedStatus(entry.status)}`"
                  >{{ operationLogStatusLabel(entry.status) }}</span
                >
              </div>
              <h2>{{ entry.summary || operationLogActionLabel(entry.action) }}</h2>
              <p class="operation-log-target">
                {{ operationLogTarget(entry) }}
              </p>
              <p v-if="entry.error" class="operation-log-error">{{ entry.error }}</p>
            </div>
            <dl class="operation-log-meta">
              <div>
                <dt>动作</dt>
                <dd>{{ operationLogActionLabel(entry.action) }}</dd>
              </div>
              <div v-if="entry.target_id">
                <dt>ID</dt>
                <dd>{{ entry.target_id }}</dd>
              </div>
              <div v-if="operationLogRelated(entry).length">
                <dt>关联</dt>
                <dd>{{ operationLogRelated(entry).join(" · ") }}</dd>
              </div>
            </dl>
            <section
              v-if="operationLogTorrentMatches(entry).length"
              class="operation-log-matches"
              aria-label="种子匹配结果"
            >
              <div class="operation-log-matches-title">种子匹配</div>
              <div
                v-for="match in operationLogTorrentMatches(entry)"
                :key="`${entry.id}-${match.torrent_id || match.title}`"
                class="operation-log-match"
                :class="{ 'operation-log-match-selected': match.selected }"
              >
                <div class="operation-log-match-head">
                  <strong>{{ match.title || match.torrent_id || "未知种子" }}</strong>
                  <span>{{ match.selected ? "已选中" : "未选中" }}</span>
                </div>
                <p class="operation-log-match-meta">
                  {{ operationLogMatchStats(match) }}
                </p>
                <p v-if="match.matched_rule_name" class="operation-log-match-rule">
                  规则 {{ match.matched_rule_name }}
                  <template v-if="match.matched_priority != null">
                    · 优先级 {{ match.matched_priority }}
                  </template>
                </p>
                <p v-if="operationLogMatchedKeywords(match)" class="operation-log-match-rule">
                  命中 {{ operationLogMatchedKeywords(match) }}
                </p>
                <p v-if="match.excluded_reason" class="operation-log-match-reason">
                  {{ match.excluded_reason }}
                </p>
                <p
                  v-if="operationLogRuleEvaluationSummary(match)"
                  class="operation-log-match-evaluations"
                >
                  {{ operationLogRuleEvaluationSummary(match) }}
                </p>
              </div>
            </section>
          </article>
        </section>

        <div class="operation-log-pager">
          <button
            type="button"
            class="btn btn-secondary"
            :disabled="operationLogsLoading || !operationLogPage.has_more"
            @click="loadMoreOperationLogs"
          >
            加载更多
          </button>
        </div>
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
                  >qB 服务器<select
                    v-model="category.qb_server_id"
                    class="select select-bordered select-sm"
                    :disabled="!settings.qb_servers.length"
                  >
                    <option v-if="!settings.qb_servers.length" value="">请先添加 qB 服务器</option>
                    <option
                      v-for="server in settings.qb_servers"
                      :key="server.id"
                      :value="server.id"
                    >
                      {{ qbServerOptionLabel(server) }}
                    </option>
                  </select></label
                >
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
                  >匹配模式<select
                    v-model="rule.mode"
                    class="select select-bordered select-sm"
                    :title="rule.mode === 'all' ? '全部满足' : '任一满足'"
                  >
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

      <section
        v-show="page === 'detail'"
        id="page-detail"
        class="app-page detail-page"
        :class="{ 'is-active': page === 'detail' }"
      >
        <header class="top detail-page-top">
          <div>
            <h1>{{ detailPageTitle }}</h1>
            <p class="sub">{{ detailPageSubtitle }}</p>
          </div>
          <div class="actions">
            <button type="button" class="btn btn-secondary" @click="closeDetail">返回</button>
          </div>
        </header>

        <div class="detail-body">
          <div v-if="detailLoading" class="detail-loading" role="status">
            <div class="spinner" aria-hidden="true"></div>
            <p>加载详情…</p>
          </div>
          <p v-else-if="detailError" class="empty-hint">加载失败：{{ detailError }}</p>

          <MediaDetailView
            v-else-if="detailKind === 'media' && detailData"
            :detail-poster="detailPoster"
            :detail-title="detailTitle"
            :detail-media-type="detailMediaType"
            :detail-date="detailDate"
            :external-links="externalLinks"
            :detail-douban-id="detailDoubanId"
            :douban-interest-status="doubanInterestStatus"
            :douban-mark="doubanMark"
            :set-douban-interest="setDoubanInterest"
            :selected-douban-rating-label="selectedDoubanRatingLabel"
            :douban-save-disabled="doubanSaveDisabled"
            :save-douban-interest="saveDoubanInterest"
            :selected-douban-category-label="selectedDoubanCategoryLabel"
            :subscription-categories-cache="subscriptionCategoriesCache"
            :douban-tag-history="doubanTagHistory"
            :apply-douban-tag-suggestion="applyDoubanTagSuggestion"
            :detail-data="detailData"
            :detail-meta-rows="detailMetaRows"
            :detail-seasons="detailSeasons"
            :load-season-episodes="loadSeasonEpisodes"
            :season-loading="seasonLoading"
            :season-errors="seasonErrors"
            :season-episodes="seasonEpisodes"
            :episode-still="episodeStill"
            :detail-overview="detailOverview"
            :mteam-sources="mteamSources"
            :active-torrent-source="activeTorrentSource"
            :select-torrent-source="selectTorrentSource"
            :torrents-loading="torrentsLoading"
            :torrent-error="torrentError"
            :torrent-rows="torrentRows"
            :mteam-torrent-web-url="mteamTorrentWebUrl"
            :torrent-stats="torrentStats"
            :open-qb-push-dialog="openQbPushDialog"
          />

          <SubscriptionDetailView
            v-else-if="detailKind === 'subscription' && selectedSubscription"
            :selected-subscription="selectedSubscription"
            :subscription-display-status="subscriptionDisplayStatus"
            :subscription-lifecycle-nodes="subscriptionLifecycleNodes"
            :subscription-detail-rows="subscriptionDetailRows"
            :subscription-action-loading="subscriptionActionLoading"
            :can-retry-subscription="canRetrySubscription"
            :retry-subscription-current="retrySubscriptionCurrent"
            :can-rerun-subscription="canRerunSubscription"
            :rerun-subscription="rerunSubscription"
            :subscription-progress="subscriptionProgress"
            :format-percent="formatPercent"
            :push-rows="pushRows"
            :subscription-episodes="subscriptionEpisodes"
            :push-status-label="pushStatusLabel"
            :completion-rows="completionRows"
            :subscription-files="subscriptionFiles"
            :format-bytes="formatBytes"
          />
        </div>
      </section>
    </div>
  </div>

  <dialog class="modal" :open="qbDialogOpen">
    <form class="modal-box" @submit.prevent="submitQbPush">
      <h2>推送到 qBittorrent</h2>
      <p class="hint subtle">{{ qbPushLabel }}</p>
      <label
        >qB 服务器<select
          v-model="qbPush.serverIndex"
          class="select select-bordered"
          :disabled="!qbServersCache.length"
          :title="selectedQbPushServerLabel"
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
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import MediaDetailView from "./components/MediaDetailView.vue";
import SubscriptionDetailView from "./components/SubscriptionDetailView.vue";

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
};

const SUB_SKIP_REASON_LABELS = {
  initial_bootstrap_existing_wish: "历史想看，首次同步跳过",
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

const SUBSCRIPTION_AUTO_SYNC_MS = 5000;

const OPERATION_LOG_CATEGORIES = [
  { value: "subscription_sync", label: "订阅同步" },
  { value: "search", label: "搜索订阅" },
  { value: "torrent_search", label: "搜索种子" },
  { value: "qb_push", label: "推送 qB" },
  { value: "download_progress", label: "下载进度" },
  { value: "completion", label: "完成检测" },
  { value: "hardlink", label: "硬链接" },
  { value: "configuration", label: "配置保存" },
  { value: "system_error", label: "系统/错误" },
];

const OPERATION_LOG_STATUSES = [
  { value: "success", label: "成功" },
  { value: "failed", label: "失败" },
  { value: "processing", label: "处理中" },
];

const OPERATION_LOG_ACTION_LABELS = {
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
};

const THEME_STORAGE_KEY = "tmdb-mteam-theme-mode";
const THEME_MODES = ["system", "light", "dark"];
const THEME_MODE_LABELS = {
  system: "主题：跟随系统",
  light: "主题：浅色",
  dark: "主题：深色",
};

function normalizeThemeMode(value) {
  return THEME_MODES.includes(value) ? value : "system";
}

function resolveThemeScheme(mode, prefersDark) {
  const normalized = normalizeThemeMode(mode);
  if (normalized === "dark") return "dark";
  if (normalized === "light") return "light";
  return prefersDark ? "dark" : "light";
}

function nextThemeMode(mode) {
  const normalized = normalizeThemeMode(mode);
  const index = THEME_MODES.indexOf(normalized);
  return THEME_MODES[(index + 1) % THEME_MODES.length];
}

function themeModeLabel(mode) {
  return THEME_MODE_LABELS[normalizeThemeMode(mode)];
}

function readStoredThemeMode() {
  if (typeof window === "undefined") return "system";
  try {
    return normalizeThemeMode(window.localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return "system";
  }
}

function storeThemeMode(mode) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, normalizeThemeMode(mode));
  } catch {}
}

function readSystemPrefersDark() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyThemeScheme(scheme) {
  if (typeof document === "undefined") return;
  const normalized = scheme === "dark" ? "dark" : "light";
  document.documentElement.dataset.colorScheme = normalized;
  document.documentElement.style.colorScheme = normalized;
  if (document.body) {
    document.body.dataset.theme = normalized === "dark" ? "mediahub-dark" : "mediahub";
  }
}

const route = useRoute();
const router = useRouter();
const routeToPage = {
  main: "main",
  "media-detail": "detail",
  subscriptions: "subscriptions",
  "subscription-detail": "detail",
  logs: "logs",
  settings: "settings",
};

const DETAIL_MEDIA_TYPES = ["movie", "tv", "douban"];

function firstQueryValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

function normalizeDetailRoute(routeLike) {
  const name = String(routeLike?.name || "");
  const params = routeLike?.params || {};
  if (name === "media-detail") {
    const mediaType = String(firstQueryValue(params.mediaType) || "").trim();
    const id = String(firstQueryValue(params.id) || "").trim();
    if (DETAIL_MEDIA_TYPES.includes(mediaType) && id) {
      return { kind: "media", mediaType, id };
    }
    return null;
  }
  if (name === "subscription-detail") {
    const id = String(firstQueryValue(params.id) || "").trim();
    return id ? { kind: "subscription", id } : null;
  }
  return null;
}

function detailRouteLocationFromMediaCard(item, fallbackType) {
  const type = item?.source === "douban" ? "douban" : item?.media_type || fallbackType;
  const mediaType = DETAIL_MEDIA_TYPES.includes(type) ? type : fallbackType;
  const rawId = type === "douban" ? (item?.id ?? item?.subject_id) : item?.id;
  const id = String(rawId || "").trim();
  if (!id) return null;
  const query = {};
  const tags = Array.isArray(item?.tags) ? item.tags.join(" ") : item?.tags || "";
  if (mediaType === "douban" && String(tags).trim()) query.doubanTags = String(tags).trim();
  return { name: "media-detail", params: { mediaType, id }, query };
}

function detailRouteLocationFromSubscriptionRecord(record) {
  const id = String(record?.subject_id || "").trim();
  return id ? { name: "subscription-detail", params: { id }, query: {} } : null;
}

function detailBackRouteLocation(parsed) {
  return parsed?.kind === "subscription" ? { name: "subscriptions" } : { name: "main" };
}

const page = computed(() => routeToPage[route.name] || "main");
const navPage = computed(() => {
  if (route.name === "media-detail") return "main";
  if (route.name === "subscription-detail") return "subscriptions";
  return page.value;
});
const error = ref("");
const toast = reactive({ message: "", kind: "ok", timer: 0 });
const themeMode = ref(readStoredThemeMode());
const systemPrefersDark = ref(readSystemPrefersDark());
const resolvedThemeScheme = computed(() =>
  resolveThemeScheme(themeMode.value, systemPrefersDark.value),
);
const themeToggleLabel = computed(() => themeModeLabel(themeMode.value));
let themePreferenceCleanup = null;
applyThemeScheme(resolvedThemeScheme.value);

const searchSource = ref("tmdb");
const query = ref("");
const searchLoading = ref(false);
const searchLoadingText = ref("正在搜索 TMDB…");
const movies = ref([]);
const tv = ref([]);
const doubanSearchPage = reactive({ page: 1, page_size: 20, has_more: false });
const showDoubanSearchPager = computed(
  () => searchSource.value === "douban" && (movies.value.length > 0 || doubanSearchPage.page > 1),
);
const doubanSearchPagerText = computed(() => {
  const start = movies.value.length
    ? (Number(doubanSearchPage.page || 1) - 1) * Number(doubanSearchPage.page_size || 20) + 1
    : 0;
  const end = start ? start + movies.value.length - 1 : 0;
  const range = start && end ? `${start}-${end}` : "0";
  return `第 ${doubanSearchPage.page} 页 · ${range}`;
});

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
let localQbServerSeq = 0;

const subscriptionsLoading = ref(false);
const subscriptionState = ref(null);
const subscriptionActionLoading = ref(false);
let subscriptionAutoSyncTimer = 0;
let subscriptionAutoSyncInFlight = false;
let detailOpenedFromRoutePush = false;

const operationLogsLoading = ref(false);
const operationLogs = ref([]);
const operationLogFilters = reactive({ category: "", status: "", q: "" });
const operationLogPage = reactive({ page: 1, page_size: 30, total: 0, has_more: false });
const operationLogCategories = OPERATION_LOG_CATEGORIES;
const operationLogStatuses = OPERATION_LOG_STATUSES;

const doubanMark = reactive({ interest: "", rating: "", tags: "", category: "", status: "" });

watch(
  () => [page.value, route.name],
  ([next]) => {
    clearError();
    if (next === "settings") loadSettings();
    if (isSubscriptionRoute()) {
      loadSubscriptions({ silent: true });
      startSubscriptionAutoSync();
    } else {
      stopSubscriptionAutoSync();
    }
    if (next === "logs") loadOperationLogs({ page: 1, silent: true });
    if (next !== "settings") resetDoubanQrUi();
    if (next === "settings") closeDetail();
  },
  { immediate: true },
);

watch(
  () => [route.name, route.params.mediaType, route.params.id, route.query.doubanTags],
  () => {
    syncDetailFromRoute().catch((err) => {
      detailOpen.value = true;
      detailLoading.value = false;
      detailError.value = err instanceof Error ? err.message : String(err);
    });
  },
  { immediate: true },
);

watch(resolvedThemeScheme, (scheme) => {
  applyThemeScheme(scheme);
});

watch(themeMode, (mode) => {
  storeThemeMode(mode);
});

onMounted(() => {
  loadSettings();
  if (isSubscriptionRoute()) startSubscriptionAutoSync();
  themePreferenceCleanup = watchSystemThemePreference((prefersDark) => {
    systemPrefersDark.value = prefersDark;
  });
});

onBeforeUnmount(() => {
  stopSubscriptionAutoSync();
  if (themePreferenceCleanup) {
    themePreferenceCleanup();
    themePreferenceCleanup = null;
  }
});

function watchSystemThemePreference(onChange) {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  onChange(media.matches);
  const listener = (event) => onChange(event.matches);
  if (typeof media.addEventListener === "function") {
    media.addEventListener("change", listener);
    return () => media.removeEventListener("change", listener);
  }
  if (typeof media.addListener === "function") {
    media.addListener(listener);
    return () => media.removeListener(listener);
  }
  return null;
}

function cycleThemeMode() {
  themeMode.value = nextThemeMode(themeMode.value);
}

function go(target) {
  const path =
    target === "subscriptions"
      ? "/subscriptions"
      : target === "logs"
        ? "/logs"
        : target === "settings"
          ? "/settings"
          : "/";
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
    const bits = [
      item.year || item.abstract_2 || item.date || "",
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
  doubanSearchPage.page = 1;
  doubanSearchPage.has_more = false;
}

function setSearchLoading(on, text = "正在搜索…") {
  searchLoading.value = on;
  searchLoadingText.value = text;
}

async function runSearch(pageNumber = 1) {
  clearError();
  const q = query.value.trim();
  if (!q) {
    showError("请输入搜索词");
    return;
  }
  const targetPage = typeof pageNumber === "number" && Number.isFinite(pageNumber) ? pageNumber : 1;
  setSearchLoading(true, searchSource.value === "douban" ? "正在搜索豆瓣…" : "正在搜索 TMDB…");
  try {
    if (searchSource.value === "douban") {
      const params = new URLSearchParams({
        q,
        page: String(Math.max(1, targetPage)),
        page_size: String(doubanSearchPage.page_size || 20),
      });
      const data = await api(`/api/douban/search?${params}`);
      movies.value = data.items || data.movies || [];
      tv.value = [];
      doubanSearchPage.page = Number(data?.page || targetPage) || targetPage;
      doubanSearchPage.page_size = Number(data?.page_size || doubanSearchPage.page_size || 20);
      doubanSearchPage.has_more = !!data?.has_more;
    } else {
      const data = await api(`/api/search?${new URLSearchParams({ q })}`);
      movies.value = data.movies || [];
      tv.value = data.tv || [];
      doubanSearchPage.page = 1;
      doubanSearchPage.has_more = false;
    }
  } catch (err) {
    showError(err instanceof Error ? err.message : String(err));
  } finally {
    setSearchLoading(false);
  }
}

function loadDoubanSearchPage(pageNumber) {
  if (searchLoading.value || searchSource.value !== "douban") return;
  runSearch(Math.max(1, Number(pageNumber) || 1));
}

function openCardDetail(item, fallbackType) {
  const detailLocation = detailRouteLocationFromMediaCard(item, fallbackType);
  if (!detailLocation) return;
  pushDetailRoute(detailLocation);
}

function pushDetailRoute(target) {
  const alreadyInDetail = !!normalizeDetailRoute(route);
  if (!alreadyInDetail) detailOpenedFromRoutePush = true;
  const navigation = alreadyInDetail ? router.replace(target) : router.push(target);
  navigation.catch(handleRouteNavigationError);
}

function handleRouteNavigationError(err) {
  const message = err instanceof Error ? err.message : String(err || "");
  if (/duplicated|redundant|same route/i.test(message)) return;
  showError(message || "更新详情 URL 失败");
}

async function syncDetailFromRoute() {
  const parsed = normalizeDetailRoute(route);
  if (!parsed) {
    detailOpenedFromRoutePush = false;
    closeDetailState();
    return;
  }

  if (parsed.kind === "media" && route.name === "media-detail") {
    const doubanTags = normalizeDoubanTags(firstQueryValue(route.query.doubanTags) || "");
    await loadMediaDetailFromRoute(parsed.mediaType, parsed.id, { doubanTags });
    return;
  }

  if (parsed.kind === "subscription" && route.name === "subscription-detail") {
    await loadSubscriptionDetailFromRoute(parsed.id);
    return;
  }

  closeDetailState();
}

async function loadMediaDetailFromRoute(mediaType, id, options = {}) {
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

async function loadSubscriptionDetailFromRoute(id) {
  clearError();
  resetDetail();
  detailOpen.value = true;
  detailLoading.value = true;
  detailKind.value = "subscription";
  try {
    if (!refreshSelectedSubscriptionFromRoute()) {
      await loadSubscriptions({ silent: true });
      refreshSelectedSubscriptionFromRoute();
    }
    if (!selectedSubscription.value) {
      detailError.value = `未找到订阅记录：${id}`;
    }
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

function closeDetailState() {
  detailOpen.value = false;
  detailLoading.value = false;
  resetDetail();
}

function closeDetail() {
  const parsed = normalizeDetailRoute(route);
  if (parsed) {
    if (detailOpenedFromRoutePush) {
      detailOpenedFromRoutePush = false;
      router.back();
      return;
    }
    router.replace(detailBackRouteLocation(parsed)).catch(handleRouteNavigationError);
    return;
  }
  closeDetailState();
}

const detailPageTitle = computed(() => {
  if (detailKind.value === "subscription") {
    return selectedSubscription.value?.title || "订阅详情";
  }
  return detailTitle.value || "影视详情";
});

const detailPageSubtitle = computed(() => {
  if (detailKind.value === "subscription") return "订阅状态、下载进度与硬链接结果";
  if (detailMediaType.value === "douban") return "豆瓣资料、标记与 M-Team 种子";
  if (detailMediaType.value === "tv") return "剧集资料、分集与 M-Team 种子";
  return "电影资料、豆瓣标记与 M-Team 种子";
});

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
  const originalTitle = String(data.original_title || "").trim();
  const localizedTitle = String(data.title || "").trim();
  if (originalTitle && originalTitle !== localizedTitle) rows.push(["原名", originalTitle]);
  const aka = joinNames(data.aka);
  if (aka) rows.push(["又名", aka]);
  const countries = joinNames(data.countries);
  if (countries) rows.push(["国家/地区", countries]);
  const languages = joinNames(data.languages);
  if (languages) rows.push(["语言", languages]);
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
const selectedDoubanRatingLabel = computed(() =>
  doubanMark.rating ? `${doubanMark.rating} 星` : "未评分",
);
const selectedDoubanCategoryLabel = computed(() => {
  if (!doubanMark.category)
    return subscriptionCategoriesCache.value.length ? "选择订阅分类" : "未配置订阅分类";
  const category = subscriptionCategoriesCache.value.find(
    (item) => item.wanted_tag === doubanMark.category,
  );
  return category
    ? `${category.name || category.wanted_tag} · ${category.wanted_tag}`
    : doubanMark.category;
});

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
const selectedQbPushServerLabel = computed(() => {
  const server = qbServersCache.value[Number(qbPush.serverIndex)];
  return server
    ? server.name || server.base_url || `服务器 ${Number(qbPush.serverIndex) + 1}`
    : "未配置 qB（请打开 API 设置）";
});

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
    settings.qb_servers = normalizeQbServerForms(
      Array.isArray(data.qb_servers) ? data.qb_servers : [],
    );
    settings.subscription_categories = (
      Array.isArray(data.subscription_categories) ? data.subscription_categories : []
    ).map((category) => ({
      ...category,
      qb_server_id: String(category.qb_server_id || "").trim() || settings.qb_servers[0]?.id || "",
    }));
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

function normalizeQbServerForms(servers) {
  const used = new Set();
  return servers.map((server) => {
    const id = uniqueClientQbServerId(server.id || server.name || server.base_url, used);
    return {
      ...server,
      id,
      testMessage: "",
      testKind: "",
      testing: false,
    };
  });
}

function newQbServerId() {
  localQbServerSeq += 1;
  return `qb-${Date.now().toString(36)}-${localQbServerSeq}`;
}

function uniqueClientQbServerId(raw, used) {
  const base =
    String(raw || "")
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, "-")
      .replace(/^-+|-+$/g, "") || newQbServerId();
  if (!used.has(base)) {
    used.add(base);
    return base;
  }
  for (let idx = 2; ; idx += 1) {
    const candidate = `${base}-${idx}`;
    if (!used.has(candidate)) {
      used.add(candidate);
      return candidate;
    }
  }
}

function qbServerOptionLabel(server) {
  const name = String(server?.name || "").trim();
  const url = String(server?.base_url || "").trim();
  if (name && url) return `${name} · ${url}`;
  return name || url || server?.id || "未命名 qB";
}

function addSubscriptionCategory() {
  settings.subscription_categories.push({
    name: "",
    wanted_tag: "",
    qb_server_id: settings.qb_servers[0]?.id || "",
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
    id: newQbServerId(),
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
    id: String(server.id || "").trim() || newQbServerId(),
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
    qb_server_id: String(category.qb_server_id || "").trim(),
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
  return records.sort(compareSubscriptionRecords);
});

function subscriptionOrderTimestamp(record) {
  const doubanSortTime = Number(record?.douban_sort_time || 0);
  if (Number.isFinite(doubanSortTime) && doubanSortTime > 0) return doubanSortTime;
  const firstSeen = Number(record?.first_seen_at || 0);
  if (Number.isFinite(firstSeen) && firstSeen > 0) return firstSeen;
  const created = Number(record?.created_at || 0);
  if (Number.isFinite(created) && created > 0) return created;
  const updated = Number(record?.updated_at || 0);
  return Number.isFinite(updated) ? updated : 0;
}

function subscriptionReturnOrder(record) {
  const order = Number(record?.douban_return_order);
  return Number.isFinite(order) && order >= 0 ? order : null;
}

function compareSubscriptionRecords(a, b) {
  const aReturnOrder = subscriptionReturnOrder(a);
  const bReturnOrder = subscriptionReturnOrder(b);
  if (aReturnOrder !== null && bReturnOrder !== null) {
    const returnOrderDelta = aReturnOrder - bReturnOrder;
    if (returnOrderDelta) return returnOrderDelta;
  }
  if (aReturnOrder !== null) return -1;
  if (bReturnOrder !== null) return 1;

  const orderDelta = subscriptionOrderTimestamp(b) - subscriptionOrderTimestamp(a);
  if (orderDelta) return orderDelta;

  const createdDelta = Number(b?.created_at || 0) - Number(a?.created_at || 0);
  if (createdDelta) return createdDelta;

  const idDelta = String(a?.subject_id || "").localeCompare(
    String(b?.subject_id || ""),
    "zh-Hans-CN",
    {
      numeric: true,
      sensitivity: "base",
    },
  );
  if (idDelta) return idDelta;

  return String(a?.title || "").localeCompare(String(b?.title || ""), "zh-Hans-CN", {
    numeric: true,
    sensitivity: "base",
  });
}

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
    const params = new URLSearchParams();
    if (!poll && !silent) params.set("log", "true");
    const suffix = params.toString() ? `?${params}` : "";
    subscriptionState.value = await api(`/api/subscriptions/wanted${suffix}`);
    refreshSelectedSubscriptionFromRoute();
    if (!silent) showToast(poll ? subscriptionPollToast(pollOutcome) : "本地订阅列表已刷新", "ok");
    return subscriptionState.value;
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    const detail = `${poll ? "刷新豆瓣订阅失败" : "刷新本地订阅列表失败"}：${msg}`;
    showError(detail);
    if (!silent) showToast(detail, "err");
    return null;
  } finally {
    subscriptionsLoading.value = false;
  }
}

function startSubscriptionAutoSync() {
  stopSubscriptionAutoSync();
  subscriptionAutoSyncTimer = window.setInterval(() => {
    syncSubscriptionState({ silent: true }).catch(() => {});
  }, SUBSCRIPTION_AUTO_SYNC_MS);
}

function stopSubscriptionAutoSync() {
  if (subscriptionAutoSyncTimer) {
    clearInterval(subscriptionAutoSyncTimer);
    subscriptionAutoSyncTimer = 0;
  }
}

function isSubscriptionRoute() {
  return route.name === "subscriptions" || route.name === "subscription-detail";
}

async function syncSubscriptionState({ silent = true } = {}) {
  if (subscriptionAutoSyncInFlight || !isSubscriptionRoute()) return subscriptionState.value;
  if (typeof document !== "undefined" && document.visibilityState === "hidden") {
    return subscriptionState.value;
  }
  subscriptionAutoSyncInFlight = true;
  const selectedId = selectedSubscription.value?.subject_id || selectedSubscriptionRouteId();
  try {
    subscriptionState.value = await api("/api/subscriptions/wanted");
    if (selectedId) {
      selectedSubscription.value =
        subscriptionRecords.value.find(
          (record) => String(record.subject_id) === String(selectedId),
        ) || selectedSubscription.value;
    }
    return subscriptionState.value;
  } catch (err) {
    if (!silent) {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(`同步订阅状态失败：${msg}`, "err");
    }
    return null;
  } finally {
    subscriptionAutoSyncInFlight = false;
  }
}

const operationLogSummary = computed(() => {
  const total = Number(operationLogPage.total || 0);
  const shown = operationLogs.value.length;
  const bits = [`共 ${total} 条`, `已显示 ${shown} 条`];
  const category = operationLogFilters.category
    ? operationLogCategoryLabel(operationLogFilters.category)
    : "";
  const status = operationLogFilters.status
    ? operationLogStatusLabel(operationLogFilters.status)
    : "";
  if (category) bits.push(`分类 ${category}`);
  if (status) bits.push(`状态 ${status}`);
  if (operationLogFilters.q) bits.push(`关键词 ${operationLogFilters.q}`);
  return bits.join(" · ");
});

async function loadOperationLogs({ page = 1, append = false, silent = false } = {}) {
  operationLogsLoading.value = true;
  try {
    const params = new URLSearchParams({
      page: String(page),
      page_size: String(operationLogPage.page_size || 30),
    });
    if (operationLogFilters.category) params.set("category", operationLogFilters.category);
    if (operationLogFilters.status) params.set("status", operationLogFilters.status);
    if (operationLogFilters.q) params.set("q", operationLogFilters.q);
    const data = await api(`/api/operation-logs?${params}`);
    const items = Array.isArray(data?.items) ? data.items : [];
    operationLogs.value = append ? [...operationLogs.value, ...items] : items;
    operationLogPage.page = Number(data?.page || page) || page;
    operationLogPage.page_size = Number(data?.page_size || operationLogPage.page_size || 30);
    operationLogPage.total = Number(data?.total || 0);
    operationLogPage.has_more = !!data?.has_more;
    if (!silent) showToast("日志已加载", "ok");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    showError(`加载日志失败：${msg}`);
    if (!silent) showToast(`加载日志失败：${msg}`, "err");
  } finally {
    operationLogsLoading.value = false;
  }
}

function loadMoreOperationLogs() {
  if (operationLogsLoading.value || !operationLogPage.has_more) return;
  loadOperationLogs({ page: Number(operationLogPage.page || 1) + 1, append: true });
}

function resetOperationLogFilters() {
  operationLogFilters.category = "";
  operationLogFilters.status = "";
  operationLogFilters.q = "";
  loadOperationLogs({ page: 1 });
}

function operationLogCategoryLabel(value) {
  return (
    OPERATION_LOG_CATEGORIES.find((item) => item.value === normalizedStatus(value))?.label ||
    value ||
    "未分类"
  );
}

function operationLogStatusLabel(value) {
  return (
    OPERATION_LOG_STATUSES.find((item) => item.value === normalizedStatus(value))?.label ||
    value ||
    "未知"
  );
}

function operationLogActionLabel(value) {
  return OPERATION_LOG_ACTION_LABELS[value] || value || "操作";
}

function operationLogTarget(entry) {
  const parts = [
    entry.target_title || "",
    entry.target_type ? `对象 ${entry.target_type}` : "",
    entry.target_id ? `ID ${entry.target_id}` : "",
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : "无关联对象";
}

function operationLogRelated(entry) {
  const related = entry?.related && typeof entry.related === "object" ? entry.related : {};
  return Object.entries(related)
    .filter(([, value]) => value != null && value !== "" && typeof value !== "object")
    .slice(0, 6)
    .map(([key, value]) => `${operationLogRelatedLabel(key)} ${value}`);
}

function operationLogTorrentMatches(entry) {
  const related = entry?.related && typeof entry.related === "object" ? entry.related : {};
  return Array.isArray(related.torrent_matches) ? related.torrent_matches : [];
}

function operationLogMatchStats(match) {
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

function operationLogMatchedKeywords(match) {
  return Array.isArray(match.matched_keywords) && match.matched_keywords.length
    ? match.matched_keywords.join("、")
    : "";
}

function operationLogRuleEvaluationSummary(match) {
  const rows = Array.isArray(match.rule_evaluations) ? match.rule_evaluations : [];
  return rows
    .map((item) => {
      const bits = [`${item.rule_name || "未命名规则"} ${item.matched ? "命中" : "未命中"}`];
      if (Array.isArray(item.matched_keywords) && item.matched_keywords.length)
        bits.push(`命中 ${item.matched_keywords.join("、")}`);
      if (Array.isArray(item.missing_keywords) && item.missing_keywords.length)
        bits.push(`缺少 ${item.missing_keywords.join("、")}`);
      if (item.excluded_reason) bits.push(item.excluded_reason);
      return bits.join("，");
    })
    .join("；");
}

function operationLogRelatedLabel(key) {
  const labels = {
    candidate_count: "候选",
    match_count: "匹配",
    selected_torrent_id: "种子",
    torrent_id: "种子",
    qb_server: "qB",
    qb_category: "分类",
    download_progress: "进度",
    file_count: "文件",
    plan_file_count: "计划",
    total_wish_items: "想看",
    created_unprocessed: "新增",
    created_skipped: "跳过",
    updated_existing: "更新",
  };
  return labels[key] || key;
}

function subscriptionPollToast(outcome) {
  if (!outcome || typeof outcome !== "object") return "订阅刷新完成";
  return `订阅刷新完成：新增待处理 ${Number(outcome.created_unprocessed || 0)} · 跳过旧想看 ${Number(outcome.created_skipped || 0)} · 更新已有 ${Number(outcome.updated_existing || 0)}`;
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
  const lifecycle = subscriptionLifecycleKey(record);
  const attention = subscriptionAttentionKey(record);
  if (attention) {
    return { key: attention, text: SUB_ATTENTION_LABELS[attention] || attention };
  }
  return { key: lifecycle, text: SUB_LIFECYCLE_LABELS[lifecycle] || "待处理" };
}

function subscriptionProgress(record) {
  const progress = Number(record?.last_push?.download_progress);
  if (Number.isFinite(progress)) return Math.max(0, Math.min(1, progress));
  if (subscriptionLifecycleKey(record) === "completed") return 1;
  return null;
}

function subscriptionProgressIndex(record) {
  return Math.max(
    0,
    SUB_LIFECYCLE_STEPS.findIndex((step) => step.key === subscriptionLifecycleKey(record)),
  );
}

function subscriptionLifecycleNodes(record) {
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

function subscriptionLifecycleKey(record) {
  const lifecycle = normalizedStatus(record?.lifecycle_state);
  if (SUB_LIFECYCLE_STEPS.some((step) => step.key === lifecycle)) return lifecycle;
  return "queued";
}

function subscriptionAttentionKey(record) {
  const tags = Array.isArray(record?.attention_tags)
    ? record.attention_tags.map((tag) => normalizedStatus(tag))
    : [];
  const activeTags = [...tags];
  if (record?.failure && !activeTags.includes("failed")) activeTags.push("failed");
  return SUB_ATTENTION_PRIORITY.find((tag) => activeTags.includes(tag)) || "";
}

function subscriptionStageTrackLabel(record) {
  return subscriptionLifecycleNodes(record)
    .map((node) => `${node.label}${node.state === "current" ? "（当前）" : ""}`)
    .join("，");
}

function canRetrySubscription(record) {
  return !!record?.subject_id && subscriptionLifecycleKey(record) !== "completed";
}

function canRerunSubscription(record) {
  return !!record?.subject_id;
}

function subscriptionCardMeta(record) {
  const push = record.last_push || {};
  return [
    record.douban_date ? `豆瓣 ${record.douban_date}` : "",
    record.release_year || "",
    record.category_text || "",
    push.qb_category ? `qB ${push.qb_category}` : "",
    push.download_state || "",
    record.updated_at ? `更新 ${formatUnixSeconds(record.updated_at)}` : "",
  ].filter(Boolean);
}

function subscriptionCardSubtitle(record) {
  return (
    String(record?.date_published || "").trim() ||
    String(record?.release_year || "").trim() ||
    record?.subject_id ||
    ""
  );
}

function subscriptionCardNotices(record) {
  const notices = [];
  const attention = subscriptionAttentionKey(record);
  if (attention === "skipped") {
    notices.push(
      subscriptionCardNotice("skipped", "stage", formatSubscriptionSkipReason(record?.skip_reason)),
    );
  }
  if (attention === "waiting_release") {
    notices.push(subscriptionCardNotice("waiting-release", "stage", "等待资源发布"));
  }
  if (attention === "retry_blocked") {
    notices.push(subscriptionCardNotice("retry-blocked", "error", "已达到重试上限"));
  }
  const failureMessage = String(record?.failure?.message || record?.last_error || "").trim();
  if (attention === "failed") {
    notices.push(subscriptionCardNotice("failure", "error", failureMessage));
  }
  return notices.filter(Boolean);
}

function subscriptionCardNotice(key, kind, text) {
  const value = String(text || "").trim();
  return value ? { key, kind, text: value } : null;
}

function openSubscriptionDetail(record) {
  const detailLocation = detailRouteLocationFromSubscriptionRecord(record);
  if (!detailLocation) return;
  pushDetailRoute(detailLocation);
}

function selectedSubscriptionRouteId() {
  const parsed = normalizeDetailRoute(route);
  return parsed?.kind === "subscription" ? parsed.id : "";
}

function refreshSelectedSubscriptionFromRoute() {
  const id = selectedSubscriptionRouteId();
  if (!id) return false;
  const record = subscriptionRecords.value.find((item) => String(item.subject_id) === String(id));
  if (!record) return false;
  selectedSubscription.value = record;
  return true;
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
    row("上映日期", record.date_published),
    row("评分", subscriptionRatingText(record)),
    row("原名", record.original_title),
    row("又名", joinDetailList(record.aka)),
    row("类型", joinDetailList(record.genres)),
    row("国家/地区", joinDetailList(record.countries)),
    row("语言", joinDetailList(record.languages)),
    row("导演", joinDetailList(record.directors)),
    row("主演", joinDetailList(record.actors)),
    row("片长", record.duration),
    row("简介", record.summary),
    row("豆瓣时间", record.douban_date),
    row("上映年份", record.release_year),
    row("失败", record.failure?.message),
    row("说明", record.failure ? "" : record.last_error),
    row("跳过原因", formatSubscriptionSkipReason(record.skip_reason)),
    row("重试", `${record.retry_count || 0}/${record.max_retries || 0}`),
    row("首次看到", formatUnixSeconds(record.first_seen_at)),
    row("最近更新", formatUnixSeconds(record.updated_at)),
  ].filter(Boolean);
}

function joinDetailList(value) {
  return Array.isArray(value) ? value.filter(Boolean).join(" · ") : "";
}

function subscriptionRatingText(record) {
  const rating = Number(record?.rating_value);
  if (!Number.isFinite(rating)) return "";
  const count = Number(record?.rating_count);
  return Number.isFinite(count) && count > 0
    ? `${rating.toFixed(1)}（${count.toLocaleString()} 人）`
    : rating.toFixed(1);
}

function pushRows(push) {
  return [
    row("种子", push.torrent_title),
    row("种子链接", push.torrent_download_url, push.torrent_download_url),
    row("M-Team", push.mteam_torrent_url, push.mteam_torrent_url),
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

function row(label, value, href = "") {
  if (value == null || String(value).trim() === "") return null;
  const text = String(value);
  const link = String(href || "").trim();
  return link ? { label, value: text, href: link } : { label, value: text };
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

async function retrySubscriptionCurrent(id) {
  subscriptionActionLoading.value = true;
  try {
    const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/retry-current`, {
      method: "POST",
      body: "{}",
    });
    await refreshSubscriptionAfterAction(id, data.record);
    showToast("已重试当前节点", "ok");
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    subscriptionActionLoading.value = false;
  }
}

async function rerunSubscription(id) {
  subscriptionActionLoading.value = true;
  try {
    const data = await api(`/api/subscriptions/wanted/${encodeURIComponent(id)}/rerun`, {
      method: "POST",
      body: "{}",
    });
    await refreshSubscriptionAfterAction(id, data.record);
    showToast("已从匹配阶段重跑", "ok");
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "err");
  } finally {
    subscriptionActionLoading.value = false;
  }
}

async function refreshSubscriptionAfterAction(id, fallbackRecord) {
  if (fallbackRecord) selectedSubscription.value = fallbackRecord;
  await syncSubscriptionState({ silent: true });
  selectedSubscription.value =
    subscriptionRecords.value.find((record) => String(record.subject_id) === String(id)) ||
    fallbackRecord ||
    selectedSubscription.value;
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
