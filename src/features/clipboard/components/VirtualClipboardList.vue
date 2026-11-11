<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch, type ComponentPublicInstance } from 'vue'
import type { ClipboardItem } from '@/features/clipboard/clipboard'
import { rowAt, rowOffsets, visibleRows } from '@/shared/collections/virtual-list'

const props = defineProps<{ items: ClipboardItem[]; hasMore?: boolean; loading?: boolean }>()
const emit = defineEmits<{ nearEnd: [] }>()
const viewport = ref<HTMLElement | null>(null)
const top = ref(0), height = ref(500)
const width = ref(0)
const measured = shallowRef(new Map<number, number>())
// 单列卡片按行虚拟化，只挂载视口附近的条目；未测量的行按类型估算高度（含 6px 行距）。
const offsets = computed(() => rowOffsets(props.items.map(item => measured.value.get(item.id)
  ?? (item.kind === 'image' ? 170 : item.kind === 'file' ? 140 : 76))))
const range = computed(() => visibleRows(offsets.value, top.value, height.value))
const rows = computed(() => props.items.slice(range.value.start, range.value.end))
/** 按虚拟列表的完整高度判断剩余距离；追加或窗口变大后也补足视口。 */
watch([top, height, () => offsets.value.at(-1) ?? 0, () => props.hasMore, () => props.loading], () => {
  if (props.hasMore && !props.loading && (offsets.value.at(-1) ?? 0) - top.value - height.value <= 200) emit('nearEnd')
}, { flush: 'post', immediate: true })
const elements = new Map<number, HTMLElement>()
let observer: ResizeObserver | undefined, disposed = false
let measurementFrame: number | undefined
const pendingMeasurements = new Map<Element, ResizeObserverEntry>()
/** 行高写回会改变观察目标尺寸；延后到下一帧合并，避免同一轮 ResizeObserver 反复触发布局。 */
function scheduleMeasure(entries: ResizeObserverEntry[]) {
  if (disposed) return
  entries.forEach(entry => pendingMeasurements.set(entry.target, entry))
  if (measurementFrame !== undefined) return
  measurementFrame = requestAnimationFrame(() => {
    measurementFrame = undefined
    if (disposed) return
    const latest = [...pendingMeasurements.values()]
    pendingMeasurements.clear()
    measure(latest)
  })
}
function scroll() { top.value = viewport.value?.scrollTop ?? 0 }
/** 动态插槽/缩略图高度变化后保留视口首行位置，避免上方测量值改变引起跳动。 */
function measure(entries: ResizeObserverEntry[]) {
  const anchor = rowAt(offsets.value, top.value), inset = top.value - offsets.value[anchor]
  const next = new Map(measured.value)
  let changed = false
  // 先处理宽度变化，再应用本批行高，避免观察器回调顺序导致刚测量的数据被清空。
  const container = entries.find(entry => entry.target === viewport.value)
  if (container) {
    height.value = container.contentRect.height
    if (width.value !== container.contentRect.width) { width.value = container.contentRect.width; next.clear(); changed = true }
  }
  for (const entry of entries) {
    if (entry.target !== viewport.value) {
      const id = Number((entry.target as HTMLElement).dataset.historyId)
      const size = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height
      if (size > 0 && next.get(id) !== size) { next.set(id, size); changed = true }
    }
  }
  if (!changed) return
  measured.value = next
  const adjusted = offsets.value[Math.min(anchor, Math.max(0, offsets.value.length - 2))] + inset
  if (viewport.value && Math.abs(adjusted - top.value) > 0.5) {
    // 先更新占位高度，再写 scrollTop，避免浏览器按旧滚动范围截断新的位置。
    top.value = adjusted
    void nextTick(() => {
      if (!disposed && viewport.value && top.value === adjusted) { viewport.value.scrollTop = adjusted; scroll() }
    })
  }
}
function observeRow(id: number, value: Element | ComponentPublicInstance | null) {
  const previous = elements.get(id)
  if (previous === value) return
  if (previous) observer?.unobserve(previous)
  if (value instanceof HTMLElement) { elements.set(id, value); observer?.observe(value) }
  else elements.delete(id)
}
/** 键盘按完整数据索引导航，先滚动挂载目标，再聚焦，支持跨虚拟窗口的 Home/End/方向键。 */
async function focusIndex(index: number) {
  const root = viewport.value
  if (!root || !props.items[index]) return
  const start = offsets.value[index], end = offsets.value[index + 1]
  if (start < root.scrollTop) root.scrollTop = start
  else if (end > root.scrollTop + root.clientHeight) root.scrollTop = end - root.clientHeight
  scroll(); await nextTick()
  if (disposed) return
  const row = elements.get(props.items[index].id)?.querySelector<HTMLElement>('.history-item')
  row?.focus({ preventScroll: true }); row?.scrollIntoView({ block: 'nearest' }); scroll()
}
function resetScroll() { if (viewport.value) viewport.value.scrollTop = 0; top.value = 0 }
watch(() => props.items, items => {
  const ids = new Set(items.map(item => item.id))
  measured.value = new Map([...measured.value].filter(([id]) => ids.has(id)))
})
onMounted(() => {
  observer = new ResizeObserver(scheduleMeasure)
  if (viewport.value) observer.observe(viewport.value)
  elements.forEach(element => observer!.observe(element))
})
onUnmounted(() => {
  disposed = true; observer?.disconnect(); elements.clear(); pendingMeasurements.clear()
  if (measurementFrame !== undefined) cancelAnimationFrame(measurementFrame)
})
defineExpose({ focusIndex, resetScroll })
</script>

<template>
  <div ref="viewport" class="history-list" style="overflow-anchor:none" @scroll.passive="scroll">
    <div aria-hidden="true" :style="{ height: `${range.before}px` }" />
    <div v-for="(item, rowIndex) in rows" :key="item.id" :ref="element => observeRow(item.id, element)" :data-history-id="item.id" :data-history-index="range.start + rowIndex" class="history-virtual-row"><slot :item="item" /></div>
    <div aria-hidden="true" :style="{ height: `${range.after}px` }" />
    <slot name="footer" />
  </div>
</template>
