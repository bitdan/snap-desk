<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, shallowRef } from 'vue'
import { Copy, Save, X, Maximize, Scan, Move, Pin, Pipette, ArrowDownToLine } from 'lucide-vue-next'
import { api, onFrame, onScroll, type CaptureFrame, type ColorFormat, type ScrollView } from '@/lib/api'
import { clamp, move, normalize, resize, toPixels, nudgeSelection, type Handle, type Point, type Rect } from '@/lib/selection'
import { useAnnotations, type AnnotationTool } from '@/lib/use-annotations'
import AnnotationSurface from './AnnotationSurface.vue'
import AnnotationToolbar from './AnnotationToolbar.vue'
import CaptureMagnifier from './CaptureMagnifier.vue'
import { magnifierSample } from '@/lib/magnifier'
import { nudgeColorPoint } from '@/lib/color-picker'
import { newerScroll } from '@/lib/scroll-capture'

const frame = ref<CaptureFrame | null>(null)
const selection = ref<Rect | null>(null)
const busy = ref(false)
const error = ref('')
const scroll = ref<ScrollView | null>(null)
const scrollSetup = ref(false)
const automaticScroll = ref(true)
const saveScrollFrames = ref(true)
const viewport = ref({ width: innerWidth, height: innerHeight })
const annotating = ref(false)
const screenImage = shallowRef<HTMLImageElement | null>(null)
const imageReady = ref(false)
const pointer = ref<Point | null>(null)
const picking = ref(false)
const colorFormat = ref<ColorFormat>('hex')
const showMagnifier = computed(() => frame.value && imageReady.value && screenImage.value && pointer.value && !busy.value && !scrollSetup.value && (picking.value || (!annotating.value && !marked.value)))
const editor = useAnnotations(() => pixels.value ?? { x: 0, y: 0, width: 0, height: 0 }, () => frame.value?.dataUrl ?? '', () => busy.value)
const marked = computed(() => editor.history.present.length > 0)
const handles: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']
let gesture: { start: Point; initial: Rect; mode: 'draw' | 'move' | Handle } | null = null
let off: (() => void) | undefined
let offScroll: (() => void) | undefined
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
  editor.reset(); selection.value = null; gesture = null; annotating.value = false; error.value = ''; pointer.value = null; picking.value = false
}
function receiveScroll(value: ScrollView) {
  const next = newerScroll(scroll.value, value, frame.value?.id)
  if (next && !next.collecting) scrollClosed()
  else scroll.value = next
}
function scrollClosed() { scroll.value = null; frame.value = null; scrollSetup.value = false }
async function startScroll() {
  if (!frame.value || !pixels.value || !valid.value || busy.value || marked.value || editor.pending || editor.interacting || gesture) return
  busy.value = true; error.value = ''
  try { receiveScroll(await api.startScroll(frame.value.id, pixels.value, automaticScroll.value, saveScrollFrames.value)); scrollSetup.value = false }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 切换取色不清空选区或标注；Esc / 右键先退出取色，再次按才取消截图。 */
function togglePicker() {
  if (busy.value || gesture || editor.pending || editor.interacting) return
  picking.value = !picking.value; error.value = ''
}
function contextMenu() { if (picking.value) { if (!busy.value) picking.value = false } else void cancel() }
async function copyColor(format = colorFormat.value) {
  if (!frame.value || !pointer.value || !imageReady.value || busy.value || editor.pending || editor.interacting) return
  const pixel = magnifierSample(pointer.value, viewport.value, frame.value).pixel
  busy.value = true; error.value = ''
  try { await api.copyColor(frame.value.id, pixel, format); frame.value = null }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function pickDown(e: PointerEvent) {
  if (!picking.value || (e.target as Element).closest('.capture-toolbar, .capture-modebar, .overlay-error')) return
  e.stopPropagation()
  if (e.button === 0) { e.preventDefault(); pointer.value = point(e); void copyColor(e.shiftKey ? 'rgb' : colorFormat.value) }
}
function point(e: PointerEvent): Point { return { x: clamp(e.clientX, 0, viewport.value.width), y: clamp(e.clientY, 0, viewport.value.height) } }
function down(e: PointerEvent, mode: 'draw' | 'move' | Handle = 'draw') {
  if (busy.value || picking.value || e.button !== 0 || !frame.value) return
  // 有标注时固定选区，避免调整裁剪范围导致已画内容跳动；“重新框选”显式清空。
  if (marked.value || editor.interacting || annotating.value) return
  editor.reset()
  const p = point(e)
  pointer.value = p
  if (mode === 'draw') selection.value = { ...p, width: 0, height: 0 }
  if (!selection.value) return
  gesture = { start: p, initial: { ...selection.value }, mode }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function drag(e: PointerEvent) {
  // 工具栏不需要像素辅助；捕获阶段监听可覆盖选区控制点的事件。
  pointer.value = (e.target as Element).closest('.capture-toolbar, .capture-modebar, .overlay-error') ? null : point(e)
  if (!gesture) return
  const p = point(e)
  const { start, initial, mode } = gesture
  selection.value = mode === 'draw' ? normalize(start, p) : mode === 'move' ? move(initial, { x: p.x - start.x, y: p.y - start.y }, viewport.value) : resize(initial, mode, p)
}
function up() { gesture = null }
function abortGesture() { gesture = null }
function selectAll() { if (!busy.value && !picking.value && !marked.value && !editor.interacting) { editor.reset(); selection.value = { x: 0, y: 0, ...viewport.value } } }
function receive(value: CaptureFrame) { busy.value = false; reselect(); scroll.value = null; scrollSetup.value = false; imageReady.value = false; frame.value = value }
async function loaded() {
  if (!frame.value) return
  imageReady.value = true
  try { await api.ready(frame.value.id) } catch (e) { error.value = String(e) }
}
async function cancel() {
  if (!frame.value || busy.value || editor.pending) return
  busy.value = true
  try { await api.cancel(frame.value.id); frame.value = null } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function finish(action: 'copy' | 'save' | 'pin') {
  if (!frame.value || picking.value || !valid.value || !pixels.value || busy.value || editor.pending || editor.interacting || gesture) return
  busy.value = true; error.value = ''
  try {
    const result = await api[action](frame.value.id, pixels.value, editor.history.present)
    if (result !== false) frame.value = null
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(e: KeyboardEvent) {
  if (scroll.value) return
  if (scrollSetup.value) {
    if (e.key === 'Escape' && !busy.value) { e.preventDefault(); scrollSetup.value = false }
    else if (e.key === 'Enter' && !e.repeat && !(e.target as HTMLElement).closest('button')) { e.preventDefault(); void startScroll() }
    return
  }
  if ((e.target as HTMLElement).closest('input, textarea, select')) return
  const command = e.ctrlKey || e.metaKey
  if (!command && !e.altKey && !e.repeat && e.key.toLowerCase() === 'i') { e.preventDefault(); togglePicker(); return }
  if (picking.value) {
    // 此模式的快捷键优先于标注编辑，避免取色时误撤销或导出原选区。
    if (e.key === 'Escape') { e.preventDefault(); if (!busy.value) picking.value = false }
    else if (!e.repeat && (e.key === 'Enter' || (command && e.key.toLowerCase() === 'c'))) { e.preventDefault(); void copyColor(e.shiftKey ? 'rgb' : colorFormat.value) }
    else if (e.key.startsWith('Arrow')) {
      e.preventDefault()
      if (pointer.value && frame.value && !busy.value) {
        const step = e.shiftKey ? 10 : 1
        pointer.value = nudgeColorPoint(pointer.value, { x: e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0, y: e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0 }, viewport.value, frame.value)
      }
    }
    return
  }
  if (annotating.value && editor.key(e)) return
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
function resized() { viewport.value = { width: innerWidth, height: innerHeight }; if (!scroll.value) reselect() }
async function imageFailed() {
  error.value = '截图预览加载失败，请重新截图。'
  await cancel()
}
onMounted(async () => {
  window.addEventListener('keydown', key); window.addEventListener('resize', resized)
  try {
    off = await onFrame(receive)
    if (disposed) { off(); return }
    offScroll = await onScroll(receiveScroll)
    if (disposed) { offScroll(); return }
    const current = await api.current()
    if (current && (!frame.value || current.id > frame.value.id)) receive(current)
    const currentScroll = await api.currentScroll()
    if (currentScroll && !disposed) receiveScroll(currentScroll)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.(); offScroll?.(); window.removeEventListener('keydown', key); window.removeEventListener('resize', resized) })
</script>

<template>
  <div v-if="scroll" class="scroll-running" role="status">正在自动拼接，结束后直接保存并复制。</div>
  <div v-else class="capture-overlay" @pointerdown.capture="pickDown" @pointerdown="down($event)" @pointermove.capture="drag" @pointerleave="pointer = null" @pointerup="up" @pointercancel="abortGesture(); pointer = null" @contextmenu.prevent="contextMenu">
    <img v-if="frame" :key="frame.id" ref="screenImage" class="screen-image" :src="frame.dataUrl" alt="当前屏幕截图" draggable="false" @load="loaded" @error="imageFailed" />
    <div v-if="!selection" class="initial-shade" />
    <div v-if="!selection && !picking" class="capture-tip">拖动框选屏幕 <span>Esc / 右键取消 · Ctrl / ⌘ + A 全屏 · I 取色</span></div>
    <div v-if="frame" class="capture-modebar" @pointerdown.stop @contextmenu.stop.prevent>
      <button :disabled="busy || editor.pending || editor.interacting" :aria-pressed="picking" title="取色 I" @click="togglePicker"><Pipette :size="15" />{{ picking ? '返回框选' : '取色 I' }}</button>
      <template v-if="picking">
        <select v-model="colorFormat" :disabled="busy" aria-label="复制色值格式"><option value="hex">HEX</option><option value="rgb">RGB</option></select>
        <span>单击 / Enter 复制并结束 · Shift 临时用 RGB · 方向键微调 · Esc 返回</span>
      </template>
    </div>
    <div v-if="selection" class="selection" :style="{ ...boxStyle, cursor: marked ? 'default' : 'move' }" @pointerdown.stop="selectionDown">
      <span class="size-label">{{ pixels?.width }} × {{ pixels?.height }} px · ({{ pixels?.x }}, {{ pixels?.y }})</span>
      <template v-if="!marked && !annotating"><span v-for="handle in handles" :key="handle" :class="['resize-handle', handle]" @pointerdown.stop="down($event, handle)" /></template>
    </div>
    <AnnotationSurface v-if="frame && pixels" :editor="editor" :width="frame.width" :height="frame.height" :bounds="pixels" :enabled="annotating && !busy && !picking" />
    <div v-if="valid && !picking" class="capture-toolbar annotation-toolbar" :style="toolbarStyle" @pointerdown.stop @contextmenu.stop.prevent>
      <AnnotationToolbar :editor="editor" :disabled="busy" :active="annotating" @tool="chooseTool">
        <template #leading><button :disabled="busy || editor.pending || editor.interacting || marked" :aria-pressed="!annotating" title="调整选区" aria-label="调整选区" @click="annotating = false"><Move :size="16" /></button></template>
        <template #actions>
          <button :disabled="busy || editor.pending || editor.interacting" title="清空标注并重新框选" aria-label="重新框选" @click="reselect"><Scan :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" title="全屏选区" aria-label="全屏选区" @click="selectAll"><Maximize :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" title="长截图：选区不要包含固定标题栏或滚动条；请先清空标注" @click="scrollSetup = true"><ArrowDownToLine :size="16" />长截图</button>
        <button :disabled="busy || editor.pending || editor.interacting" title="贴图 Ctrl/⌘+P" @click="finish('pin')"><Pin :size="16" />贴图</button>
        <button :disabled="busy || editor.pending || editor.interacting" @click="finish('save')"><Save :size="16" />保存</button>
        <button class="copy-button" :disabled="busy || editor.pending || editor.interacting" @click="finish('copy')"><Copy :size="16" />{{ busy ? '处理中' : '复制' }}</button>
        <button :disabled="busy || editor.pending" title="取消截图" @click="cancel"><X :size="16" /></button>
        </template>
      </AnnotationToolbar>
    </div>
    <CaptureMagnifier v-if="showMagnifier && screenImage && pointer && frame" :image="screenImage" :point="pointer" :viewport="viewport" :image-size="frame" :show-color="picking" />
    <div v-if="scrollSetup" class="scroll-setup-shade" @pointerdown.stop @pointermove.stop @contextmenu.stop.prevent>
      <section class="scroll-setup" role="dialog" aria-modal="true" aria-label="开始长截图">
        <h2>滚动长截图</h2>
        <p v-if="error" class="scroll-setup-error" role="alert">{{ error }}</p>
        <p>选区 {{ pixels?.width }} × {{ pixels?.height }} px。只框选可滚动的内容，避开固定标题栏、滚动条和悬浮按钮。</p>
        <label><input v-model="automaticScroll" type="checkbox" :disabled="busy" /> 自动向下滚动（移动鼠标结束）</label>
        <label><input v-model="saveScrollFrames" type="checkbox" :disabled="busy" /> 保存原始帧到本地（取消后仍保留，需自行清理）</label>
        <p v-if="automaticScroll">开始后有 2 秒将鼠标放到选区内的可滚动内容上，并保持不动。小步滚动，画面稳定后继续，连续多次没有位移后停止。目标应用需能接收鼠标所在位置的滚轮。</p>
        <p v-else>开始后在原窗口向下滚动，每次短暂停顿，等预览更新后继续，建议保留至少半屏重叠。</p>
        <p>再次按截图快捷键、从托盘结束或移动鼠标后，自动保存 PNG 并复制。拼接重试失败会自动结束，只输出已确认部分，无需手动处理。</p>
        <div><button :disabled="busy" @click="scrollSetup = false">返回调整选区</button><button :disabled="busy" @click="startScroll">{{ busy ? '准备中…' : '开始滚动采集' }}</button></div>
      </section>
    </div>
    <div v-if="error && !scrollSetup" class="overlay-error" role="alert" @pointerdown.stop>{{ error }}</div>
  </div>
</template>

<style scoped>
.capture-modebar { position: absolute; top: 8px; left: 8px; z-index: 4; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; max-width: calc(100vw - 16px); border-radius: 6px; padding: 6px 8px; background: #242330ed; color: white; font-size: 12px; cursor: default; }
.capture-modebar button { display: flex; align-items: center; gap: 5px; padding: 3px; }
.capture-modebar button[aria-pressed=true] { color: #c4b5fd; }
.capture-modebar button:disabled { opacity: .5; }
.capture-modebar select { background: #3f3f46; color: white; border-radius: 3px; padding: 3px; }
.capture-modebar span { color: #d4d4d8; }
.scroll-setup-shade { position: absolute; inset: 0; z-index: 5; display: grid; place-items: center; background: #0005; cursor: default; }
.scroll-setup { width: min(480px, calc(100vw - 32px)); padding: 20px; border-radius: 8px; background: white; color: #27272a; font-size: 13px; box-shadow: 0 8px 30px #0005; }
.scroll-setup h2 { font-size: 16px; font-weight: 600; margin-bottom: 12px; }.scroll-setup p { margin-bottom: 12px; line-height: 1.6; }
.scroll-setup-error { color: #991b1b; background: #fee2e2; padding: 8px; border-radius: 4px; }
.scroll-setup div { display: flex; justify-content: flex-end; gap: 10px; }.scroll-setup button { padding: 8px 12px; border: 1px solid #d4d4d8; border-radius: 5px; }.scroll-setup button:last-child { background: #2563eb; color: white; }
</style>
