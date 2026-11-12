<script setup lang="ts">
import { appearance } from '@/shared/ui/appearance'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { splitTooltip } from '@/shared/ui/tooltip-text'
// 提示挂在 body，避免被工具栏缩放或 overflow 裁剪。
const text = ref('')
// 快捷键单独显示为按键，说明文字与快捷键在所有窗口里保持同一种排版。
const parts = computed(() => splitTooltip(text.value))
// 刚隐藏后短时间内移到相邻按钮直接显示，扫过工具栏时不必每个都等待。
let warmUntil = 0
const tooltip = ref<HTMLElement | null>(null)
const position = ref({ left: '0px', top: '0px' })
let target: HTMLElement | null = null
let original = ''
let timer: ReturnType<typeof setTimeout> | undefined
function hide() {
  clearTimeout(timer)
  if (text.value) warmUntil = Date.now() + 500
  text.value = ''
  // 语言切换时 Vue 会更新 title；不能用旧提示覆盖新属性。
  if (target && !target.hasAttribute('title')) target.setAttribute('title', original)
  target = null
}
function show(event: Event) {
  if (target && event.target instanceof Node && target.contains(event.target)) return
  const element = event.target instanceof Element ? event.target.closest<HTMLElement>('[title]') : null
  if (!element) return
  hide(); original = element.title
  if (!original) return
  target = element; element.removeAttribute('title')
  timer = setTimeout(async () => {
    if (!element.isConnected) return hide()
    const rect = element.getBoundingClientRect()
    text.value = original
    await nextTick()
    if (target !== element || !tooltip.value) return
    const bounds = tooltip.value.getBoundingClientRect()
    // 优先放在下方，避开工具栏上方的图片或选区；下方放不下再翻到上方。
    const top = innerHeight - rect.bottom >= bounds.height + 8 ? rect.bottom + 6 : rect.top - bounds.height - 6
    position.value = { left: `${Math.max(8, Math.min(innerWidth - bounds.width - 8, rect.left + (rect.width - bounds.width) / 2))}px`, top: `${Math.max(4, Math.min(innerHeight - bounds.height - 4, top))}px` }
  }, Date.now() < warmUntil ? 0 : 400)
}
watch(() => appearance.value.language, hide, { flush: 'post' })
function leave(event: Event) {
  const next = (event as MouseEvent).relatedTarget
  if (target && (!(next instanceof Node) || !target.contains(next))) hide()
}
function key(event: KeyboardEvent) { if (event.key === 'Escape') hide() }
onMounted(() => {
  document.addEventListener('pointerover', show); document.addEventListener('focusin', show)
  document.addEventListener('pointerout', leave); document.addEventListener('focusout', leave)
  document.addEventListener('pointerdown', hide, true); document.addEventListener('keydown', key)
  window.addEventListener('blur', hide)
})
onUnmounted(() => {
  hide(); document.removeEventListener('pointerover', show); document.removeEventListener('focusin', show)
  document.removeEventListener('pointerout', leave); document.removeEventListener('focusout', leave)
  document.removeEventListener('pointerdown', hide, true); document.removeEventListener('keydown', key)
  window.removeEventListener('blur', hide)
})
</script>
<template><Teleport to="body"><div v-if="text" ref="tooltip" class="chinese-tooltip" role="tooltip" :style="position"><span>{{ parts.label }}</span><span v-if="parts.keys.length" class="chinese-tooltip-keys"><kbd v-for="(key, index) in parts.keys" :key="index">{{ key }}</kbd></span></div></Teleport></template>
<style>
.chinese-tooltip { position: fixed; z-index: 10000; display: flex; align-items: center; gap: 8px; width: max-content; max-width: min(280px, calc(100vw - 16px)); pointer-events: none; color: #f6f8fa; font: 12px/1.45 'Segoe UI', 'Microsoft YaHei', sans-serif; background: #1f2328f2; padding: 5px 8px; border-radius: 6px; box-shadow: 0 4px 14px #0003; }
.chinese-tooltip-keys { display: inline-flex; flex: none; gap: 3px; }
.chinese-tooltip kbd { min-width: 18px; padding: 0 5px; border: 1px solid #ffffff2e; border-radius: 4px; background: #ffffff17; color: #d0d7de; font: 11px/16px 'Segoe UI', 'Microsoft YaHei', sans-serif; text-align: center; }
[data-theme=dark] .chinese-tooltip { background: #f6f8fa; color: #1f2328; box-shadow: 0 4px 14px #0006; }
[data-theme=dark] .chinese-tooltip kbd { border-color: #1f23282e; background: #1f232812; color: #57606a; }
</style>
