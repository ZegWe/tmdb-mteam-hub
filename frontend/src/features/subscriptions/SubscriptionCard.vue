<template>
  <article
    class="subscription-card card bg-base-100 border border-base-300 shadow-sm"
    role="link"
    tabindex="0"
    :aria-label="`打开订阅 ${title}`"
    :data-subscription-id="record.subject_id || undefined"
    @click="open"
    @keydown.enter.prevent="open"
  >
    <div class="subscription-card-cover">
      <img :src="imageUrl || transparentPixel" alt="" loading="lazy" />
      <span class="subscription-status badge" :class="`subscription-status-${displayStatus.key}`">{{
        displayStatus.text
      }}</span>
    </div>
    <div class="meta subscription-card-meta">
      <div class="title">{{ title }}</div>
      <div class="subtle">{{ subtitle }}</div>
    </div>
  </article>
</template>

<script setup>
import { computed } from "vue";
import { itemImageUrl } from "../../shared/media/images.js";
import {
  subscriptionCardSubtitle,
  subscriptionDisplayStatus,
} from "./domain.js";

const transparentPixel =
  "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

const props = defineProps({
  record: { type: Object, required: true },
});
const emit = defineEmits(["open"]);
const title = computed(() => props.record.title || props.record.subject_id || "未命名订阅");
const subtitle = computed(() => subscriptionCardSubtitle(props.record));
const imageUrl = computed(() => itemImageUrl(props.record));
const displayStatus = computed(() => subscriptionDisplayStatus(props.record));

function open() {
  emit("open", props.record);
}
</script>
