<script setup lang="ts">
withDefaults(defineProps<{ size?: number | string }>(), { size: 16 })
</script>

<template>
  <!-- T 字与底部向右箭头组合，在线条工具中直接表达“箭头文字”。 -->
  <svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
    <path d="M6 7V4h12v3M12 4v12m-3 0h6M4 19h16m-4-3 4 3-4 3" />
  </svg>
</template>
