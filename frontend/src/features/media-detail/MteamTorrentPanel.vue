<template>
  <section class="mteam-torrent-panel">
    <div v-if="mteam.mediaType === 'tv' && mteam.seasons.length" class="mteam-tv-target">
      <label>
        搜索季
        <select
          class="select select-bordered select-sm"
          :value="mteam.seasonNumber || ''"
          @change="emit('select-season', Number($event.target.value))"
        >
          <option
            v-for="season in mteam.seasons"
            :key="season.season_number"
            :value="season.season_number"
          >
            第 {{ season.season_number }} 季{{
              season.episode_count ? `（${season.episode_count} 集）` : ""
            }}
          </option>
        </select>
      </label>
      <span class="subtle">识别单集、部分合集和整季合集后再允许推送</span>
    </div>
    <div class="mteam-actions">
      <template v-if="mteam.sources.length">
        <span class="mteam-actions-label subtle">M-Team</span>
        <div class="mteam-tablist tabs tabs-boxed" role="tablist" aria-label="M-Team 检索路径">
          <button
            v-for="source in mteam.sources"
            :key="source.source"
            type="button"
            class="mteam-tab tab"
            :class="{ 'is-active tab-active': mteam.activeSource === source.source }"
            role="tab"
            @click="emit('select-source', source.source)"
          >
            {{ source.label }}
          </button>
        </div>
      </template>
      <span v-else class="subtle">缺少 IMDb / 豆瓣 ID，且无原标题，无法在 M-Team 检索</span>
    </div>
    <div class="torrent-list">
      <div v-if="mteam.loading" class="inline-loading" role="status">
        <div class="spinner spinner-sm"></div>
        <span>正在加载 M-Team…</span>
      </div>
      <p v-else-if="mteam.error" class="empty-hint">加载失败：{{ mteam.error }}</p>
      <template v-else-if="mteam.rows.length">
        <h4 class="torrent-list-title">M-Team 种子</h4>
        <div class="torrent-cards">
          <article
            v-for="torrent in mteam.rows"
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
                <div
                  v-if="torrent.tv_match"
                  class="torrent-tv-match"
                  :class="{ 'is-incompatible': !torrent.tv_match.compatible }"
                >
                  {{ torrent.tv_match.label
                  }}<template v-if="!torrent.tv_match.compatible">
                    · 不属于当前季或集数未知
                  </template>
                </div>
                <div class="torrent-stats">{{ torrentStats(torrent) }}</div>
                <div class="torrent-desc">{{ torrent.small_description || "" }}</div>
              </a>
              <div class="torrent-card-actions">
                <button
                  v-if="torrent.id && (!torrent.tv_match || torrent.tv_match.compatible)"
                  type="button"
                  class="btn btn-sm btn-primary torrent-push-trigger"
                  @click.prevent.stop="emit('push-torrent', torrent, $event.currentTarget)"
                >
                  推送 qB
                </button>
                <span
                  v-else
                  class="subtle torrent-push-hint"
                  :title="torrent.id ? '集数无法安全识别，请核对标题' : '无种子 ID，无法推送'"
                  >—</span
                >
              </div>
            </div>
          </article>
        </div>
      </template>
      <p v-else-if="mteam.activeSource" class="empty-hint">
        未返回种子列表（请检查 M-Team 返回结构或账号权限）。
      </p>
    </div>
  </section>
</template>

<script setup>
import { mteamTorrentWebUrl, torrentStats } from "./domain.js";

defineProps({
  mteam: { type: Object, required: true },
});

const emit = defineEmits(["select-source", "select-season", "push-torrent"]);
</script>
