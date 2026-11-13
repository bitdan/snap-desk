<script setup lang="ts">
import type { Component } from 'vue'
/** 工具页顶部的主操作卡片：图标、名称、可选状态行、快捷键与操作按钮。 */
defineProps<{ icon: Component; title: string; meta?: string; shortcut?: string }>()
</script>

<template>
  <section class="tool-hero" :aria-label="title">
    <span class="tool-hero-icon" aria-hidden="true"><component :is="icon" :size="22" :stroke-width="1.8" /></span>
    <span class="tool-hero-text">
      <span class="tool-hero-title">{{ title }}</span>
      <span v-if="meta" class="tool-hero-meta"><slot name="meta">{{ meta }}</slot></span>
    </span>
    <span class="tool-hero-actions">
      <kbd v-if="shortcut">{{ shortcut }}</kbd>
      <slot />
    </span>
  </section>
</template>
