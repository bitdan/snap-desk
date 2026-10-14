<script setup lang="ts">
import ScrollCaptureIcon from './ScrollCaptureIcon.vue'
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { Copy, Save, X, Monitor, RotateCcw, Scan, Move, Pin, Pipette, Video } from 'lucide-vue-next'
import { api, onFrame, onScroll, type CaptureFrame, type ColorFormat, type ScrollView } from '@/lib/api'
import { clamp, move, normalize, resize, toPixels, nudgeSelection, type Handle, type Point, type Rect } from '@/lib/selection'
import { useAnnotations, type AnnotationTool } from '@/lib/use-annotations'
import AnnotationSurface from './AnnotationSurface.vue'
import AnnotationToolbar from './AnnotationToolbar.vue'
import CaptureMagnifier from './CaptureMagnifier.vue'
import { magnifierSample } from '@/lib/magnifier'
import { nudgeColorPoint } from '@/lib/color-picker'
import { newerScroll } from '@/lib/scroll-capture'
import { defaultInputEffects, recordingBitrate, recordingConfig, type EncodingPreference, type RecordingQuality, type RecorderCapabilities } from '@/lib/recording'
import RecordingSetupMenu from './RecordingSetupMenu.vue'
import ScrollSetupMenu from './ScrollSetupMenu.vue'
import { readRecentSelection, restoreRecentSelection, writeRecentSelection } from '@/lib/recent-selection'

const frame = ref<CaptureFrame | null>(null)
const selection = ref<Rect | null>(null)
const recentSelection = ref(readRecentSelection())
const recentRect = computed(() => frame.value ? restoreRecentSelection(recentSelection.value, frame.value, viewport.value) : null)
const busy = ref(false)
const error = ref('')
const scroll = ref<ScrollView | null>(null)
const scrollSetup = ref(false)
const scrollTrigger = ref<HTMLButtonElement | null>(null)
const scrollAnchor = ref<DOMRect | null>(null)
function openScrollMenu() {
  scrollAnchor.value = scrollTrigger.value?.getBoundingClientRect() ?? null
  error.value = ''; scrollSetup.value = true
}
watch(scrollSetup, open => { if (!open && frame.value) void nextTick(() => scrollTrigger.value?.focus()) })
const recordingSetup = ref(false)
const recordingTrigger = ref<HTMLButtonElement | null>(null)
const recordingAnchor = ref<{ left: number; right: number; top: number; bottom: number } | null>(null)
function openRecordingMenu() {
  const rect = recordingTrigger.value?.getBoundingClientRect()
  if (rect) recordingAnchor.value = { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom }
  error.value = ''; recordingSetup.value = true
}
watch(recordingSetup, open => { if (!open && frame.value) void nextTick(() => recordingTrigger.value?.focus()) })
const recordingCapabilities = ref<RecorderCapabilities | null>(null)
const recordingCapabilityError = ref('')
const recordingFps = ref<30 | 60>(30)
const recordingEncoding = ref<EncodingPreference>('auto')
const recordingQuality = ref<RecordingQuality>('balanced')
// 空白使用自动码率；提交时才换算 bit/s，避免表单编辑影响已启动会话。
const recordingMbps = ref<string | number>('')

const recordingCountdown = ref<0 | 3 | 5>(0)
const captureCursor = ref(true)
const recordingEffects = ref(defaultInputEffects())
const saveScrollFrames = ref(true)
const viewport = ref({ width: innerWidth, height: innerHeight })
const annotating = ref(false)
const screenImage = shallowRef<HTMLImageElement | null>(null)
const imageReady = ref(false)
const pointer = ref<Point | null>(null)
const picking = ref(false)
const colorFormat = ref<ColorFormat>('hex')
const showMagnifier = computed(() => frame.value && imageReady.value && screenImage.value && pointer.value && !busy.value && !scrollSetup.value && !recordingSetup.value && (picking.value || (!annotating.value && !marked.value)))
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
const toolbar = ref<HTMLElement | null>(null)
const toolbarSize = ref({ width: 1100, height: 46 })
// 工具完整展开为一排；窄屏按实际内容宽度整体缩小，不把尾部操作藏进横向滚动区。
watch(toolbar, (element, _, cleanup) => {
  if (!element) return
  const observer = new ResizeObserver(() => { toolbarSize.value = { width: element.offsetWidth, height: element.offsetHeight } })
  observer.observe(element)
  cleanup(() => observer.disconnect())
})
const toolbarStyle = computed(() => {
  const r = selection.value
  if (!r) return {}
  const scale = Math.min(1, Math.max(1, viewport.value.width - 16) / toolbarSize.value.width)
  const width = toolbarSize.value.width * scale, height = toolbarSize.value.height * scale
  return { left: `${clamp(r.x, 8, Math.max(8, viewport.value.width - width - 8))}px`, top: `${clamp(r.y + r.height + 12, 8, Math.max(8, viewport.value.height - height - 8))}px`, transform: `scale(${scale})`, transformOrigin: 'top left' }
})
// 二级面板紧邻工具栏，空间不足时翻到选区上方，仍保持在屏幕内。
const recordingMenuStyle = computed(() => {
  const r = recordingAnchor.value
  if (!r) return {}
  const below = r.bottom + 8
  const top = clamp(below + 340 <= viewport.value.height ? below : r.top - 340, 8, Math.max(8, viewport.value.height - 348))
  return { left: `${clamp(r.right - 390, 8, Math.max(8, viewport.value.width - 398))}px`, top: `${top}px`, maxHeight: `${viewport.value.height - top - 8}px` }
})
// 长截图面板比录屏面板更小；靠近触发按钮，底部空间不足时向上展开。
const scrollMenuStyle = computed(() => {
  const r = scrollAnchor.value
  if (!r) return {}
  const top = clamp(r.bottom + 8 + 170 <= viewport.value.height ? r.bottom + 8 : r.top - 178, 8, Math.max(8, viewport.value.height - 178))
  return { left: `${clamp(r.right - 300, 8, Math.max(8, viewport.value.width - 308))}px`, top: `${top}px`, maxHeight: `${viewport.value.height - top - 8}px` }
})
// 所有图形保存在原图坐标系内；选区只负责裁剪和限制绘制区域。
function chooseTool(tool: AnnotationTool) { if (busy.value || editor.pending || editor.interacting) return; editor.setTool(tool); annotating.value = true }
function selectionDown(e: PointerEvent) { if (!annotating.value) down(e, 'move') }
function reselect() {
  if (busy.value) return
  editor.reset(); selection.value = null; gesture = null; annotating.value = false; error.value = ''; pointer.value = null; picking.value = false
}
/** 恢复只替换空白选区，已有标注、文字草稿和进行中的拖动都必须先完成。 */
function restoreSelection() {
  if (!recentRect.value || busy.value || picking.value || scrollSetup.value || gesture || marked.value || editor.pending || editor.interacting) return
  const rect = { ...recentRect.value }
  reselect(); selection.value = rect
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
  try { receiveScroll(await api.startScroll(frame.value.id, pixels.value, saveScrollFrames.value)); scrollSetup.value = false }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 使用与截图相同的原图像素选区，成功后录屏后端接管会话及快捷键。 */
async function startRecording() {
  if (!frame.value || !pixels.value || !valid.value || busy.value || marked.value || editor.pending || editor.interacting || gesture) return
  busy.value = true; error.value = ''
  try { await api.startRecording(frame.value.id, pixels.value, recordingConfig(pixels.value, recordingFps.value, captureCursor.value, recordingCountdown.value, recordingEncoding.value, recordingQuality.value, recordingEffects.value, recordingBitrate(recordingMbps.value))); recordingSetup.value = false; frame.value = null }
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
function receive(value: CaptureFrame) { busy.value = false; reselect(); scroll.value = null; scrollSetup.value = false; recordingSetup.value = false; imageReady.value = false; frame.value = value }
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
    // 保存调用前的快照；取消对话框或导出失败均不覆盖上次成功的范围。
    const image = { width: frame.value.width, height: frame.value.height }
    const rect = { ...pixels.value }
    const result = await api[action](frame.value.id, rect, editor.history.present)
    if (result !== false) {
      recentSelection.value = writeRecentSelection(rect, image)
      frame.value = null
    }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(e: KeyboardEvent) {
  if (scroll.value) return
  if (recordingSetup.value) {
    if (e.key === 'Escape' && !busy.value) { e.preventDefault(); recordingSetup.value = false }
    return
  }
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
  if (!command && !e.altKey && !e.repeat && e.key.toLowerCase() === 'r') { e.preventDefault(); restoreSelection(); return }
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
  void api.recordingCapabilities().then(value => { if (!disposed) recordingCapabilities.value = value })
    .catch(e => { if (!disposed) recordingCapabilityError.value = String(e) })
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
  <div v-if="scroll" />
  <div v-else class="capture-overlay" @pointerdown.capture="pickDown" @pointerdown="down($event)" @pointermove.capture="drag" @pointerleave="pointer = null" @pointerup="up" @pointercancel="abortGesture(); pointer = null" @contextmenu.prevent="contextMenu">
    <img v-if="frame" :key="frame.id" ref="screenImage" class="screen-image" :src="frame.dataUrl" alt="当前屏幕截图" draggable="false" @load="loaded" @error="imageFailed" />
    <div v-if="!selection" class="initial-shade" />
    <div v-if="!selection && !picking" class="capture-tip">拖动框选屏幕 <span>Esc / 右键取消 · Ctrl / ⌘ + A 全屏 · I 取色 · R 最近选区</span></div>
    <div v-if="frame" class="capture-modebar" @pointerdown.stop @contextmenu.stop.prevent>
      <button v-if="!picking" :disabled="!recentRect || busy || marked || editor.pending || editor.interacting" :title="recentRect ? '恢复上次成功复制、保存或贴图的选区（R）' : '暂无最近选区，或当前屏幕分辨率已变化'" @click="restoreSelection"><Scan :size="15" />最近选区 R</button>
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
    <div v-if="valid && !picking" ref="toolbar" class="capture-toolbar annotation-toolbar" :style="toolbarStyle" @pointerdown.stop @contextmenu.stop.prevent>
      <AnnotationToolbar :editor="editor" :disabled="busy" :active="annotating" @tool="chooseTool">
        <template #leading><button :disabled="busy || editor.pending || editor.interacting || marked" :aria-pressed="!annotating" title="调整选区" aria-label="调整选区" @click="annotating = false"><Move :size="16" /></button></template>
        <template #actions>
          <button :disabled="busy || editor.pending || editor.interacting" title="清空标注并重新框选" aria-label="清空标注并重新框选" @click="reselect"><RotateCcw :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" title="选择整个屏幕 Ctrl/⌘+A" aria-label="选择整个屏幕" @click="selectAll"><Monitor :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" ref="scrollTrigger" title="长截图选项" aria-label="长截图选项" aria-haspopup="dialog" :aria-expanded="scrollSetup" @click="openScrollMenu"><ScrollCaptureIcon :size="16" /></button>
          <button ref="recordingTrigger" :disabled="busy || editor.pending || editor.interacting || marked || !recordingCapabilities?.available" :title="recordingCapabilities?.reason ?? (recordingCapabilityError || (recordingCapabilities ? '录屏选项' : '正在检查录屏能力…'))" aria-label="录屏选项" aria-haspopup="dialog" :aria-expanded="recordingSetup" @click="openRecordingMenu"><Video :size="16" /></button>
        <button :disabled="busy || editor.pending || editor.interacting" title="贴图 Ctrl/⌘+P" aria-label="贴图" @click="finish('pin')"><Pin :size="16" /></button>
        <button :disabled="busy || editor.pending || editor.interacting" title="保存图片 Ctrl/⌘+S" aria-label="保存图片" @click="finish('save')"><Save :size="16" /></button>
        <button class="copy-button" :disabled="busy || editor.pending || editor.interacting" :title="busy ? '处理中…' : '复制图片 Enter'" :aria-label="busy ? '处理中' : '复制图片'" :aria-busy="busy" @click="finish('copy')"><Copy :size="16" /></button>
        <button :disabled="busy || editor.pending" title="取消截图" aria-label="取消截图" @click="cancel"><X :size="16" /></button>
        </template>
      </AnnotationToolbar>
    </div>
    <CaptureMagnifier v-if="showMagnifier && screenImage && pointer && frame" :image="screenImage" :point="pointer" :viewport="viewport" :image-size="frame" :show-color="picking" />
    <div v-if="recordingSetup" class="recording-menu-shade" @pointerdown.self.stop="!busy && (recordingSetup = false)" @pointermove.stop @contextmenu.stop.prevent>
      <RecordingSetupMenu v-model:fps="recordingFps" v-model:encoding="recordingEncoding" v-model:quality="recordingQuality" v-model:mbps="recordingMbps" v-model:countdown="recordingCountdown" v-model:cursor="captureCursor" v-model:effects="recordingEffects" :busy="busy" :error="error" :style="recordingMenuStyle" @confirm="startRecording" @close="recordingSetup = false" />
    </div>
    <div v-if="scrollSetup" class="recording-menu-shade" @pointerdown.self.stop="!busy && (scrollSetup = false)" @pointermove.stop @contextmenu.stop.prevent>
      <ScrollSetupMenu v-model:save-frames="saveScrollFrames" :busy="busy" :error="error" :style="scrollMenuStyle" @confirm="startScroll" @close="scrollSetup = false" />
    </div>
    <div v-if="error && !scrollSetup && !recordingSetup" class="overlay-error" role="alert" @pointerdown.stop>{{ error }}</div>
  </div>
</template>

<style scoped>
.recording-menu-shade { position: absolute; inset: 0; z-index: 6; cursor: default; }
.capture-toolbar.annotation-toolbar { max-width: none; }
.capture-toolbar :deep(.annotation-tools) { width: max-content; max-width: none; }
.capture-toolbar :deep(.annotation-tool-row) { overflow: visible; }
.capture-modebar { position: absolute; top: 8px; left: 8px; z-index: 4; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; max-width: calc(100vw - 16px); border-radius: 6px; padding: 6px 8px; background: #242330ed; color: white; font-size: 12px; cursor: default; }
.capture-modebar button { display: flex; align-items: center; gap: 5px; padding: 3px; }
.capture-modebar button[aria-pressed=true] { color: #c4b5fd; }
.capture-modebar button:disabled { opacity: .5; }
.capture-modebar select { background: #3f3f46; color: white; border-radius: 3px; padding: 3px; }
.capture-modebar span { color: #d4d4d8; }
</style>
