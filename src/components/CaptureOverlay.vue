<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Copy, Save, X, Maximize, Scan, Move, Pin } from 'lucide-vue-next'
import { api, onFrame, type CaptureFrame } from '@/lib/api'
import { clamp, move, normalize, resize, toPixels, nudgeSelection, type Handle, type Point, type Rect } from '@/lib/selection'
import { useAnnotations, type AnnotationTool } from '@/lib/use-annotations'
import AnnotationSurface from './AnnotationSurface.vue'
import AnnotationToolbar from './AnnotationToolbar.vue'

const frame = ref<CaptureFrame | null>(null)
const selection = ref<Rect | null>(null)
const busy = ref(false)
const error = ref('')
const viewport = ref({ width: innerWidth, height: innerHeight })
const annotating = ref(false)
const editor = useAnnotations(() => pixels.value ?? { x: 0, y: 0, width: 0, height: 0 }, () => frame.value?.dataUrl ?? '', () => busy.value)
const marked = computed(() => editor.history.present.length > 0)
const handles: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']
let gesture: { start: Point; initial: Rect; mode: 'draw' | 'move' | Handle } | null = null
let off: (() => void) | undefined
let disposed = false
const pixels = computed(() => selection.value && frame.value ? toPixels(selection.value, viewport.value, frame.value) : null)
const valid = computed(() => pixels.value && pixels.value.width > 0 && pixels.value.height > 0)
const boxStyle = computed(() => selection.value ? { left: `${selection.value.x}px`, top: `${selection.value.y}px`, width: `${selection.value.width}px`, height: `${selection.value.height}px` } : {})
const toolbarStyle = computed(() => {
  const r = selection.value
  if (!r) return {}
  return { left: `${clamp(r.x, 8, Math.max(8, viewport.value.width - 960))}px`, top: `${clamp(r.y + r.height + 12, 8, Math.max(8, viewport.value.height - 106))}px` }
})
// 所有图形保存在原图坐标系内；选区只负责裁剪和限制绘制区域。
function chooseTool(tool: AnnotationTool) { if (busy.value || editor.pending || editor.interacting) return; editor.setTool(tool); annotating.value = true }
function selectionDown(e: PointerEvent) { if (!annotating.value) down(e, 'move') }
function reselect() {
  if (busy.value) return
  editor.reset(); selection.value = null; gesture = null; annotating.value = false; error.value = ''
}
function point(e: PointerEvent): Point { return { x: clamp(e.clientX, 0, viewport.value.width), y: clamp(e.clientY, 0, viewport.value.height) } }
function down(e: PointerEvent, mode: 'draw' | 'move' | Handle = 'draw') {
  if (busy.value || e.button !== 0 || !frame.value) return
  // 有标注时固定选区，避免调整裁剪范围导致已画内容跳动；“重新框选”显式清空。
  if (marked.value || editor.interacting || annotating.value) return
  editor.reset()
  const p = point(e)
  if (mode === 'draw') selection.value = { ...p, width: 0, height: 0 }
  if (!selection.value) return
  gesture = { start: p, initial: { ...selection.value }, mode }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function drag(e: PointerEvent) {
  if (!gesture) return
  const p = point(e)
  const { start, initial, mode } = gesture
  selection.value = mode === 'draw' ? normalize(start, p) : mode === 'move' ? move(initial, { x: p.x - start.x, y: p.y - start.y }, viewport.value) : resize(initial, mode, p)
}
function up() { gesture = null }
function abortGesture() { gesture = null }
function selectAll() { if (!busy.value && !marked.value && !editor.interacting) { editor.reset(); selection.value = { x: 0, y: 0, ...viewport.value } } }
function receive(value: CaptureFrame) { busy.value = false; reselect(); frame.value = value }
async function loaded() {
  if (!frame.value) return
  try { await api.ready(frame.value.id) } catch (e) { error.value = String(e) }
}
async function cancel() {
  if (!frame.value || busy.value || editor.pending) return
  busy.value = true
  try { await api.cancel(frame.value.id); frame.value = null } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function finish(action: 'copy' | 'save' | 'pin') {
  if (!frame.value || !valid.value || !pixels.value || busy.value || editor.pending || editor.interacting || gesture) return
  busy.value = true; error.value = ''
  try {
    const result = await api[action](frame.value.id, pixels.value, editor.history.present)
    if (result !== false) frame.value = null
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(e: KeyboardEvent) {
  if (annotating.value && editor.key(e)) return
  if ((e.target as HTMLElement).closest('input, textarea, select')) return
  const command = e.ctrlKey || e.metaKey
  // 工具栏控件编辑时保留原生按键（例如下拉框的方向键和 Enter）。
  const target = e.target as HTMLElement
  if (['INPUT', 'SELECT', 'BUTTON'].includes(target.tagName) && e.key !== 'Escape' && !command) return
  // 方向键允许长按连续微调；绘制、导出或已有标注时不改变裁剪范围。
  const directions: Record<string, Point> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } }
  const direction = directions[e.key]
  if (direction && !command) {
    e.preventDefault()
    if (busy.value || gesture || editor.interacting || marked.value || annotating.value || !valid.value || !selection.value || !frame.value) return
    const step = e.shiftKey ? 10 : 1
    selection.value = nudgeSelection(selection.value, { x: direction.x * step, y: direction.y * step }, viewport.value, frame.value, e.altKey)
    editor.reset()
    return
  }
  if (e.repeat) return
  if (e.key === 'Escape') { e.preventDefault(); void cancel() }
  else if (e.key === 'Enter' || (command && e.key.toLowerCase() === 'c')) { e.preventDefault(); void finish('copy') }
  else if (command && e.key.toLowerCase() === 's') { e.preventDefault(); void finish('save') }
  else if (command && e.key.toLowerCase() === 'p') { e.preventDefault(); void finish('pin') }
  else if (command && e.key.toLowerCase() === 'a') { e.preventDefault(); selectAll() }
  else if (command && e.key.toLowerCase() === 'z') { e.preventDefault(); void editor.travel(e.shiftKey ? 'redo' : 'undo') }
  else if (command && e.key.toLowerCase() === 'y') { e.preventDefault(); void editor.travel('redo') }
}
function resized() { viewport.value = { width: innerWidth, height: innerHeight }; reselect() }
async function imageFailed() {
  error.value = '截图预览加载失败，请重新截图。'
  await cancel()
}
onMounted(async () => {
  window.addEventListener('keydown', key); window.addEventListener('resize', resized)
  try {
    off = await onFrame(receive)
    if (disposed) { off(); return }
    const current = await api.current()
    if (current && (!frame.value || current.id > frame.value.id)) receive(current)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.(); window.removeEventListener('keydown', key); window.removeEventListener('resize', resized) })
</script>

<template>
  <div class="capture-overlay" @pointerdown="down($event)" @pointermove="drag" @pointerup="up" @pointercancel="abortGesture" @contextmenu.prevent="cancel">
    <img v-if="frame" :key="frame.id" class="screen-image" :src="frame.dataUrl" alt="当前屏幕截图" draggable="false" @load="loaded" @error="imageFailed" />
    <div v-if="!selection" class="initial-shade" />
    <div v-if="!selection" class="capture-tip">拖动框选屏幕 <span>Esc / 右键取消 · Ctrl / ⌘ + A 全屏</span></div>
    <div v-if="selection" class="selection" :style="{ ...boxStyle, cursor: marked ? 'default' : 'move' }" @pointerdown.stop="selectionDown">
      <span class="size-label">{{ pixels?.width }} × {{ pixels?.height }} px · ({{ pixels?.x }}, {{ pixels?.y }})</span>
      <template v-if="!marked && !annotating"><span v-for="handle in handles" :key="handle" :class="['resize-handle', handle]" @pointerdown.stop="down($event, handle)" /></template>
    </div>
    <AnnotationSurface v-if="frame && pixels" :editor="editor" :width="frame.width" :height="frame.height" :bounds="pixels" :enabled="annotating && !busy" />
    <div v-if="valid" class="capture-toolbar annotation-toolbar" :style="toolbarStyle" @pointerdown.stop @contextmenu.stop.prevent>
      <AnnotationToolbar :editor="editor" :disabled="busy" :active="annotating" @tool="chooseTool">
        <template #leading><button :disabled="busy || editor.pending || editor.interacting || marked" :aria-pressed="!annotating" title="调整选区" aria-label="调整选区" @click="annotating = false"><Move :size="16" /></button></template>
        <template #actions>
          <button :disabled="busy || editor.pending || editor.interacting" title="清空标注并重新框选" aria-label="重新框选" @click="reselect"><Scan :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" title="全屏选区" aria-label="全屏选区" @click="selectAll"><Maximize :size="16" /></button>
        <button :disabled="busy || editor.pending || editor.interacting" title="贴图 Ctrl/⌘+P" @click="finish('pin')"><Pin :size="16" />贴图</button>
        <button :disabled="busy || editor.pending || editor.interacting" @click="finish('save')"><Save :size="16" />保存</button>
        <button class="copy-button" :disabled="busy || editor.pending || editor.interacting" @click="finish('copy')"><Copy :size="16" />{{ busy ? '处理中' : '复制' }}</button>
        <button :disabled="busy || editor.pending" title="取消截图" @click="cancel"><X :size="16" /></button>
        </template>
      </AnnotationToolbar>
    </div>
    <div v-if="error" class="overlay-error" role="alert" @pointerdown.stop>{{ error }}</div>
  </div>
</template>
