<script setup lang="ts">
// Live "good first issue" rows from /api/good-first-issues.
// limit=0 (default) shows everything; a positive limit makes a teaser.
const props = withDefaults(defineProps<{ limit?: number }>(), { limit: 0 })
const { data: issues } = await useFetch('/api/good-first-issues', { default: () => [] })
const shown = computed(() => (props.limit ? issues.value.slice(0, props.limit) : issues.value))
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
</script>

<template>
  <div v-if="shown && shown.length" class="board">
    <a v-for="i in shown" :key="i.number" class="issue" :href="i.url" target="_blank" rel="noopener">
      <span class="n">#{{ i.number }}</span>
      <span class="ti">{{ i.title }}</span>
      <span v-for="l in i.labels.slice(0, 2)" :key="l" class="lb">{{ l }}</span>
    </a>
  </div>
  <div v-else class="board">
    <div class="empty">No open “good first issue” tickets right now — check <a :href="`${repo}/issues`">all issues</a> or open one.</div>
  </div>
</template>
