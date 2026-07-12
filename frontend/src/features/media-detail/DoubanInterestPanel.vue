<template>
  <section class="douban-mark-panel">
    <div class="douban-mark-head">
      <h4>豆瓣标记</h4>
      <span class="douban-mark-status subtle" aria-live="polite">{{ interest.status }}</span>
    </div>
    <p v-if="interest.error" class="empty-hint">加载失败：{{ interest.error }}</p>
    <div class="douban-mark-controls">
      <div class="douban-mark-mode tabs tabs-boxed" role="group" aria-label="豆瓣标记状态">
        <button
          type="button"
          class="mteam-tab tab"
          :class="{ 'is-active tab-active': interest.mark.interest === 'wish' }"
          @click="emit('set-interest', 'wish')"
        >
          想看
        </button>
        <button
          type="button"
          class="mteam-tab tab"
          :class="{ 'is-active tab-active': interest.mark.interest === 'collect' }"
          @click="emit('set-interest', 'collect')"
        >
          看过
        </button>
      </div>
      <label v-if="interest.mark.interest === 'collect'" class="douban-rating-select">
        <span>评分</span>
        <select
          :value="interest.mark.rating"
          class="select select-bordered select-sm"
          :title="interest.ratingLabel"
          @change="emit('update-rating', $event.target.value)"
        >
          <option value="">未评分</option>
          <option value="5">5 星</option>
          <option value="4">4 星</option>
          <option value="3">3 星</option>
          <option value="2">2 星</option>
          <option value="1">1 星</option>
        </select>
      </label>
      <button
        type="button"
        class="btn btn-sm btn-primary"
        :disabled="interest.saveDisabled"
        @click="emit('save-interest')"
      >
        保存
      </button>
    </div>
    <label class="douban-tag-input">
      <span>{{ interest.mark.interest === "wish" ? "订阅分类" : "标签" }}</span>
      <select
        v-if="interest.mark.interest === 'wish'"
        :value="interest.mark.category"
        class="select select-bordered select-sm"
        :title="interest.categoryLabel"
        @change="emit('update-category', $event.target.value)"
      >
        <option value="">
          {{ interest.categories.length ? "选择订阅分类" : "未配置订阅分类" }}
        </option>
        <option
          v-for="category in interest.categories"
          :key="category.wanted_tag"
          :value="category.wanted_tag"
        >
          {{ category.name || category.wanted_tag }} · {{ category.wanted_tag }}
        </option>
      </select>
      <input
        v-else
        :value="interest.mark.tags"
        type="text"
        class="input input-bordered input-sm"
        autocomplete="off"
        spellcheck="false"
        placeholder="可选，例如：想补、冷门、家人一起看"
        @input="emit('update-tags', $event.target.value)"
      />
    </label>
    <div v-if="interest.tagHistory.length" class="douban-tag-history" aria-live="polite">
      <button
        v-for="tag in interest.tagHistory.slice(0, 24)"
        :key="tag"
        type="button"
        class="douban-tag-chip btn btn-xs btn-ghost"
        @click="emit('tag-suggestion', tag)"
      >
        {{ tag }}
      </button>
    </div>
  </section>
</template>

<script setup>
defineProps({
  interest: { type: Object, required: true },
});

const emit = defineEmits([
  "set-interest",
  "update-rating",
  "update-category",
  "update-tags",
  "save-interest",
  "tag-suggestion",
]);
</script>
