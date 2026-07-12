<template>
  <div class="subscription-state-graph" :aria-label="`订阅状态：${displayStatus.text}`">
    <div
      v-for="node in nodes"
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
        attentionText(node.attention)
      }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed } from "vue";
import { subscriptionDisplayStatus, subscriptionLifecycleNodes } from "./domain.js";

const ATTENTION_TEXT = Object.freeze({
  waiting_release: "等待发布",
  retry_blocked: "阻塞",
  skipped: "跳过",
  failed: "失败",
});

const props = defineProps({
  record: { type: Object, required: true },
});
const nodes = computed(() => subscriptionLifecycleNodes(props.record));
const displayStatus = computed(() => subscriptionDisplayStatus(props.record));

function attentionText(value) {
  return ATTENTION_TEXT[value] || "失败";
}
</script>
