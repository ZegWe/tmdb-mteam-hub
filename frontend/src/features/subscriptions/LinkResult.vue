<template>
  <section
    v-for="link in links"
    :key="link.key"
    class="subscription-detail-section subscription-link-artifact"
  >
    <h4>硬链接</h4>
    <p v-if="links.length > 1" class="hint">链接任务 {{ link.label }}</p>
    <dl class="detail-meta">
      <div v-for="row in link.rows" :key="row.label" class="detail-meta-row">
        <dt>{{ row.label }}</dt>
        <dd>
          <a v-if="row.href" :href="row.href" target="_blank" rel="noreferrer">{{ row.value }}</a>
          <span v-else>{{ row.value }}</span>
        </dd>
      </div>
    </dl>
  </section>
  <section v-if="files.length" class="subscription-detail-section">
    <h4>文件</h4>
    <div class="subscription-file-list">
      <div
        v-for="file in files"
        :key="file.name || file.target_path || file.source_path"
        class="subscription-file-row"
      >
        <div class="subscription-file-main">
          <span class="subscription-file-name">{{
            file.name || file.target_path || file.source_path
          }}</span>
          <span v-if="file.error || file.source_path || file.size" class="subscription-file-note">{{
            file.error || file.source_path || formatBytes(file.size)
          }}</span>
        </div>
        <span class="subscription-file-status">{{
          file.status || (file.progress != null ? formatPercent(file.progress) : "")
        }}</span>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed } from "vue";
import { formatBytes, formatPercent } from "../../shared/lib/formatters.js";
import { linkArtifactRows } from "./domain.js";

const props = defineProps({
  record: { type: Object, required: true },
});

const links = computed(() => {
  const nested = Array.isArray(props.record?.links) ? props.record.links : [];
  return nested.map((link, index) => ({
    key: link.id || `link-${index}`,
    label: link.target_dir || link.id || String(index + 1),
    rows: linkArtifactRows(link),
  }));
});

const files = computed(() => {
  const nestedDownloads = Array.isArray(props.record?.downloads) ? props.record.downloads : [];
  const nestedLinks = Array.isArray(props.record?.links) ? props.record.links : [];
  return [
    ...nestedDownloads.flatMap((download) =>
      (Array.isArray(download?.files) ? download.files : []).map((file) => ({
        ...file,
        status: file.progress != null ? "" : download.state,
      })),
    ),
    ...nestedLinks.flatMap((link) =>
      (Array.isArray(link?.files) ? link.files : []).map((file) => ({
        ...file,
        status: file.outcome || link.state,
      })),
    ),
  ].slice(0, 120);
});
</script>
