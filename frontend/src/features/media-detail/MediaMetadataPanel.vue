<template>
  <p v-if="primary.data.tagline" class="tagline-block">{{ primary.data.tagline }}</p>
  <dl v-if="primary.metaRows.length" class="detail-meta">
    <div v-for="row in primary.metaRows" :key="row.label" class="detail-meta-row">
      <dt>{{ row.label }}</dt>
      <dd>
        <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
        <span v-else>{{ row.value }}</span>
      </dd>
    </div>
  </dl>
</template>

<script setup>
defineProps({
  primary: { type: Object, required: true },
});
</script>
