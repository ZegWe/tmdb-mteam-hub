<template>
  <section class="subscription-detail-section">
    <h4>下载任务</h4>
    <div v-if="progress != null" class="subscription-detail-download-progress">
      <div class="subscription-progress" :aria-label="`下载进度 ${formatPercent(progress)}`">
        <span :style="{ width: `${Math.round(progress * 100)}%` }"></span>
      </div>
      <span>{{ formatPercent(progress) }}</span>
    </div>
    <div v-if="tasks.length" class="subscription-task-list">
      <details
        v-for="task in tasks"
        :key="task.key"
        class="subscription-task-panel"
      >
        <summary class="subscription-task-summary">
          <span class="subscription-task-episode">{{
            task.episodeLabel || "未识别分集"
          }}</span>
          <span class="subscription-task-title">{{ task.label }}</span>
          <span class="subscription-task-state">{{
            pushStatusLabel(task.state)
          }}</span>
        </summary>
        <div class="subscription-task-body">
          <dl class="detail-meta">
            <div
              v-for="row in task.downloadRows"
              :key="row.label"
              class="detail-meta-row"
            >
              <dt>{{ row.label }}</dt>
              <dd>
                <a
                  v-if="row.href"
                  :href="row.href"
                  target="_blank"
                  rel="noreferrer"
                  >{{ row.value }}</a
                >
                <span v-else>{{ row.value }}</span>
              </dd>
            </div>
          </dl>
          <div
            v-for="link in task.matchedLinks"
            :key="link.key"
            class="subscription-link-block"
          >
            <p class="hint">硬链接 {{ link.label }}</p>
            <dl class="detail-meta">
              <div
                v-for="row in link.rows"
                :key="row.label"
                class="detail-meta-row"
              >
                <dt>{{ row.label }}</dt>
                <dd>
                  <a
                    v-if="row.href"
                    :href="row.href"
                    target="_blank"
                    rel="noreferrer"
                    >{{ row.value }}</a
                  >
                  <span v-else>{{ row.value }}</span>
                </dd>
              </div>
            </dl>
          </div>
          <div
            v-if="task.allFiles.length"
            class="subscription-task-files"
          >
            <div
              v-for="(file, fileIndex) in task.allFiles.slice(0, 80)"
              :key="file.name || file.target_path || file.source_path || fileIndex"
              class="subscription-file-row"
            >
              <div class="subscription-file-main">
                <span class="subscription-file-name">{{
                  file.name || file.target_path || file.source_path
                }}</span>
                <span
                  v-if="
                    file.error || file.source_path || file.size
                  "
                  class="subscription-file-note"
                  >{{
                    file.error ||
                    file.source_path ||
                    formatBytes(file.size)
                  }}</span
                >
              </div>
              <span class="subscription-file-status">{{
                file.status ||
                (file.progress != null
                  ? formatPercent(file.progress)
                  : "")
              }}</span>
            </div>
            <p
              v-if="task.allFiles.length > 80"
              class="empty-hint"
            >
              已截断：仅显示前 80 个文件（共
              {{ task.allFiles.length }} 个）
            </p>
          </div>
        </div>
      </details>
    </div>
    <p v-else class="empty-hint">暂无下载任务</p>
  </section>
  <section
    v-if="orphanLinks.length"
    class="subscription-detail-section"
  >
    <h4>独立硬链接</h4>
    <details
      v-for="link in orphanLinks"
      :key="link.key"
      class="subscription-task-panel"
    >
      <summary class="subscription-task-summary">
        <span class="subscription-task-episode">—</span>
        <span class="subscription-task-title">{{ link.label }}</span>
        <span class="subscription-task-state">{{ pushStatusLabel(link.state) }}</span>
      </summary>
      <div class="subscription-task-body">
        <dl class="detail-meta">
          <div
            v-for="row in link.rows"
            :key="row.label"
            class="detail-meta-row"
          >
            <dt>{{ row.label }}</dt>
            <dd>
              <a
                v-if="row.href"
                :href="row.href"
                target="_blank"
                rel="noreferrer"
                >{{ row.value }}</a
              >
              <span v-else>{{ row.value }}</span>
            </dd>
          </div>
        </dl>
        <div
          v-if="link.files && link.files.length"
          class="subscription-task-files"
        >
          <div
            v-for="(file, fileIndex) in link.files.slice(0, 80)"
            :key="
              file.name ||
              file.target_path ||
              file.source_path ||
              fileIndex
            "
            class="subscription-file-row"
          >
            <div class="subscription-file-main">
              <span class="subscription-file-name">{{
                file.name || file.target_path || file.source_path
              }}</span>
              <span
                v-if="
                  file.error || file.source_path || file.size
                "
                class="subscription-file-note"
                >{{
                  file.error ||
                  file.source_path ||
                  formatBytes(file.size)
                }}</span
              >
            </div>
            <span class="subscription-file-status">{{
              file.status ||
              (file.progress != null
                ? formatPercent(file.progress)
                : "")
            }}</span>
          </div>
          <p
            v-if="link.files.length > 80"
            class="empty-hint"
          >
            已截断：仅显示前 80 个文件（共
            {{ link.files.length }} 个）
          </p>
        </div>
      </div>
    </details>
  </section>
</template>

<script setup>
import { computed } from "vue";
import { formatBytes, formatPercent } from "../../shared/lib/formatters.js";
import {
  matchLinksToDownloads,
  pushStatusLabel,
  subscriptionProgress,
} from "./domain.js";

const props = defineProps({
  record: { type: Object, required: true },
});

const progress = computed(() => subscriptionProgress(props.record));

const grouped = computed(() =>
  matchLinksToDownloads(props.record?.downloads, props.record?.links),
);
const tasks = computed(() => grouped.value.tasks);
const orphanLinks = computed(() => grouped.value.orphanLinks);
</script>
