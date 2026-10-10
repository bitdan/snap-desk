<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Copy, Save, X, Maximize, Square, Circle, Minus, ArrowUpRight, Undo2, Redo2, MousePointer2, Scan, Pin } from 'lucide-vue-next'
import { api, onFrame, type CaptureFrame } from '@/lib/api'
import { clamp, move, normalize, resize, toPixels, nudgeSelection, type Handle, type Point, type Rect } from '@/lib/selection'
import { append, emptyHistory, undo, redo, shapePath, imagePoint as toImagePoint, type Annotation } from '@/lib/annotations'

const frame = ref<CaptureFrame | null>(null)
const selection = ref<Rect | null>(null)
const busy = ref(false)
const error = ref('')
const viewport = ref({ width: innerWidth, height: innerHeight })
const tool = ref<'select' | Annotation['kind']>('select')
const history = ref(emptyHistory())
const drawing = ref<Annotation | null>(null)
const color = ref('#ef4444')
const lineWidth = ref(3)
const colors = ['#ef4444', '#facc15', '#22c55e', '#3b82f6', '#ffffff', '#18181b']
const marked = computed(() => history.value.present.length > 0)
const shapes = computed(() => drawing.value ? [...history.value.present, drawing.value] : history.value.present)
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
  return { left: `${clamp(r.x, 8, Math.max(8, viewport.value.width - 548))}px`, top: `${clamp(r.y + r.height + 12, 8, Math.max(8, viewport.value.height - 122))}px` }
})
// 所有图形保存在原图坐标系内；选区只负责裁剪和限制绘制区域。
function imagePoint(e: PointerEvent): Point {
  return toImagePoint({ x: e.clientX, y: e.clientY }, viewport.value, frame.value!, pixels.value!)
}
function selectionDown(e: PointerEvent) {
  if (busy.value || e.button !== 0 || !valid.value) return
  if (tool.value === 'select') { down(e, 'move'); return }
  if (history.value.present.length >= 100) { error.value = '单张截图最多支持 100 个标注。'; return }
  const start = imagePoint(e)
  drawing.value = { kind: tool.value, start, end: start, color: color.value, width: clamp(lineWidth.value * frame.value!.width / viewport.value.width, 0.5, 64) }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function undoShape() { if (!busy.value && !drawing.value) history.value = undo(history.value) }
function redoShape() { if (!busy.value && !drawing.value) history.value = redo(history.value) }
function reselect() {
  if (busy.value) return
  history.value = emptyHistory(); drawing.value = null; selection.value = null; gesture = null; tool.value = 'select'; error.value = ''
}
function point(e: PointerEvent): Point { return { x: clamp(e.clientX, 0, viewport.value.width), y: clamp(e.clientY, 0, viewport.value.height) } }
function down(e: PointerEvent, mode: 'draw' | 'move' | Handle = 'draw') {
  if (busy.value || e.button !== 0 || !frame.value) return
  // 有标注时固定选区，避免调整裁剪范围导致已画内容跳动；“重新框选”显式清空。
  if (marked.value || drawing.value || tool.value !== 'select') return
  history.value = emptyHistory()
  const p = point(e)
  if (mode === 'draw') selection.value = { ...p, width: 0, height: 0 }
  if (!selection.value) return
  gesture = { start: p, initial: { ...selection.value }, mode }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function drag(e: PointerEvent) {
  if (drawing.value) { drawing.value = { ...drawing.value, end: imagePoint(e) }; return }
  if (!gesture) return
  const p = point(e)
  const { start, initial, mode } = gesture
  selection.value = mode === 'draw' ? normalize(start, p) : mode === 'move' ? move(initial, { x: p.x - start.x, y: p.y - start.y }, viewport.value) : resize(initial, mode, p)
}
function up() {
  gesture = null
  if (!drawing.value) return
  const s = drawing.value
  const dx = Math.abs(s.end.x - s.start.x), dy = Math.abs(s.end.y - s.start.y)
  if (s.kind === 'rectangle' || s.kind === 'ellipse' ? dx >= 1 && dy >= 1 : Math.hypot(dx, dy) >= 2) history.value = append(history.value, s)
  drawing.value = null
}
function abortGesture() { drawing.value = null; gesture = null }
function selectAll() { if (!busy.value && !marked.value && !drawing.value) { history.value = emptyHistory(); selection.value = { x: 0, y: 0, ...viewport.value } } }
function receive(value: CaptureFrame) { busy.value = false; reselect(); frame.value = value }
async function loaded() {
  if (!frame.value) return
  try { await api.ready(frame.value.id) } catch (e) { error.value = String(e) }
}
async function cancel() {
  if (!frame.value || busy.value) return
  busy.value = true
  try { await api.cancel(frame.value.id); frame.value = null } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function finish(action: 'copy' | 'save' | 'pin') {
  if (!frame.value || !valid.value || !pixels.value || busy.value || drawing.value || gesture) return
  busy.value = true; error.value = ''
  try {
    const result = await api[action](frame.value.id, pixels.value, history.value.present)
    if (result !== false) frame.value = null
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(e: KeyboardEvent) {
  const command = e.ctrlKey || e.metaKey
  // 工具栏控件编辑时保留原生按键（例如下拉框的方向键和 Enter）。
  const target = e.target as HTMLElement
  if (['INPUT', 'SELECT', 'BUTTON'].includes(target.tagName) && e.key !== 'Escape' && !command) return
  // 方向键允许长按连续微调；绘制、导出或已有标注时不改变裁剪范围。
  const directions: Record<string, Point> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } }
  const direction = directions[e.key]
  if (direction && !command) {
    e.preventDefault()
    if (busy.value || gesture || drawing.value || marked.value || tool.value !== 'select' || !valid.value || !selection.value || !frame.value) return
    const step = e.shiftKey ? 10 : 1
    selection.value = nudgeSelection(selection.value, { x: direction.x * step, y: direction.y * step }, viewport.value, frame.value, e.altKey)
    history.value = emptyHistory()
    return
  }
  if (e.repeat) return
  if (e.key === 'Escape') { e.preventDefault(); void cancel() }
  else if (e.key === 'Enter' || (command && e.key.toLowerCase() === 'c')) { e.preventDefault(); void finish('copy') }
  else if (command && e.key.toLowerCase() === 's') { e.preventDefault(); void finish('save') }
  else if (command && e.key.toLowerCase() === 'p') { e.preventDefault(); void finish('pin') }
  else if (command && e.key.toLowerCase() === 'a') { e.preventDefault(); selectAll() }
  else if (command && e.key.toLowerCase() === 'z') { e.preventDefault(); e.shiftKey ? redoShape() : undoShape() }
  else if (command && e.key.toLowerCase() === 'y') { e.preventDefault(); redoShape() }
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
    <div v-if="selection" class="selection" :style="{ ...boxStyle, cursor: tool === 'select' ? marked ? 'default' : 'move' : 'crosshair' }" @pointerdown.stop="selectionDown">
      <span class="size-label">{{ pixels?.width }} × {{ pixels?.height }} px · ({{ pixels?.x }}, {{ pixels?.y }})</span>
      <template v-if="!marked && tool === 'select'"><span v-for="handle in handles" :key="handle" :class="['resize-handle', handle]" @pointerdown.stop="down($event, handle)" /></template>
    </div>
    <svg v-if="frame && pixels" class="annotation-preview" :viewBox="`0 0 ${frame.width} ${frame.height}`" preserveAspectRatio="none" aria-hidden="true">
      <defs><clipPath id="selection-clip"><rect :x="pixels.x" :y="pixels.y" :width="pixels.width" :height="pixels.height" /></clipPath></defs>
      <g clip-path="url(#selection-clip)"><path v-for="(s, index) in shapes" :key="index" :d="shapePath(s)" :stroke="s.color" :stroke-width="s.width" fill="none" stroke-linecap="round" stroke-linejoin="round" /></g>
    </svg>
    <div v-if="valid && !drawing" class="capture-toolbar annotation-toolbar" :style="toolbarStyle" @pointerdown.stop>
      <div class="tool-row">
        <button title="调整选区" aria-label="调整选区" :class="{ 'tool-active': tool === 'select' }" :disabled="busy || marked" @click="tool = 'select'"><MousePointer2 :size="17" /></button>
        <button title="矩形" aria-label="矩形标注" :aria-pressed="tool === 'rectangle'" :class="{ 'tool-active': tool === 'rectangle' }" :disabled="busy" @click="tool = 'rectangle'"><Square :size="17" /></button>
        <button title="椭圆" aria-label="椭圆标注" :aria-pressed="tool === 'ellipse'" :class="{ 'tool-active': tool === 'ellipse' }" :disabled="busy" @click="tool = 'ellipse'"><Circle :size="17" /></button>
        <button title="直线" aria-label="直线标注" :aria-pressed="tool === 'line'" :class="{ 'tool-active': tool === 'line' }" :disabled="busy" @click="tool = 'line'"><Minus :size="17" /></button>
        <button title="箭头" aria-label="箭头标注" :aria-pressed="tool === 'arrow'" :class="{ 'tool-active': tool === 'arrow' }" :disabled="busy" @click="tool = 'arrow'"><ArrowUpRight :size="17" /></button>
        <span class="toolbar-divider" />
        <button title="撤销 Ctrl/⌘+Z" aria-label="撤销" :disabled="busy || !marked" @click="undoShape"><Undo2 :size="17" /></button>
        <button title="重做 Ctrl/⌘+Shift+Z" aria-label="重做" :disabled="busy || !history.future.length" @click="redoShape"><Redo2 :size="17" /></button>
        <button title="清空标注并重新框选" :disabled="busy" @click="reselect"><Scan :size="17" />重选</button>
        <button title="选择整个主屏" aria-label="选择整个主屏" :disabled="busy || marked" @click="selectAll"><Maximize :size="17" /></button>
      </div>
      <div class="tool-row">
        <button v-for="c in colors" :key="c" class="color-swatch" :style="{ background: c }" :class="{ chosen: color === c }" :aria-label="`颜色 ${c}`" :aria-pressed="color === c" :disabled="busy" @click="color = c" />
        <select v-model.number="lineWidth" aria-label="标注线宽" :disabled="busy"><option :value="2">细</option><option :value="3">中</option><option :value="6">粗</option></select>
        <button title="贴图 Ctrl/⌘+P" :disabled="busy" @click="finish('pin')"><Pin :size="16" />贴图</button>
        <button :disabled="busy" @click="finish('save')"><Save :size="16" />保存</button><button class="copy-button" :disabled="busy" @click="finish('copy')"><Copy :size="16" />{{ busy ? '处理中' : '复制' }}</button><button title="取消 (Esc)" aria-label="取消截图" :disabled="busy" @click="cancel"><X :size="18" /></button>
      </div>
      <small v-if="tool === 'select' && !marked">方向键移动 1 px · Shift 10 px · Alt 调整宽高</small>
      <small v-else>选工具后在选区内拖动 · {{ history.present.length }}/100 · 重选会清空标注</small>
    </div>
    <div v-if="error" class="overlay-error" role="alert" @pointerdown.stop>{{ error }}</div>
  </div>
</template>
