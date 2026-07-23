<template>
  <article class="d-head">
    <MediaPrimaryPanel :primary="model.primary" />
    <DoubanInterestPanel
      v-if="model.primary.doubanId"
      :interest="model.interest"
      @set-interest="emit('set-interest', $event)"
      @update-rating="emit('update-rating', $event)"
      @update-category="emit('update-category', $event)"
      @update-tags="emit('update-tags', $event)"
      @save-interest="emit('save-interest')"
      @tag-suggestion="emit('tag-suggestion', $event)"
    />
    <MediaMetadataPanel :primary="model.primary" />
    <TvSeasonPanel
      v-if="model.primary.mediaType === 'tv'"
      :seasons="model.primary.seasons"
      :episodes="model.seasonEpisodes"
      :loading="model.seasonLoading"
      :errors="model.seasonErrors"
      @load-season="emit('load-season', $event)"
    />
    <p class="overview">{{ model.primary.overview }}</p>
    <MteamTorrentPanel
      :mteam="model.mteam"
      @select-source="emit('select-torrent-source', $event)"
      @select-season="emit('select-torrent-season', $event)"
      @push-torrent="(torrent, trigger) => emit('push-torrent', torrent, trigger)"
    />
  </article>
</template>

<script setup>
import DoubanInterestPanel from "../features/media-detail/DoubanInterestPanel.vue";
import MediaMetadataPanel from "../features/media-detail/MediaMetadataPanel.vue";
import MediaPrimaryPanel from "../features/media-detail/MediaPrimaryPanel.vue";
import MteamTorrentPanel from "../features/media-detail/MteamTorrentPanel.vue";
import TvSeasonPanel from "../features/media-detail/TvSeasonPanel.vue";

defineProps({
  model: { type: Object, required: true },
});

const emit = defineEmits([
  "set-interest",
  "update-rating",
  "update-category",
  "update-tags",
  "save-interest",
  "tag-suggestion",
  "load-season",
  "select-torrent-source",
  "select-torrent-season",
  "push-torrent",
]);
</script>
