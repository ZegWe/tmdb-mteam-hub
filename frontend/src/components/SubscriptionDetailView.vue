<template>
  <article
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
      class="subscription-state-graph"
      :aria-label="`订阅状态：${subscriptionDisplayStatus(selectedSubscription).text}`"
    >
      <div
        v-for="node in subscriptionLifecycleNodes(selectedSubscription)"
        :key="node.key"
        class="subscription-state-node"
        :class="[
          `subscription-state-node-${node.state}`,
          node.attention ? `subscription-state-node-${node.attention}` : '',
        ]"
      >
        <span class="subscription-state-dot" aria-hidden="true"></span>
        <span class="subscription-state-label">{{ node.label }}</span>
        <span v-if="node.attention" class="subscription-state-attention">{{
          node.attention === "waiting_release"
            ? "等待发布"
            : node.attention === "retry_blocked"
              ? "阻塞"
              : node.attention === "skipped"
                ? "跳过"
                : "失败"
        }}</span>
      </div>
    </div>
    <dl class="detail-meta">
      <div
        v-for="row in subscriptionDetailRows(selectedSubscription)"
        :key="row.label"
        class="detail-meta-row"
      >
        <dt>{{ row.label }}</dt>
        <dd>
          <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
          <span v-else>{{ row.value }}</span>
        </dd>
      </div>
    </dl>
    <div class="row-actions">
      <button
        type="button"
        class="btn btn-secondary"
        :disabled="subscriptionActionLoading || !canRetrySubscription(selectedSubscription)"
        @click="retrySubscriptionCurrent(selectedSubscription.subject_id)"
      >
        重试当前节点
      </button>
      <button
        type="button"
        class="btn btn-ghost"
        :disabled="subscriptionActionLoading || !canRerunSubscription(selectedSubscription)"
        @click="rerunSubscription(selectedSubscription.subject_id)"
      >
        重跑任务
      </button>
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
      <div
        v-if="subscriptionProgress(selectedSubscription) != null"
        class="subscription-detail-download-progress"
      >
        <div
          class="subscription-progress"
          :aria-label="`下载进度 ${formatPercent(subscriptionProgress(selectedSubscription))}`"
        >
          <span
            :style="{
              width: `${Math.round(subscriptionProgress(selectedSubscription) * 100)}%`,
            }"
          ></span>
        </div>
        <span>{{ formatPercent(subscriptionProgress(selectedSubscription)) }}</span>
      </div>
      <dl v-if="selectedSubscription.last_push" class="detail-meta">
        <div
          v-for="row in pushRows(selectedSubscription.last_push)"
          :key="row.label"
          class="detail-meta-row"
        >
          <dt>{{ row.label }}</dt>
          <dd>
            <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
            <span v-else>{{ row.value }}</span>
          </dd>
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
          <dd>
            <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
            <span v-else>{{ row.value }}</span>
          </dd>
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
</template>

<script setup>
defineProps({
  selectedSubscription: { type: Object, required: true },
  subscriptionDisplayStatus: { type: Function, required: true },
  subscriptionLifecycleNodes: { type: Function, required: true },
  subscriptionDetailRows: { type: Function, required: true },
  subscriptionActionLoading: { type: Boolean, default: false },
  canRetrySubscription: { type: Function, required: true },
  retrySubscriptionCurrent: { type: Function, required: true },
  canRerunSubscription: { type: Function, required: true },
  rerunSubscription: { type: Function, required: true },
  refreshSubscriptionProgress: { type: Function, required: true },
  checkSubscriptionCompletion: { type: Function, required: true },
  subscriptionProgress: { type: Function, required: true },
  formatPercent: { type: Function, required: true },
  pushRows: { type: Function, required: true },
  subscriptionEpisodes: { type: Array, default: () => [] },
  pushStatusLabel: { type: Function, required: true },
  completionRows: { type: Function, required: true },
  subscriptionFiles: { type: Array, default: () => [] },
  formatBytes: { type: Function, required: true },
});
</script>
