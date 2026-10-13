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
      v-if="candidates.length || subscriptionCapability.schedulable"
      class="subscription-detail-section subscription-candidate-list"
    >
      <h4>候选种子</h4>
      <form
        class="flex flex-wrap items-center gap-2"
        @submit.prevent="$emit('search-candidates', searchKeyword, searchSource)"
      >
        <select
          v-model="searchSource"
          class="select select-bordered select-sm"
          aria-label="种子搜索方式"
          :disabled="searching"
        >
          <option value="keyword">名称</option>
          <option value="douban">豆瓣 ID</option>
          <option value="imdb">IMDb ID</option>
        </select>
        <input
          v-model="searchKeyword"
          class="input input-bordered input-sm"
          aria-label="种子搜索内容"
          :placeholder="
            searchSource === 'douban'
              ? '例如 1292052'
              : searchSource === 'imdb'
                ? '例如 tt0111161'
                : '输入名称、年份或季集数'
          "
          maxlength="256"
        />
        <button
          type="submit"
          class="btn btn-secondary btn-sm"
          :disabled="
            searching ||
            confirming ||
            retrying ||
            !searchKeyword.trim() ||
            selectedSubscription.replacement_pending ||
            selectedSubscription.execution_state === 'running' ||
            !subscriptionCapability.schedulable
          "
        >
          {{ searching ? "搜索中…" : "查找其他种子" }}
        </button>
      </form>
      <p v-if="selectedSubscription.replacement_pending" class="hint" role="status">
        正在换种：清理旧下载和链接后从新种子开始。清理失败时可重跑继续。
      </p>
      <p v-if="hasPreviousArtifacts" class="hint" role="note">
        可手动改选其他种子。换种会清理本订阅旧 qB 任务、下载文件和链接；剧集将从目标起始集重新下载。
      </p>
      <p v-if="needsConfirmation" class="hint" role="note">
        手动搜索结果不会自动下载。请核对名称、年份和季集数，再选中需要下载的种子。
      </p>
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
        <div class="flex items-center gap-2">
          <span class="subscription-file-status">{{
            candidate.user_confirmed
              ? "已手动确认"
              : manualSourceLabels[candidate.source]
                ? `${manualSourceLabels[candidate.source]} · 待确认`
                : candidate.selected
                  ? "已选择"
                  : candidate.source || "候选"
          }}</span>
          <button
            v-if="canConfirm(candidate)"
            type="button"
            class="btn btn-primary btn-sm"
            :disabled="
              confirming ||
              searching ||
              retrying ||
              selectedSubscription.execution_state === 'running'
            "
            @click="
              $emit(
                'confirm-candidate',
                candidate.torrent_id,
                hasPreviousArtifacts && !canContinueTv,
              )
            "
          >
            {{
              confirming
                ? "提交中…"
                : hasPreviousArtifacts && !canContinueTv
                  ? "清理旧任务并换种"
                  : "选中并下载"
            }}
          </button>
          <button
            v-if="canConfirm(candidate) && hasPreviousArtifacts && canContinueTv"
            type="button"
            class="btn btn-secondary btn-sm"
            :disabled="
              confirming ||
              searching ||
              retrying ||
              selectedSubscription.execution_state === 'running'
            "
            @click="$emit('confirm-candidate', candidate.torrent_id, true)"
          >
            清理旧任务并换种
          </button>
        </div>
      </article>
    </section>
    <DownloadTaskList :record="selectedSubscription" />
    <LinkResult :record="selectedSubscription" />
  </article>
</template>

<script setup>
import { computed, ref, watch } from "vue";
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
  confirming: { type: Boolean, default: false },
  searching: { type: Boolean, default: false },
});

defineEmits(["retry", "confirm-candidate", "search-candidates"]);
const searchKeyword = ref("");
const searchSource = ref("keyword");
const manualSourceLabels = {
  keyword: "名字匹配",
  manual_douban: "豆瓣 ID",
  manual_imdb: "IMDb ID",
};
watch(searchSource, (source) => {
  searchKeyword.value =
    source === "keyword"
      ? props.selectedSubscription.title || ""
      : source === "douban"
        ? props.selectedSubscription.subject_id || ""
        : "";
});
watch(
  () => props.selectedSubscription.subject_id,
  () => {
    searchSource.value = "keyword";
    searchKeyword.value = props.selectedSubscription.title || "";
  },
  { immediate: true },
);

const subscriptionCapability = computed(() => subscriptionCapabilities(props.selectedSubscription));
const issues = computed(() =>
  Array.isArray(props.selectedSubscription?.issues) ? props.selectedSubscription.issues : [],
);
const candidates = computed(() =>
  Array.isArray(props.selectedSubscription?.candidates)
    ? props.selectedSubscription.candidates
    : [],
);
const needsConfirmation = computed(() =>
  candidates.value.some(
    (candidate) => manualSourceLabels[candidate.source] && !candidate.user_confirmed,
  ),
);
const hasPreviousArtifacts = computed(() =>
  Boolean(props.selectedSubscription.downloads?.length || props.selectedSubscription.links?.length),
);
const canContinueTv = computed(
  () =>
    props.selectedSubscription.media_kind === "tv" &&
    props.selectedSubscription.lifecycle_state === "searching",
);

function canConfirm(candidate) {
  return (
    candidate.torrent_id &&
    !candidate.user_confirmed &&
    ["searching", "downloading", "linking", "completed"].includes(
      props.selectedSubscription.lifecycle_state,
    ) &&
    !props.selectedSubscription.replacement_pending &&
    subscriptionCapability.value.schedulable &&
    !(props.selectedSubscription.downloads || []).some(
      (download) =>
        download.torrent_id === candidate.torrent_id &&
        !["failed", "ignored", "superseded"].includes(download.state),
    )
  );
}

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
