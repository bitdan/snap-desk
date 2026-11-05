<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { X, Undo2, ChevronLeft, ChevronRight } from 'lucide-vue-next'
import { api, onShelfChanged, onShelfRevealed, onShelfCaptureState } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import type { ShelfSnapshot } from '../types'
import ShelfCard from './ShelfCard.vue'
import { shelfLayout } from '../shelf-layout'

const snapshot = ref<ShelfSnapshot | null>(null), error = ref(''), message = ref(''), busy = ref(false)
const surface = ref<HTMLElement | null>(null)
const viewportWidth = ref(window.innerWidth || 900), page = ref(0)
const capacity = computed(() => shelfLayout(viewportWidth.value, 0).capacity)
const pageCount = computed(() => Math.max(1, Math.ceil((snapshot.value?.items.length ?? 0) / capacity.value)))
const visibleItems = computed(() => snapshot.value?.items.slice(page.value * capacity.value, (page.value + 1) * capacity.value) ?? [])
const layout = computed(() => shelfLayout(viewportWidth.value, visibleItems.value.length))
watch(pageCount, count => { page.value = Math.min(page.value, count - 1) })
function resize() { viewportWidth.value = window.innerWidth || 900 }
function cardStyle(index: number) {
  const position = layout.value.positions[index]
  return { left: `${position.left}px`, top: `${position.top}px` }
}
function turn(direction: number) {
  if (!busy.value) page.value = Math.max(0, Math.min(pageCount.value - 1, page.value + direction))
}
let wheelAt = -Infinity
function wheel(event: WheelEvent) {
  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
  if (Math.abs(delta) < 12 || Date.now() - wheelAt < 220) return
  wheelAt = Date.now(); turn(Math.sign(delta))
}
const removed = ref<{ id: number; kept: boolean } | null>(null)
let off: (() => void) | undefined, offRevealed: (() => void) | undefined, offCapture: (() => void) | undefined, timer: ReturnType<typeof setTimeout> | undefined, disposed = false
let loading = false, reload = false, inside = false, suspended = false, hideNotBefore = 0
async function refresh() {
  if (loading) { reload = true; return }
  loading = true
  try {
    do {
      reload = false
      const next = await api.shelfSnapshot()
      if (!disposed) { snapshot.value = next; if (!next.settings.enabled) await api.shelfVisibility(false); else scheduleHide() }
    } while (reload && !disposed)
  } catch (e) { error.value = String(e) }
  finally { loading = false }
}
function cancelHide() { if (timer) clearTimeout(timer); timer = undefined }
function enter() { inside = true; hideNotBefore = 0; cancelHide() }
function leave() { inside = false; scheduleHide() }
function scheduleHide() { cancelHide(); if (!suspended && !inside && !busy.value) timer = setTimeout(() => { void hide() }, Math.max(700, hideNotBefore - Date.now())) }
function reveal(graceMs = 0) {
  hideNotBefore = Date.now() + graceMs
  page.value = 0; message.value = ''
  inside = surface.value?.matches?.(':hover') ?? false
  // 只在展开时做一次位移，不运行常驻动画；保留系统“减少动态效果”偏好。
  if (!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
    surface.value?.animate?.([{ opacity: 0, transform: 'translateY(-18px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 320, easing: 'cubic-bezier(.2,.7,.2,1)' })
  }
  scheduleHide()
}
function captureState(value: boolean) {
  suspended = value
  if (value) cancelHide()
  // 截图后给出短暂查看时间；历史异步刷新不能把恢复窗口立刻收回。
  else reveal(1800)
}
async function hide() {
  cancelHide()
  try { await api.shelfVisibility(false) } catch (e) { error.value = String(e) }
}
async function run(action: () => Promise<unknown>, success = '') {
  if (busy.value) return
  cancelHide(); busy.value = true; error.value = ''; message.value = ''
  try { await action(); message.value = success; await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false; scheduleHide() }
}
async function remove(id: number) {
  const kept = snapshot.value?.keptIds.includes(id) ?? false
  await api.shelfItemAction(id, 'remove'); removed.value = { id, kept }
}
async function pin(id: number) { await api.clipboardPin(id); await remove(id) }
async function keep(id: number) { await api.shelfItemAction(id, snapshot.value?.keptIds.includes(id) ? 'unkeep' : 'keep') }
async function undo() {
  const previous = removed.value
  if (previous !== null) {
    await api.shelfItemAction(previous.id, previous.kept ? 'keep' : 'restore'); removed.value = null
  }
}
function key(event: KeyboardEvent) {
  if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); turn(event.key === 'ArrowLeft' ? -1 : 1) }
  if (event.key === 'Escape') { event.preventDefault(); void hide() }
  if (event.ctrlKey && event.key.toLowerCase() === 'z') { event.preventDefault(); void run(undo) }
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  window.addEventListener('resize', resize)
  try { const unlisten = await onShelfChanged(() => { void refresh() }); if (disposed) unlisten(); else off = unlisten; await refresh() }
  catch (e) { error.value = String(e) }
  try { const unlisten = await onShelfRevealed(reveal); if (disposed) unlisten(); else offRevealed = unlisten }
  catch (e) { error.value = String(e) }
  try { const unlisten = await onShelfCaptureState(captureState); if (disposed) unlisten(); else offCapture = unlisten }
  catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.(); offRevealed?.(); offCapture?.(); cancelHide(); window.removeEventListener('keydown', key); window.removeEventListener('resize', resize) })
</script>
<template>
  <main ref="surface" class="shelf-window" :aria-label="t('暂存架')" @pointerenter="enter" @pointerleave="leave" @focusin="enter" @focusout="leave" @wheel.prevent="wheel">
    <svg class="shelf-line" :viewBox="`0 0 ${viewportWidth} 64`" preserveAspectRatio="none" aria-hidden="true">
      <path class="rope-shadow" :d="layout.rope" /><path class="rope-body" :d="layout.rope" /><path class="rope-light" :d="layout.rope" />
    </svg>
    <div class="shelf-controls">
      <button v-if="removed !== null" :disabled="busy" :title="t('撤销移除')" :aria-label="t('撤销移除')" @click="run(undo)"><Undo2 :size="13" /></button>
      <button :disabled="busy" :title="t('收起')" :aria-label="t('收起')" @click="hide"><X :size="13" /></button>
    </div>
    <div v-if="snapshot?.items.length" class="shelf-cards">
      <ShelfCard v-for="(item, index) in visibleItems" :key="item.id" :item="item" :style="cardStyle(index)" :kept="snapshot.keptIds.includes(item.id)" :busy="busy"
        @copy="run(() => api.clipboardRestore(item.id), '已复制')" @drag="run(() => api.clipboardDrag(item.id))"
        @keep="run(() => keep(item.id))"
        @remove="run(() => remove(item.id))" @pin="run(() => pin(item.id))" @error="error = $event" />
    </div>
    <p v-else class="shelf-empty">{{ snapshot ? t('暂无匹配内容') : t('正在加载…') }}</p>
    <nav v-if="pageCount > 1" class="shelf-pages" :aria-label="t('暂存架翻页')">
      <button :disabled="busy || page === 0" :aria-label="t('上一组')" :title="t('上一组')" @click="turn(-1)"><ChevronLeft :size="14" /></button>
      <span aria-live="polite">{{ page + 1 }} / {{ pageCount }}</span>
      <button :disabled="busy || page === pageCount - 1" :aria-label="t('下一组')" :title="t('下一组')" @click="turn(1)"><ChevronRight :size="14" /></button>
    </nav>
    <p v-if="message && !error" class="shelf-feedback" role="status">{{ t(message) }}</p>
    <p v-if="error" class="shelf-error" role="alert">{{ te(error) }}</p>
  </main>
</template>
<style scoped>
.shelf-window { position: relative; width: 100%; height: 100vh; overflow: hidden; font-size: 12px; color: #334155; animation: reveal .3s ease-out; }
.shelf-line { position: absolute; inset: 0 0 auto; width: 100%; height: 64px; overflow: visible; pointer-events: none; fill: none; }
.shelf-line path { vector-effect: non-scaling-stroke; }
.rope-shadow { stroke: #0002; stroke-width: 2; transform: translateY(1px); filter: blur(1px); }
.rope-body { stroke: #888b8e; stroke-width: 1; }
.rope-light { stroke: #ffffffe6; stroke-width: .6; transform: translateY(-.7px); }
.shelf-controls { position: absolute; top: 5px; right: 8px; z-index: 4; display: flex; gap: 5px; opacity: 0; transition: opacity .15s; }
.shelf-controls button, .shelf-pages button { display: grid; place-items: center; flex: none; width: 26px; height: 26px; border-radius: 50%; }
.shelf-controls button { background: #f8fafce8; border: 1px solid #ffffffc7; box-shadow: 0 2px 5px #0002; }
.shelf-window:hover .shelf-controls, .shelf-window:focus-within .shelf-controls { opacity: 1; }
.shelf-controls button:hover, .shelf-pages button:hover:not(:disabled) { background: #e2e8f0; }
button:focus-visible { outline: 2px solid #2563eb; outline-offset: 2px; }
.shelf-cards { position: absolute; inset: 0; pointer-events: none; }
.shelf-cards > * { pointer-events: auto; }
.shelf-empty { position: absolute; top: 64px; left: 50%; transform: translateX(-50%); white-space: nowrap; padding: 9px 14px; border: 1px solid #ffffffb8; border-radius: 18px; background: #fafafaf2; box-shadow: 0 3px 12px #0001; }
.shelf-pages { position: absolute; bottom: 9px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 3px; padding: 2px 4px; border: 1px solid #ffffffb8; border-radius: 18px; background: #fafafae8; box-shadow: 0 2px 8px #0001; opacity: .65; }
.shelf-window:hover .shelf-pages, .shelf-window:focus-within .shelf-pages { opacity: 1; }
.shelf-pages span { min-width: 34px; text-align: center; font-variant-numeric: tabular-nums; font-size: 11px; }
.shelf-pages button:disabled { opacity: .3; }
.shelf-feedback, .shelf-error { position: absolute; left: 50%; bottom: 48px; transform: translateX(-50%); width: max-content; max-width: calc(100% - 32px); margin: 0; padding: 5px 11px; border: 1px solid #ffffffc7; border-radius: 12px; background: #fafafaf5; box-shadow: 0 2px 8px #0001; }
.shelf-feedback { top: 3px; bottom: auto; color: #24834b; pointer-events: none; z-index: 4; }
.shelf-error { color: #b42318; max-height: 48px; overflow: auto; z-index: 5; }
@keyframes reveal { from { opacity: 0; transform: translateY(-14px); } to { opacity: 1; transform: translateY(0); } }
@media (prefers-reduced-motion: reduce) { .shelf-window { animation: none; } .shelf-controls { transition: none; } }
</style>
