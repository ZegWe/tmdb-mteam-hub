<template>
  <section class="subscription-detail-section">
    <h4>下载</h4>
    <div v-if="progress != null" class="subscription-detail-download-progress">
      <div class="subscription-progress" :aria-label="`下载进度 ${formatPercent(progress)}`">
        <span :style="{ width: `${Math.round(progress * 100)}%` }"></span>
      </div>
      <span>{{ formatPercent(progress) }}</span>
    </div>
    <div v-if="downloads.length" class="subscription-download-list">
      <article
        v-for="download in downloads"
        :key="download.key"
        class="subscription-download-artifact"
      >
        <p v-if="downloads.length > 1" class="hint">下载任务 {{ download.label }}</p>
        <dl class="detail-meta">
          <div v-for="row in download.rows" :key="row.label" class="detail-meta-row">
            <dt>{{ row.label }}</dt>
            <dd>
              <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{
                row.value
              }}</a>
              <span v-else>{{ row.value }}</span>
            </dd>
          </div>
        </dl>
      </article>
    </div>
    <p v-else class="empty-hint">暂无下载任务</p>
  </section>
  <section v-if="episodes.length" class="subscription-detail-section">
    <h4>分集</h4>
    <div class="subscription-episode-list">
      <div
        v-for="episode in episodes"
        :key="episode.label || episode.episode_number"
        class="subscription-episode-row"
      >
        <span class="subscription-episode-title">{{ episode.label || "未识别分集" }}</span>
        <span class="subscription-episode-state">{{ pushStatusLabel(episode.status) }}</span>
        <div v-if="episode.progress != null" class="subscription-progress">
          <span :style="{ width: `${Math.round(Number(episode.progress) * 100)}%` }"></span>
        </div>
        <span class="subscription-episode-files">
          {{ episode.completed_file_count || episode.linked_file_count || 0 }}/{{
            episode.file_count || 0
          }}
        </span>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed } from "vue";
import { formatPercent } from "../../shared/lib/formatters.js";
import { downloadArtifactRows, pushStatusLabel, subscriptionProgress } from "./domain.js";

const props = defineProps({
  record: { type: Object, required: true },
});
const progress = computed(() => subscriptionProgress(props.record));
const downloads = computed(() => {
  const nested = Array.isArray(props.record?.downloads) ? props.record.downloads : [];
  return nested.map((download, index) => ({
    key: download.id || `${download.torrent_id || "download"}-${index}`,
    label: download.qb_name || download.torrent_title || download.id || String(index + 1),
    rows: downloadArtifactRows(download),
  }));
});
const episodes = computed(() => {
  const nestedDownloads = Array.isArray(props.record?.downloads) ? props.record.downloads : [];
  return nestedDownloads.flatMap((download) =>
    (Array.isArray(download?.files) ? download.files : [])
      .filter(
        (file) =>
          file?.episode_label ||
          Number.isInteger(file?.episode_number) ||
          Number.isInteger(file?.season_number),
      )
      .map((file) => ({
        ...file,
        label:
          file.episode_label ||
          [file.season_number, file.episode_number]
            .map((value) => (Number.isInteger(value) ? String(value).padStart(2, "0") : ""))
            .filter(Boolean)
            .join("E"),
        status: download.state,
        file_count: 1,
        completed_file_count: Number(file.progress) >= 1 ? 1 : 0,
      })),
  );
});
</script>
