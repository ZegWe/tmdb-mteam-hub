<template>
  <img v-if="primary.poster" :src="primary.poster" alt="" />
  <h3>{{ primary.title }}</h3>
  <div class="detail-type-line">
    <span class="tag">{{ mediaTypeLabel }}</span>
    <span v-if="primary.date" class="tag">{{ primary.date }}</span>
  </div>
  <div v-if="primary.externalLinks.length" class="detail-external-ids">
    <a
      v-for="link in primary.externalLinks"
      :key="link.href"
      class="detail-ext-link tag"
      :href="link.href"
      target="_blank"
      rel="noopener noreferrer"
      >{{ link.label }}</a
    >
  </div>
</template>

<script setup>
import { computed } from "vue";

const props = defineProps({
  primary: { type: Object, required: true },
});

const mediaTypeLabel = computed(() => {
  if (props.primary.mediaType === "douban") return "豆瓣";
  return props.primary.mediaType === "tv" ? "剧集" : "电影";
});
</script>
