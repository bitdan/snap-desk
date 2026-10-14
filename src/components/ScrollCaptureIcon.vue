<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 16 })
</script>

<template>
  <!-- 断开的选区边框与双向箭头表示在固定区域内滚动采集，线条随按钮颜色变化。 -->
  <svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
    <path d="M8 3H5a1 1 0 0 0-1 1v4m12-5h3a1 1 0 0 1 1 1v4M4 16v4a1 1 0 0 0 1 1h3m12-5v4a1 1 0 0 1-1 1h-3" />
    <path d="M4 11v2m16-2v2M12 6v12m-3-9 3-3 3 3m-6 6 3 3 3-3" />
  </svg>
</template>
