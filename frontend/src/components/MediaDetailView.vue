<template>
  <article class="d-head">
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
        <span class="douban-mark-status subtle" aria-live="polite">{{ doubanInterestStatus }}</span>
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
          <select
            v-model="doubanMark.rating"
            class="select select-bordered select-sm"
            :title="selectedDoubanRatingLabel"
          >
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
          :title="selectedDoubanCategoryLabel"
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
        <dd>
          <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
          <span v-else>{{ row.value }}</span>
        </dd>
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
              >第 {{ season.season_number }} 季{{ season.name ? ` · ${season.name}` : "" }}</span
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
                  <div v-else class="tv-ep-still tv-ep-still-placeholder" aria-hidden="true"></div>
                </div>
                <div class="tv-ep-main">
                  <div class="tv-ep-title-line">
                    <span class="tv-ep-num">E{{ ep.episode_number ?? "—" }}</span>
                    <span class="tv-ep-title">{{ ep.name || `第 ${ep.episode_number} 集` }}</span>
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
                <span v-else class="subtle torrent-push-hint" title="无种子 ID，无法推送">—</span>
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
</template>

<script setup>
defineProps({
  detailPoster: { type: String, default: "" },
  detailTitle: { type: String, default: "" },
  detailMediaType: { type: String, default: "" },
  detailDate: { type: String, default: "" },
  externalLinks: { type: Array, default: () => [] },
  detailDoubanId: { type: String, default: "" },
  doubanInterestStatus: { type: String, default: "" },
  doubanMark: { type: Object, required: true },
  setDoubanInterest: { type: Function, required: true },
  selectedDoubanRatingLabel: { type: String, default: "" },
  doubanSaveDisabled: { type: Boolean, default: false },
  saveDoubanInterest: { type: Function, required: true },
  selectedDoubanCategoryLabel: { type: String, default: "" },
  subscriptionCategoriesCache: { type: Array, default: () => [] },
  doubanTagHistory: { type: Array, default: () => [] },
  applyDoubanTagSuggestion: { type: Function, required: true },
  detailData: { type: Object, required: true },
  detailMetaRows: { type: Array, default: () => [] },
  detailSeasons: { type: Array, default: () => [] },
  loadSeasonEpisodes: { type: Function, required: true },
  seasonLoading: { type: Object, required: true },
  seasonErrors: { type: Object, required: true },
  seasonEpisodes: { type: Object, required: true },
  episodeStill: { type: Function, required: true },
  detailOverview: { type: String, default: "" },
  mteamSources: { type: Array, default: () => [] },
  activeTorrentSource: { type: String, default: "" },
  selectTorrentSource: { type: Function, required: true },
  torrentsLoading: { type: Boolean, default: false },
  torrentError: { type: String, default: "" },
  torrentRows: { type: Array, default: () => [] },
  mteamTorrentWebUrl: { type: Function, required: true },
  torrentStats: { type: Function, required: true },
  openQbPushDialog: { type: Function, required: true },
});
</script>
