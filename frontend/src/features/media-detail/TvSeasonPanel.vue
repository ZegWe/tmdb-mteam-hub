<template>
  <section class="tv-seasons-mount">
    <h4 class="tv-episodes-heading">分集</h4>
    <p v-if="!seasons.length" class="subtle">暂无分季信息</p>
    <div v-else class="tv-seasons-list" role="list">
      <details
        v-for="season in seasons"
        :key="season.season_number"
        class="tv-season-block"
        @toggle="onSeasonToggle($event, season.season_number)"
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
            v-if="loading[season.season_number]"
            class="inline-loading tv-season-loading"
            role="status"
          >
            <div class="spinner spinner-sm"></div>
            <span>加载中…</span>
          </div>
          <p v-else-if="errors[season.season_number]" class="empty-hint">
            加载失败：{{ errors[season.season_number] }}
          </p>
          <p v-else-if="!episodes[season.season_number]" class="subtle tv-season-placeholder">
            展开以加载本季分集…
          </p>
          <template v-else>
            <p v-if="!episodes[season.season_number].length" class="subtle">本季暂无分集数据</p>
            <div
              v-for="episode in episodes[season.season_number]"
              :key="episode.episode_number"
              class="tv-episode-row"
            >
              <div class="tv-ep-thumb">
                <img
                  v-if="episodeStill(episode)"
                  class="tv-ep-still"
                  :src="episodeStill(episode)"
                  alt=""
                  loading="lazy"
                />
                <div v-else class="tv-ep-still tv-ep-still-placeholder" aria-hidden="true"></div>
              </div>
              <div class="tv-ep-main">
                <div class="tv-ep-title-line">
                  <span class="tv-ep-num">E{{ episode.episode_number ?? "—" }}</span>
                  <span class="tv-ep-title">{{
                    episode.name || `第 ${episode.episode_number} 集`
                  }}</span>
                  <span v-if="episode.air_date" class="tv-ep-air">{{ episode.air_date }}</span>
                </div>
                <p v-if="episode.overview" class="tv-ep-overview">{{ episode.overview }}</p>
              </div>
            </div>
          </template>
        </div>
      </details>
    </div>
  </section>
</template>

<script setup>
import { episodeStill } from "./domain.js";

defineProps({
  seasons: { type: Array, required: true },
  episodes: { type: Object, required: true },
  loading: { type: Object, required: true },
  errors: { type: Object, required: true },
});

const emit = defineEmits(["load-season"]);

function onSeasonToggle(event, seasonNumber) {
  if (event.target.open) emit("load-season", seasonNumber);
}
</script>
