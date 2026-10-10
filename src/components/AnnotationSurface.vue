<script setup lang="ts">
import { computed, useId } from 'vue'
import { shapePath, shapeBounds, isRaster } from '@/lib/annotations'
import type { AnnotationEditor } from '@/lib/use-annotations'
import type { Point, Rect, Handle } from '@/lib/selection'

const props = defineProps<{ editor: AnnotationEditor; width: number; height: number; bounds: Rect; enabled: boolean }>()
const clipId = `annotation-${useId().replaceAll(':', '')}`
const shapes = computed(() => {
  const e = props.editor
  if (!e.draft) return e.history.present
  return e.selected >= 0 ? e.history.present.map((s, i) => i === e.selected ? e.draft! : s) : [...e.history.present, e.draft]
})
const chosen = computed(() => props.editor.selected >= 0 ? shapes.value[props.editor.selected] : null)
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
  const p = point(event), b = props.bounds
  if (p.x < b.x || p.y < b.y || p.x > b.x + b.width || p.y > b.y + b.height) return
  const handle = (event.target as Element).getAttribute('data-handle') as Handle | 'start' | 'end' | null
  const svg = event.currentTarget as SVGSVGElement
  props.editor.begin(p, 6 * props.width / svg.getBoundingClientRect().width, handle ?? undefined)
  if (props.editor.interacting) svg.setPointerCapture(event.pointerId)
}
</script>

<template>
  <svg class="annotation-surface" :class="{ enabled }" :viewBox="`0 0 ${width} ${height}`" preserveAspectRatio="none"
    @pointerdown.stop="down" @pointermove.stop="editor.drag(point($event))" @pointerup.stop="editor.finish()" @pointercancel.stop="!editor.pending && editor.abort()" @contextmenu.stop.prevent="!editor.pending && editor.abort()">
    <defs><clipPath :id="clipId"><rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" /></clipPath></defs>
    <g :clip-path="`url(#${clipId})`">
      <template v-for="(s, index) in shapes" :key="index">
        <image v-if="isRaster(s) && s.raster" :href="`data:image/png;base64,${s.raster}`" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" preserveAspectRatio="none" :opacity="s.opacity ?? 1" />
        <rect v-else-if="isRaster(s)" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" fill="#94a3b844" stroke="#8b5cf6" vector-effect="non-scaling-stroke" />
        <path v-else :d="shapePath(s)" :stroke="s.color" :stroke-width="s.width" :opacity="s.opacity ?? 1" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      </template>
    </g>
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
</style>
