<template>
  <article
    class="subscription-detail"
    :data-subscription-detail-id="selectedSubscription.subject_id"
  >
    <div class="subscription-detail-head">
      <h3>{{ selectedSubscription.title || selectedSubscription.subject_id }}</h3>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <span
          class="subscription-status badge"
          :class="`subscription-status-${subscriptionDisplayStatus(selectedSubscription).key}`"
          >{{ subscriptionDisplayStatus(selectedSubscription).text }}</span
        >
        <span
          v-for="badge in subscriptionCapability.badges"
          :key="badge.key"
          class="badge badge-sm badge-outline"
          :class="{
            'badge-success': badge.tone === 'success',
            'badge-warning': badge.tone === 'warning',
            'badge-error': badge.tone === 'danger',
            'badge-ghost': badge.tone === 'muted',
          }"
          >{{ badge.text }}</span
        >
      </div>
    </div>
    <div class="subscription-detail-actions">
      <button
        type="button"
        class="btn btn-secondary"
        :disabled="retrying"
        @click="$emit('retry', selectedSubscription.subject_id)"
      >
        {{ retrying ? "重跑中…" : "重跑任务" }}
      </button>
    </div>
    <LifecycleGraph :record="selectedSubscription" />
    <p
      id="subscription-capability-note"
      class="hint rounded-lg border border-base-300 bg-base-200 p-3"
      role="note"
    >
      {{ subscriptionCapability.explanation }} 状态以后端最新数据为准。
    </p>
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
    <section v-if="issues.length" class="subscription-detail-section subscription-issue-list">
      <h4>诊断</h4>
      <article
        v-for="(issue, index) in issues"
        :key="`${issue.owner || 'issue'}-${issue.artifact_id || index}-${issue.occurred_at || 0}`"
        class="alert alert-warning mb-2 items-start"
      >
        <div>
          <strong>{{ issue.message }}</strong>
          <p class="hint">
            {{ issueOwnerText(issue) }}
            <template v-if="issue.operation"> · {{ issue.operation }}</template>
            <template v-if="issue.occurred_at">
              · {{ formatUnixSeconds(issue.occurred_at) }}</template
            >
          </p>
        </div>
      </article>
    </section>
    <section
      v-if="candidates.length"
      class="subscription-detail-section subscription-candidate-list"
    >
      <h4>候选种子</h4>
      <article
        v-for="candidate in candidates"
        :key="candidate.torrent_id"
        class="subscription-file-row"
      >
        <div class="subscription-file-main">
          <span class="subscription-file-name">{{ candidate.title || candidate.torrent_id }}</span>
          <span v-if="candidate.subtitle" class="subscription-file-note">{{
            candidate.subtitle
          }}</span>
          <span v-if="candidate.excluded_reason" class="subscription-file-note">{{
            candidate.excluded_reason
          }}</span>
        </div>
        <span class="subscription-file-status">{{
          candidate.selected ? "已选择" : candidate.source || "候选"
        }}</span>
      </article>
    </section>
    <DownloadTaskList :record="selectedSubscription" />
    <LinkResult :record="selectedSubscription" />
  </article>
</template>

<script setup>
import { computed } from "vue";
import { formatUnixSeconds } from "../shared/lib/formatters.js";
import DownloadTaskList from "../features/subscriptions/DownloadTaskList.vue";
import LinkResult from "../features/subscriptions/LinkResult.vue";
import LifecycleGraph from "../features/subscriptions/LifecycleGraph.vue";
import {
  subscriptionCapabilities,
  subscriptionDetailRows,
  subscriptionDisplayStatus,
} from "../features/subscriptions/domain.js";

const props = defineProps({
  selectedSubscription: { type: Object, required: true },
  retrying: { type: Boolean, default: false },
});

defineEmits(["retry"]);

const subscriptionCapability = computed(() => subscriptionCapabilities(props.selectedSubscription));
const issues = computed(() =>
  Array.isArray(props.selectedSubscription?.issues) ? props.selectedSubscription.issues : [],
);
const candidates = computed(() =>
  Array.isArray(props.selectedSubscription?.candidates)
    ? props.selectedSubscription.candidates
    : [],
);

function issueOwnerText(issue) {
  if (issue.owner === "download_artifact") return `下载任务 ${issue.artifact_id || ""}`.trim();
  if (issue.owner === "link_artifact") return `链接任务 ${issue.artifact_id || ""}`.trim();
  if (issue.owner === "tv_episode") {
    return `第 ${issue.season_number || "?"} 季第 ${issue.episode_number || "?"} 集`;
  }
  if (issue.owner === "tv_lane") return `TV ${issue.lane || "任务"}`;
  return "订阅";
}
</script>
