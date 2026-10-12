<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { shapePath, shapeBounds, isRaster } from '@/lib/annotations'
import type { AnnotationEditor } from '@/lib/use-annotations'
import type { Point, Rect, Handle } from '@/lib/selection'

const props = defineProps<{ editor: AnnotationEditor; width: number; height: number; bounds: Rect; enabled: boolean }>()
const clipId = `annotation-${useId().replaceAll(':', '')}`
const textInput = ref<HTMLTextAreaElement | null>(null)
const textBox = computed(() => props.editor.textBox())
const textHeight = ref(36)
// 初始只显示一行；用浏览器实际换行高度扩展，达到可用高度后在框内滚动。
watch(() => [props.editor.textEditing, props.editor.draft?.text, props.editor.draft?.fontSize, textBox.value.width], async () => {
  await nextTick()
  const input = textInput.value
  if (!input) return
  input.style.height = '0px'
  textHeight.value = Math.min(textBox.value.height, Math.max(Math.ceil((props.editor.draft?.fontSize ?? 24) * 1.3 + 4), input.scrollHeight))
  input.style.height = '100%'
}, { flush: 'post' })
watch(() => props.editor.textEditing, async active => {
  if (active) { await nextTick(); textInput.value?.focus() }
})
async function finishText() {
  await props.editor.finishText()
  // 原生保存或文字排版失败时保留原输入，焦点回到框内以便继续修改。
  if (props.editor.textEditing) { await nextTick(); textInput.value?.focus() }
}
function textKey(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return
  if (event.key === 'Escape') { event.preventDefault(); if (!props.editor.pending) props.editor.abort() }
  else if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); void finishText() }
}
const shapes = computed(() => {
  const e = props.editor
  if (!e.draft) return e.history.present
  return e.selected >= 0 ? e.history.present.map((s, i) => i === e.selected ? e.draft! : s) : [...e.history.present, e.draft]
})
const chosen = computed(() => props.editor.selected >= 0 ? shapes.value[props.editor.selected] : null)
const spotlights = computed(() => shapes.value.filter(s => s.kind === 'spotlight'))
const spotlightIndex = computed(() => shapes.value.findIndex(s => s.kind === 'spotlight'))
const spotlightMaskId = `${clipId}-spotlight`
const handles = computed(() => {
  const s = chosen.value
  if (!s) return []
  if (s.kind === 'line' || s.kind === 'arrow') return [{ name: 'start', ...s.start }, { name: 'end', ...s.end }]
  const b = shapeBounds(s)
  return [ ['nw', 0, 0], ['n', .5, 0], ['ne', 1, 0], ['e', 1, .5], ['se', 1, 1], ['s', .5, 1], ['sw', 0, 1], ['w', 0, .5] ].map(([name, x, y]) => ({ name: String(name), x: b.x + Number(x) * b.width, y: b.y + Number(y) * b.height }))
})
function point(event: PointerEvent): Point {
  const box = (event.currentTarget as SVGSVGElement).getBoundingClientRect()
  return { x: (event.clientX - box.left) * props.width / box.width, y: (event.clientY - box.top) * props.height / box.height }
}
function down(event: PointerEvent) {
  if (!props.enabled || event.button !== 0) return
  if (props.editor.textEditing) { void finishText(); return }
  const p = point(event), b = props.bounds
  if (p.x < b.x || p.y < b.y || p.x > b.x + b.width || p.y > b.y + b.height) return
  const handle = (event.target as Element).getAttribute('data-handle') as Handle | 'start' | 'end' | null
  const svg = event.currentTarget as SVGSVGElement
  props.editor.begin(p, 6 * props.width / svg.getBoundingClientRect().width, handle ?? undefined)
  if (props.editor.interacting && !props.editor.textEditing) svg.setPointerCapture(event.pointerId)
}
</script>

<template>
  <svg class="annotation-surface" :class="{ enabled }" :viewBox="`0 0 ${width} ${height}`" preserveAspectRatio="none"
    @pointerdown.stop="down" @pointermove.stop="editor.drag(point($event))" @pointerup.stop="editor.finish()" @pointercancel.stop="!editor.pending && editor.abort()" @contextmenu.stop.prevent="!editor.pending && editor.abort()">
    <defs>
      <clipPath :id="clipId"><rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" /></clipPath>
      <!-- 黑色亮区在同一遮罩内取并集，重叠区域不会像 evenodd 那样重新变暗。 -->
      <mask :id="spotlightMaskId" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" style="mask-type: luminance">
        <rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" fill="white" />
        <rect v-for="(s, index) in spotlights" :key="index" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" fill="black" />
      </mask>
    </defs>
    <g :clip-path="`url(#${clipId})`">
      <template v-for="(s, index) in shapes" :key="index">
        <template v-if="s.kind === 'spotlight'">
          <rect v-if="index === spotlightIndex" :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" :mask="`url(#${spotlightMaskId})`" :fill="s.color" :opacity="s.opacity ?? 1" />
        </template>
        <image v-else-if="isRaster(s) && s.raster" :href="`data:image/png;base64,${s.raster}`" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" preserveAspectRatio="none" :opacity="s.opacity ?? 1" />
        <rect v-else-if="isRaster(s)" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" fill="#94a3b844" stroke="#8b5cf6" vector-effect="non-scaling-stroke" />
        <path v-else :d="shapePath(s)" :stroke="s.color" :stroke-width="s.width" :opacity="s.opacity ?? 1" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      </template>
    </g>
    <foreignObject v-if="enabled && editor.textEditing && editor.draft" :x="textBox.x" :y="textBox.y" :width="textBox.width" :height="textHeight"
      @pointerdown.stop @pointermove.stop @pointerup.stop @pointercancel.stop @contextmenu.stop @keydown.stop="textKey">
      <textarea ref="textInput" v-model="editor.draft.text" class="inline-text" aria-label="标注文字" placeholder="输入文字" maxlength="500" :readonly="editor.pending"
        title="Enter 完成，Shift+Enter 换行，Esc 取消" :style="{ color: editor.draft.color, fontSize: `${editor.draft.fontSize ?? 24}px`, fontFamily: ({ sans: 'sans-serif', serif: 'serif', mono: 'monospace' })[editor.draft.fontFamily ?? 'sans'] }" />
    </foreignObject>
    <g v-if="enabled && chosen && editor.tool === 'edit'" class="annotation-selection">
      <rect :x="shapeBounds(chosen).x" :y="shapeBounds(chosen).y" :width="Math.max(1, shapeBounds(chosen).width)" :height="Math.max(1, shapeBounds(chosen).height)" fill="none" stroke="#8b5cf6" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" />
      <path v-for="h in handles" :key="h.name" :data-handle="h.name" :d="`M ${h.x} ${h.y} h .001`" stroke="white" stroke-width="9" stroke-linecap="square" vector-effect="non-scaling-stroke" class="edit-handle" />
    </g>
  </svg>
</template>

<style scoped>
.annotation-surface { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.annotation-surface.enabled { pointer-events: auto; cursor: crosshair; touch-action: none; }
.annotation-selection { pointer-events: none; }
.edit-handle { pointer-events: stroke; cursor: nwse-resize; filter: drop-shadow(0 0 1px #7c3aed); }
.inline-text { display: block; box-sizing: border-box; width: 100%; height: 100%; margin: 0; padding: 2px; border: 0; outline: 1px dashed #8b5cf6; outline-offset: -1px; border-radius: 0; background: #ffffffdd; line-height: 1.3; resize: none; cursor: text; white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
