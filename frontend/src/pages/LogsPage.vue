<template>
  <section id="page-logs" class="app-page is-active">
    <div v-if="error" id="logs-error" class="banner err alert alert-error" role="alert">
      {{ error }}
    </div>
    <div v-if="toast.message" id="logs-toast" class="app-toast" role="status" aria-live="polite">
      <div class="app-toast-message" :class="toast.kind === 'err' ? 'toast-err' : 'toast-ok'">
        {{ toast.message }}
      </div>
    </div>

    <header class="top logs-top">
      <h1>日志</h1>
      <p class="sub">操作事件、结果状态与关联对象</p>
      <div class="actions">
        <button
          type="button"
          class="btn btn-secondary"
          :disabled="operationLogsLoading"
          @click="applyOperationLogFilters"
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
          @change="applyOperationLogFilters"
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
          @change="applyOperationLogFilters"
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
          @keydown.enter="applyOperationLogFilters"
        />
      </label>
      <div class="operation-log-filter-actions">
        <button
          type="button"
          class="btn btn-primary"
          :disabled="operationLogsLoading"
          @click="applyOperationLogFilters"
        >
          筛选
        </button>
        <button type="button" class="btn btn-ghost" @click="resetOperationLogFilters">重置</button>
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
</template>

<script setup>
import { onBeforeUnmount } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  formatOperationLogTime as formatUnixSeconds,
  OPERATION_LOG_CATEGORIES,
  OPERATION_LOG_STATUSES,
  operationLogActionLabel,
  operationLogCategoryLabel,
  operationLogMatchedKeywords,
  operationLogMatchStats,
  operationLogRelated,
  operationLogRuleEvaluationSummary,
  operationLogStatusClass as normalizedStatus,
  operationLogStatusLabel,
  operationLogTarget,
  operationLogTorrentMatches,
} from "../features/logs/domain.js";
import { createLogsRouteSync } from "../features/logs/route.js";
import { createLogsStore } from "../features/logs/store.js";

const route = useRoute();
const router = useRouter();
const logsStore = createLogsStore();
const {
  entries: operationLogs,
  filters: operationLogFilters,
  page: operationLogPage,
  loading: operationLogsLoading,
  lastError: error,
  toast,
  summary: operationLogSummary,
} = logsStore;
const operationLogCategories = OPERATION_LOG_CATEGORIES;
const operationLogStatuses = OPERATION_LOG_STATUSES;
const routeSync = createLogsRouteSync({ route, router, store: logsStore });

onBeforeUnmount(() => {
  routeSync.dispose();
  logsStore.dispose();
});

function applyOperationLogFilters() {
  return routeSync.applyFilters();
}

function loadMoreOperationLogs() {
  return logsStore.loadMore();
}

function resetOperationLogFilters() {
  return routeSync.resetFilters();
}
</script>

<style src="../features/logs/styles.css"></style>
