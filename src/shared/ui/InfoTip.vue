<script setup lang="ts">
import { Info } from 'lucide-vue-next'
/** 使用应用既有 TooltipLayer；键盘聚焦与指针悬停共享同一提示。 */
defineProps<{ text: string }>()
</script>
<template>
  <button type="button" class="info-tip" :title="text" :aria-label="text"><Info :size="14" aria-hidden="true" /></button>
</template>
<style scoped>
.info-tip { display:inline-flex; align-items:center; justify-content:center; flex:none; width:24px; height:24px; padding:0; border:0; border-radius:4px; background:transparent; color:var(--desk-muted,#57606a); vertical-align:middle; }
.info-tip:hover { background:var(--desk-soft,#f6f7f8); color:var(--desk-text,#24292f); }
.info-tip:focus-visible { outline:2px solid var(--desk-accent,#2563eb); outline-offset:1px; }
</style>
