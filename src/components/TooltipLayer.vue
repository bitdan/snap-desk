<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
// 提示挂在 body，避免被工具栏缩放或 overflow 裁剪。
const text = ref('')
const tooltip = ref<HTMLElement | null>(null)
const position = ref({ left: '0px', top: '0px' })
let target: HTMLElement | null = null
let original = ''
let timer: ReturnType<typeof setTimeout> | undefined
function hide() {
  clearTimeout(timer); text.value = ''
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
    const top = rect.top >= bounds.height + 8 ? rect.top - bounds.height - 8 : rect.bottom + 8
    position.value = { left: `${Math.max(8, Math.min(innerWidth - bounds.width - 8, rect.left + (rect.width - bounds.width) / 2))}px`, top: `${Math.max(4, Math.min(innerHeight - bounds.height - 4, top))}px` }
  }, 180)
}
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
<template><Teleport to="body"><div v-if="text" ref="tooltip" class="chinese-tooltip" role="tooltip" :style="position">{{ text }}</div></Teleport></template>
<style>
.chinese-tooltip { position: fixed; z-index: 10000; width: max-content; max-width: min(280px, calc(100vw - 16px)); pointer-events: none; text-align: center; color: #fff; font: 12px/1.4 'Microsoft YaHei', sans-serif; background: #27272af5; padding: 6px 10px; border-radius: 5px; box-shadow: 0 3px 12px #0002; }
</style>
