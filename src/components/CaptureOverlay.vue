<script setup lang="ts">
import { handleEscape } from '@/lib/escape'
import { t, te } from '@/lib/i18n'
import { convertFileSrc } from '@tauri-apps/api/core'
import ScrollCaptureIcon from './ScrollCaptureIcon.vue'
import QrResultCard from './QrResultCard.vue'
import OcrResultCard from './OcrResultCard.vue'
import ImageTranslation from './ImageTranslation.vue'
import type { OcrResult } from '@/lib/ai'
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { Copy, Save, X, Monitor, RotateCcw, Scan, Move, Pin, Video, QrCode, ScanText, Languages, AppWindow, GripVertical, Check, MousePointer2, Volume2, VolumeX, Mic, MicOff, ArrowLeft } from 'lucide-vue-next'
import { api, onFrame, onCaptureClosed, onCapturePin, onScroll, type CaptureFrame, type ColorFormat, type ScrollView, type QrResult } from '@/lib/api'
import { clamp, move, normalize, resize, toPixels, nudgeSelection, type Handle, type Point, type Rect } from '@/lib/selection'
import { useAnnotations, type AnnotationTool } from '@/lib/use-annotations'
import AnnotationWorkspace from './AnnotationWorkspace.vue'
import RecordingAnnotations from './RecordingAnnotations.vue'
import { annotationToolbarPosition, useFloatingToolbar } from '@/lib/toolbar-layout'
import CaptureMagnifier from './CaptureMagnifier.vue'
import { waitForCapturePaint } from '@/lib/capture-paint'
import { magnifierSample } from '@/lib/magnifier'
import { newerScroll } from '@/lib/scroll-capture'
import { defaultInputEffects, recordingBitrate, recordingConfig, recordingRegion, type RecordingSelection, type RecordingFormat, type RecordingFps, type EncodingPreference, type RecordingQuality, type RecorderCapabilities } from '@/lib/recording'
import RecordingSetupMenu from './RecordingSetupMenu.vue'
import RecordingSourceSelect from './RecordingSourceSelect.vue'
import { targetAt, fromPixels, snapPoint, displayBounds } from '@/lib/capture-targets'
import { readRecentSelection, restoreRecentSelection, writeRecentSelection } from '@/lib/recent-selection'

// 截图帧是不可变快照，大量窗口目标不需要逐项响应式代理。
const frame = shallowRef<CaptureFrame | null>(null)
// 选区确认后仅替换背景和运行状态，继续使用当前 capture WebView，不弹出中间控制窗口。
const liveRecordingId = ref<number | null>(null)
watch(liveRecordingId, id => document.documentElement.classList.toggle('recording-live', id !== null), { flush: 'sync' })
// 只传会话编号与版本；大图通过受限协议读取，URL 不包含本地文件路径。
const imageUrl = computed(() => frame.value ? `${convertFileSrc(String(frame.value.id), 'capture-image')}/${frame.value.previewRevision}.png` : '')
const selection = ref<Rect | null>(null)
const selectedTarget = ref<number | null>(null)
const windowsOnly = ref(false)
const hoverTarget = computed(() => {
  if (!frame.value || !pointer.value || selection.value || busy.value || (!recordingMode.value && frame.value.snapEnabled === false)) return null
  return targetAt(frame.value.targets, { x: pointer.value.x * frame.value.width / viewport.value.width,
    y: pointer.value.y * frame.value.height / viewport.value.height }, windowsOnly.value)
})
const hoverStyle = computed(() => {
  if (!frame.value || hoverTarget.value === null) return {}
  const rect = fromPixels(frame.value.targets[hoverTarget.value].rect, frame.value, viewport.value)
  return { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` }
})
/** 原生抓取失败保留原图与选区；成功后等待图片加载再显示蒙层。 */
async function captureNative() {
  if (!frame.value || selectedTarget.value === null || busy.value || marked.value) return
  busy.value = true; error.value = ''
  const id = frame.value.id
  try { const next = await api.nativeWindow(id, selectedTarget.value); if (!disposed && frame.value?.id === id) frame.value = next }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}

const recentSelection = ref(readRecentSelection())
const recentRect = computed(() => frame.value ? restoreRecentSelection(recentSelection.value, frame.value, viewport.value) : null)
const busy = ref(false)
const qrResult = ref<QrResult | null>(null)
const ocrResult = ref<OcrResult | null>(null)
// 识别与翻译共享可编辑草稿，切换面板不重复 OCR，也不丢失修正后的文字。
const ocrDraft = ref('')
const ocrEntryWasTranslation = ref(false)
const translationMode = ref(false)
const translationTrigger = ref<HTMLButtonElement | null>(null)
const ocrTrigger = ref<HTMLButtonElement | null>(null)
function closeOcr() {
  ocrResult.value = null
  ocrDraft.value = ''
  void nextTick(() => (ocrEntryWasTranslation.value ? translationTrigger.value : ocrTrigger.value)?.focus())
}
async function recognizeText(translate = false) {
  if (!frame.value || !pixels.value || busy.value || editor.pending || editor.interacting || gesture.value) return
  const id = frame.value.id
  translationMode.value = translate
  ocrEntryWasTranslation.value = translate
  busy.value = true; error.value = ''
  try {
    const text = await api.recognizeCapture(id, pixels.value)
    if (frame.value?.id === id) { ocrDraft.value = text.fullText; ocrResult.value = text }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
watch(() => frame.value?.id, () => { qrResult.value = null; ocrResult.value = null; ocrDraft.value = '' })
/** 主动识别保留截图现场；结果面板隔离选区拖动与截图快捷键。 */
async function recognizeQr() {
  if (!frame.value || !pixels.value || busy.value || editor.pending || editor.interacting || gesture.value) return
  busy.value = true; error.value = ''
  try { qrResult.value = await api.recognizeQr(frame.value.id, pixels.value, editor.history.present) }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
const error = ref('')
const scroll = ref<ScrollView | null>(null)
// 录屏使用独立准备条，截图标注与导出操作在此模式下全部隐藏。
const recordingMode = ref(false)
const recordingTrigger = ref<HTMLButtonElement | null>(null)
const recordingConfirmTrigger = ref<HTMLButtonElement | null>(null)
/** F3 与截图中的录屏按钮使用同一模式；调整范围后保留录屏偏好。 */
function enterRecordingMode() {
  if (busy.value || editor.pending || editor.interacting) return
  annotating.value = false; error.value = ''; recordingMode.value = true
  void nextTick(() => recordingConfirmTrigger.value?.focus())
}
function leaveRecordingMode() {
  if (busy.value) return
  recordingMode.value = false; error.value = ''
  void nextTick(() => recordingTrigger.value?.focus())
}
async function selectedRecordingRegion() {
  if (!recordingMode.value || !valid.value) return
  await nextTick()
  recordingConfirmTrigger.value?.focus()
}
const recordingCapabilities = ref<RecorderCapabilities | null>(null)
const recordingCapabilityError = ref('')
const recordingSource = ref<RecordingSelection>('region')
/** 录屏仅选择屏幕区域或整台显示器，不再置前或绑定应用窗口。 */
function chooseRecordingSource(value: string) {
  if (!frame.value || busy.value || marked.value || editor.interacting) return
  if (value !== 'display' && value !== 'region') return
  selectedTarget.value = null
  if (value === 'display') selection.value = { ...activeDisplay.value }
  recordingSource.value = value
}
const recordingFormat = ref<RecordingFormat>('mp4')
const recordingFps = ref<RecordingFps>(60)
watch(recordingCapabilities, capabilities => {
  if (capabilities?.formats.length && !capabilities.formats.includes(recordingFormat.value)) recordingFormat.value = capabilities.formats[0]!
})
// 准备条和设置面板共用格式；切换时不能把 MP4 帧率带入 GIF。
watch(recordingFormat, format => { recordingFps.value = format === 'gif' ? 10 : 60 })
const recordingEncoding = ref<EncodingPreference>('auto')
const recordingQuality = ref<RecordingQuality>('balanced')
// 空白使用自动码率；提交时才换算 bit/s，避免表单编辑影响已启动会话。
const recordingMbps = ref<string | number>('')

const recordingCountdown = ref<0 | 3 | 5>(0)
const captureCursor = ref(true)
// 每次提交显式附带声音偏好，默认不采集；能力尚未加载时保持禁用。
const recordingSystemAudio = ref(false)
const recordingMicrophone = ref(false)
const recordingEffects = ref(defaultInputEffects())
const viewport = ref({ width: innerWidth, height: innerHeight })
const annotating = ref(false)
const screenImage = shallowRef<HTMLImageElement | null>(null)
const imageReady = ref(false)
const pointer = ref<Point | null>(null)
const relativeCoordinates = ref(false)
const colorCopied = ref(false)
const colorFormat = ref<ColorFormat>('rgb')
const showMagnifier = computed(() => frame.value && imageReady.value && screenImage.value && pointer.value && !busy.value && !recordingMode.value && (!annotating.value && !marked.value))
const editor = useAnnotations(() => pixels.value ?? { x: 0, y: 0, width: 0, height: 0 }, () => imageUrl.value, () => busy.value)
const marked = computed(() => editor.history.present.length > 0)
const handles: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']
// 手势使用响应式状态，工具栏在框选、移动或缩放结束后才显示。
const gesture = shallowRef<{ start: Point; initial: Rect; mode: 'draw' | 'move' | Handle; target: number | null } | null>(null)
let off: (() => void) | undefined
let offClosed: (() => void) | undefined
// current() 的旧响应或迟到 ready 事件不能让已关闭的大图重新驻留。
let closedThroughId = 0
let offScroll: (() => void) | undefined
let offPin: (() => void) | undefined
let disposed = false
const pixels = computed(() => selection.value && frame.value ? toPixels(selection.value, viewport.value, frame.value) : null)
const valid = computed(() => pixels.value && pixels.value.width > 0 && pixels.value.height > 0)
const boxStyle = computed(() => selection.value ? { left: `${selection.value.x}px`, top: `${selection.value.y}px`, width: `${selection.value.width}px`, height: `${selection.value.height}px` } : {})
const activeDisplay = computed(() => {
  if (!frame.value) return { x: 0, y: 0, ...viewport.value }
  const p = selection.value ? { x: selection.value.x + selection.value.width / 2, y: selection.value.y + selection.value.height / 2 } : pointer.value
  return displayBounds(frame.value.displays, { x: frame.value.originX, y: frame.value.originY },
    p ? { x: p.x * frame.value.width / viewport.value.width, y: p.y * frame.value.height / viewport.value.height } : { x: -frame.value.originX, y: -frame.value.originY }, frame.value, viewport.value)
})
const pointerDisplay = computed(() => frame.value && pointer.value ? displayBounds(frame.value.displays,
  { x: frame.value.originX, y: frame.value.originY },
  { x: pointer.value.x * frame.value.width / viewport.value.width, y: pointer.value.y * frame.value.height / viewport.value.height }, frame.value, viewport.value) : activeDisplay.value)
const dialogStyle = computed(() => ({ position: 'absolute' as const,
  left: `${activeDisplay.value.x + activeDisplay.value.width / 2}px`,
  top: `${activeDisplay.value.y + activeDisplay.value.height / 2}px`, transform: 'translate(-50%, -50%)',
  maxWidth: `${activeDisplay.value.width - 32}px`, maxHeight: `${activeDisplay.value.height - 32}px` }))
const toolbar = ref<HTMLElement | null>(null)
const { style: floatingToolbarStyle, start: floatToolbar, reset: resetToolbarPosition } = useFloatingToolbar(toolbar)
const toolbarSize = ref({ width: 800, height: 46 })
// 按实际尺寸避让屏幕边缘；窄屏允许横向滚动，保留字号和按钮命中区域。
watch(toolbar, (element, _, cleanup) => {
  if (!element) return
  const observer = new ResizeObserver(() => { toolbarSize.value = { width: element.offsetWidth, height: element.offsetHeight } })
  observer.observe(element)
  cleanup(() => observer.disconnect())
})
const toolbarStyle = computed(() => {
  const r = selection.value
  // F3 进入录屏时，即使还没框选也显示来源列表，可以直接选择应用窗口。
  if (!r) return recordingMode.value ? annotationToolbarPosition({ x: activeDisplay.value.x + 16, y: activeDisplay.value.y + 64, width: 0, height: 0 }, activeDisplay.value, toolbarSize.value) : {}
  return annotationToolbarPosition(r, activeDisplay.value, toolbarSize.value)
})
// 所有图形保存在原图坐标系内；选区只负责裁剪和限制绘制区域。
function chooseTool(tool: AnnotationTool) { if (busy.value || editor.pending || editor.interacting) return; editor.setTool(tool); annotating.value = true }
function selectionDown(e: PointerEvent) { if (!annotating.value) down(e, 'move') }
function reselect() {
  if (busy.value) return
  editor.reset(); selectedTarget.value = null; selection.value = null; gesture.value = null; annotating.value = false; error.value = ''; pointer.value = null; colorCopied.value = false
}
/** 恢复只替换空白选区，已有标注、文字草稿和进行中的拖动都必须先完成。 */
function restoreSelection() {
  if (!recentRect.value || busy.value || gesture.value || marked.value || editor.pending || editor.interacting) return
  const rect = { ...recentRect.value }
  reselect(); selection.value = rect
  void selectedRecordingRegion()
}
function receiveScroll(value: ScrollView) {
  const next = newerScroll(scroll.value, value, frame.value?.id)
  if (next && !next.collecting) scrollClosed()
  else scroll.value = next
}
/** 只在后端确认结束后调用；保存取消或失败时保留标注及撤销栈。 */
function releaseFrame(id = frame.value?.id) {
  if (id !== undefined) closedThroughId = Math.max(closedThroughId, id)
  // 旧导出响应可能晚于新截图 ready 到达，不能清掉新会话。
  if (frame.value && frame.value.id !== id) return
  frame.value = null
  editor.reset()
  selection.value = null; gesture.value = null; pointer.value = null
  imageReady.value = false; selectedTarget.value = null; annotating.value = false; recordingMode.value = false
  qrResult.value = null; ocrResult.value = null
  ocrDraft.value = ''
}
function scrollClosed() { scroll.value = null; releaseFrame() }
async function startScroll() {
  if (!frame.value || !pixels.value || !valid.value || busy.value || marked.value || editor.pending || editor.interacting || gesture.value) return
  busy.value = true; error.value = ''
  // 滚动截图仅输出拼接结果；排错使用应用日志，不再额外保存原始帧。
  const id = frame.value.id
  try {
    await api.startScroll(id, pixels.value, false, 'auto', 'white')
    // 独立预览窗口接管进度；隐藏浮层不保留整桌面图片或标注历史。
    releaseFrame(id)
    if (!frame.value) scroll.value = null
  }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 确认原图像素选区并进入 Ready；后端接管后等待开始按钮或空格，不自动采集。 */
async function startRecording() {
  if (!recordingMode.value || !recordingCapabilities.value?.available) return
  if (!frame.value || !pixels.value || !valid.value || busy.value || editor.pending || editor.interacting || gesture.value) return
  const id = frame.value.id
  let prepared = false
  busy.value = true; error.value = ''
  try {
    const rect = recordingRegion(pixels.value, recordingSource.value, frame.value.displays, { x: frame.value.originX, y: frame.value.originY })
    const config = recordingConfig(rect, recordingFps.value, captureCursor.value, recordingCountdown.value, recordingEncoding.value, recordingQuality.value, recordingEffects.value, recordingFormat.value === 'gif' ? null : recordingBitrate(recordingMbps.value), recordingFormat.value, recordingSystemAudio.value && !!recordingCapabilities.value?.systemAudio, recordingMicrophone.value && !!recordingCapabilities.value?.microphone)
    config.source = 'selectedMonitor'
    await api.startRecording(id, rect, config, undefined, editor.history.present)
    prepared = true
    liveRecordingId.value = id
    releaseFrame(id)
    await nextTick()
    await api.setRecordingAnnotationMode(id, true)
  }
  catch (e) {
    // 这里只进入 Ready，开始由录屏工具栏按钮或空格触发；接管失败则释放会话。
    if (prepared) { try { await api.controlRecording(id, 'cancel') } catch { /* 保留最初的失败原因。 */ } }
    error.value = String(e)
  }
  finally { busy.value = false }
}
async function copyColor(format = colorFormat.value) {
  if (!frame.value || !pointer.value || !imageReady.value || busy.value || editor.pending || editor.interacting) return
  const pixel = magnifierSample(pointer.value, viewport.value, frame.value).pixel
  busy.value = true; error.value = ''
  try { await api.copyColor(frame.value.id, pixel, format); colorCopied.value = true }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function point(e: PointerEvent): Point { return { x: clamp(e.clientX, 0, viewport.value.width), y: clamp(e.clientY, 0, viewport.value.height) } }
function down(e: PointerEvent, mode: 'draw' | 'move' | Handle = 'draw') {
  if (busy.value || e.button !== 0 || !frame.value) return
  // 有标注时固定选区，避免调整裁剪范围导致已画内容跳动；“重新框选”显式清空。
  if (marked.value || editor.interacting || annotating.value) return
  editor.reset()
  const p = point(e)
  pointer.value = p
  const target = mode === 'draw' && !e.shiftKey ? hoverTarget.value : null
  selectedTarget.value = null
  if (mode === 'draw') selection.value = { ...p, width: 0, height: 0 }
  if (!selection.value) return
  gesture.value = { start: p, initial: { ...selection.value }, mode, target }
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function drag(e: PointerEvent) {
  colorCopied.value = false
  // 工具栏不需要像素辅助；捕获阶段监听可覆盖选区控制点的事件。
  pointer.value = (e.target as Element).closest('.capture-toolbar, .capture-modebar, .overlay-error') ? null : point(e)
  if (!gesture.value) return
  let p = point(e)
  if (!e.shiftKey && frame.value && (recordingMode.value || frame.value.snapEnabled !== false) && gesture.value.mode !== 'move') {
    const pixel = { x: p.x * frame.value.width / viewport.value.width, y: p.y * frame.value.height / viewport.value.height }
    const index = targetAt(frame.value.targets, pixel, windowsOnly.value)
    if (index !== null) {
      const snapped = snapPoint(pixel, frame.value.targets[index].rect, { x: 6 * frame.value.width / viewport.value.width, y: 6 * frame.value.height / viewport.value.height })
      p = { x: snapped.x * viewport.value.width / frame.value.width, y: snapped.y * viewport.value.height / frame.value.height }
    }
  }
  const { start, initial, mode } = gesture.value
  selection.value = mode === 'draw' ? normalize(start, p) : mode === 'move' ? move(initial, { x: p.x - start.x, y: p.y - start.y }, viewport.value) : resize(initial, mode, p)
}
function up(e: PointerEvent) {
  const current = gesture.value
  if (current?.mode === 'draw' && current.target !== null && frame.value && !e.shiftKey
    && Math.hypot(e.clientX - current.start.x, e.clientY - current.start.y) < 4) {
    selectedTarget.value = current.target
    selection.value = fromPixels(frame.value.targets[current.target].rect, frame.value, viewport.value)
  }
  const drawn = gesture.value?.mode === 'draw'
  gesture.value = null
  if (drawn) void selectedRecordingRegion()
}
function abortGesture() { gesture.value = null }
function selectAll() { selectedTarget.value = null; if (!busy.value && !marked.value && !editor.interacting) { editor.reset(); selection.value = { x: 0, y: 0, ...viewport.value }; void selectedRecordingRegion() } }
function receive(value: CaptureFrame) {
  if (disposed || value.id <= closedThroughId || (frame.value && value.id < frame.value.id)) return
  liveRecordingId.value = null
  busy.value = false; reselect(); windowsOnly.value = false; scroll.value = null; imageReady.value = false; recordingMode.value = !!value.recording; frame.value = value
}
/** load 只表示资源可用；先解码并等待 Vue 更新与绘制机会，再显示原生蒙层。 */
async function loaded(event: Event) {
  const current = frame.value
  const image = event.target as HTMLImageElement
  if (!current || image !== screenImage.value) return
  const url = imageUrl.value
  const isCurrent = () => !disposed && frame.value?.id === current.id && imageUrl.value === url && screenImage.value === image
  try { await image.decode() }
  catch {
    if (isCurrent()) await imageFailed()
    return
  }
  if (!isCurrent()) return
  imageReady.value = true
  await nextTick()
  await waitForCapturePaint()
  // 等待期间可能取消或换图，旧图片绝不能重新唤起窗口。
  if (!isCurrent()) return
  try { await api.ready(current.id) }
  catch (e) { if (isCurrent()) error.value = String(e) }
}
async function cancel() {
  if (!frame.value || busy.value || editor.pending) return
  const id = frame.value.id
  busy.value = true
  try { await api.cancel(id); releaseFrame(id) } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function finish(action: 'copy' | 'save' | 'pin') {
  if (!frame.value || !valid.value || !pixels.value || busy.value || qrResult.value || ocrResult.value !== null || editor.pending || editor.interacting || gesture.value) return
  const id = frame.value.id
  busy.value = true; error.value = ''
  try {
    // 保存调用前的快照；取消对话框或导出失败均不覆盖上次成功的范围。
    const image = { width: frame.value.width, height: frame.value.height, layoutKey: frame.value.layoutKey }
    const rect = { ...pixels.value }
    const result = await api[action](id, rect, editor.history.present)
    if (result !== false) {
      recentSelection.value = writeRecentSelection(rect, image)
      releaseFrame(id)
    }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(e: KeyboardEvent) {
  if (e.defaultPrevented || e.isComposing || e.keyCode === 229) return
  if (liveRecordingId.value !== null) return
  if (qrResult.value || ocrResult.value !== null) return
  if (scroll.value) return
  // 行内设置默认可见，不作为模态层消费 Esc；真正的下拉菜单自行关闭当前层。
  // 先退出窗口，再判断输入框和标注快捷键，避免 Esc 只取消文字或笔画。
  if (handleEscape(e, () => { void cancel() })) return
  if ((e.target as HTMLElement).closest('.recording-inline-options')) return
  if ((e.target as HTMLElement).closest('input, textarea, select')) return
  if (e.key === 'Alt') windowsOnly.value = true
  const command = e.ctrlKey || e.metaKey
  if (annotating.value && editor.key(e)) return
  // 录屏模式拦截截图导出快捷键，避免 Enter / Ctrl+C 意外复制静态图片。
  if (recordingMode.value) {
    if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'BUTTON') { e.preventDefault(); if (!e.repeat) void startRecording(); return }
    if (command && ['c', 's', 'p'].includes(e.key.toLowerCase())) { e.preventDefault(); return }
  }
  // 取色快捷键仅在放大镜可用时生效；复制后保留选区，修饰键组合仍用于截图操作。
  if (showMagnifier.value && !gesture.value && !command && !e.altKey && !e.repeat) {
    if (e.key === 'Shift') { e.preventDefault(); colorFormat.value = colorFormat.value === 'rgb' ? 'hex' : 'rgb'; colorCopied.value = false; return }
    if (e.key.toLowerCase() === 'c') { e.preventDefault(); void copyColor(); return }
    if (e.key.toLowerCase() === 'p') { e.preventDefault(); relativeCoordinates.value = !relativeCoordinates.value; return }
  }
  // 保留按钮的 Enter / 空格行为，恢复选区后按钮仍聚焦也能使用上方的取色快捷键。
  const target = e.target as HTMLElement
  if (['INPUT', 'SELECT', 'BUTTON'].includes(target.tagName) && e.key !== 'Escape' && !command) return
  if (!command && !e.altKey && !e.repeat && e.key.toLowerCase() === 'r') { e.preventDefault(); restoreSelection(); return }
  // 方向键允许长按连续微调；绘制、导出或已有标注时不改变裁剪范围。
  const directions: Record<string, Point> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } }
  const direction = directions[e.key]
  if (direction && !command) {
    e.preventDefault()
    if (busy.value || gesture.value || editor.interacting || marked.value || annotating.value || !valid.value || !selection.value || !frame.value) return
    const step = e.shiftKey ? 10 : 1
    selectedTarget.value = null
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
function keyUp(e: KeyboardEvent) { if (e.key === 'Alt') windowsOnly.value = false }
function resized() { viewport.value = { width: innerWidth, height: innerHeight }; if (!scroll.value && liveRecordingId.value === null) reselect() }
async function imageFailed() {
  error.value = '截图预览加载失败，请重新截图。'
  await cancel()
}
onMounted(async () => {
  void api.recordingCapabilities().then(value => { if (!disposed) recordingCapabilities.value = value })
    .catch(e => { if (!disposed) recordingCapabilityError.value = String(e) })
  window.addEventListener('keydown', key); window.addEventListener('keyup', keyUp); window.addEventListener('resize', resized)
  try {
    offClosed = await onCaptureClosed(id => {
      closedThroughId = Math.max(closedThroughId, id)
      if (frame.value?.id === id) { scroll.value = null; releaseFrame() }
    }, 'capture')
    if (disposed) { offClosed(); return }
    off = await onFrame(receive)
    if (disposed) { off(); return }
    offScroll = await onScroll(receiveScroll)
    if (disposed) { offScroll(); return }
    offPin = await onCapturePin(id => {
      // 复用工具栏的导出校验，编辑/拖动/滚动截图期间不提交，也不回退剪贴板。
      if (frame.value?.id === id && !scroll.value && !recordingMode.value) void finish('pin')
    })
    if (disposed) { offPin(); return }
    const current = await api.current()
    if (current && (!frame.value || current.id > frame.value.id)) receive(current)
    const currentScroll = await api.currentScroll()
    if (currentScroll && !disposed) receiveScroll(currentScroll)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { releaseFrame(); disposed = true; offClosed?.(); off?.(); offScroll?.(); offPin?.(); window.removeEventListener('keydown', key); window.removeEventListener('keyup', keyUp); window.removeEventListener('resize', resized) })
</script>

<template>
  <RecordingAnnotations v-if="liveRecordingId !== null" :session-id="liveRecordingId" :initial-editor="editor" @closed="liveRecordingId = null" />
  <div v-else-if="scroll" />
  <div v-else class="capture-overlay" @pointerdown="down($event)" @pointermove.capture="drag" @pointerleave="pointer = null" @pointerup="up" @pointercancel="abortGesture(); pointer = null" @contextmenu.prevent="cancel">
    <img v-if="frame" :key="frame.id" ref="screenImage" class="screen-image" :src="imageUrl" crossorigin="anonymous" :alt='t("当前屏幕截图")' draggable="false" @load="loaded" @error="imageFailed" />
    <!-- 截图和录屏共用选区入口：开始框选前保持原图，仅在形成选区后遮暗外围。 -->
    <div v-if="!selection" class="capture-tip" :style="{ left: `${activeDisplay.x + activeDisplay.width / 2}px`, top: `${activeDisplay.y + 64}px`, maxWidth: `${activeDisplay.width - 32}px`, whiteSpace: 'normal' }">{{ recordingMode ? t("拖动选择录屏范围") : frame?.targets.length ? t("点击吸附 · 拖动框选 · Alt 选窗口 · Shift 自由框选") : t("拖动框选屏幕") }} <span>{{ recordingMode ? t("Esc 退出 · Ctrl / ⌘ + A 全屏 · R 最近选区") : t("C 取色 · Esc 取消 · Ctrl / ⌘ + A 全屏 · R 最近选区") }}</span></div>
    <div v-if="frame" class="capture-modebar" :style="{ left: `${activeDisplay.x + 16}px`, top: `${activeDisplay.y + 16}px` }" @pointerdown.stop @contextmenu.stop.prevent>
      <button :disabled="!recentRect || busy || marked || editor.pending || editor.interacting" :title="recentRect ? t('恢复上次成功复制、保存或贴图的选区（R）') : t('暂无最近选区，或当前屏幕分辨率已变化')" @click="restoreSelection"><Scan :size="15" />{{ t("最近选区 R") }}</button>
    </div>
    <div v-if="hoverTarget !== null" class="snap-target" :style="hoverStyle" />
    <div v-if="selection" class="selection" :style="{ ...boxStyle, boxShadow: valid ? undefined : 'none', cursor: marked ? 'default' : 'move' }" @pointerdown.stop="selectionDown">
      <span class="size-label">{{ pixels?.width }} × {{ pixels?.height }} px · ({{ pixels?.x }}, {{ pixels?.y }})</span>
      <template v-if="!marked && !annotating"><span v-for="handle in handles" :key="handle" :class="['resize-handle', handle]" @pointerdown.stop="down($event, handle)" /></template>
    </div>
    <AnnotationWorkspace :editor="editor" :width="frame?.width ?? 1" :height="frame?.height ?? 1" :bounds="pixels ?? { x: 0, y: 0, width: 1, height: 1 }" :surface-visible="!!frame && !!pixels" :toolbar-visible="(!!valid || (!!frame && recordingMode)) && !gesture" :enabled="annotating && !busy" :active="annotating" :disabled="busy || !valid" :toolbar-style="[toolbarStyle, floatingToolbarStyle]" @toolbar-element="toolbar = $event" @tool="chooseTool" @float="floatToolbar" @reset-position="resetToolbarPosition">
        <template #leading><button :disabled="busy || editor.pending || editor.interacting || marked" :aria-pressed="!annotating" :title='t("调整选区")' :aria-label='t("调整选区")' @click="annotating = false"><Move :size="16" /></button></template>
        <template #actions>
          <button v-if="!recordingMode && selectedTarget !== null && frame?.targets[selectedTarget]?.kind === 'window'" :disabled="busy || marked || editor.pending || editor.interacting" :title='t("原生窗口截图")' :aria-label='t("原生窗口截图")' @click="captureNative"><AppWindow :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting" :title='t("清空标注并重新框选")' :aria-label='t("清空标注并重新框选")' @click="reselect"><RotateCcw :size="16" /></button>
          <button :disabled="busy || editor.pending || editor.interacting || marked" :title='t("选择整个屏幕 Ctrl/⌘+A")' :aria-label='t("选择整个屏幕")' @click="selectAll"><Monitor :size="16" /></button>
          <button v-if="!recordingMode" :disabled="busy || editor.pending || editor.interacting || marked" :title='t("开始滚动截图，手动上下左右滚动后停顿拼接")' :aria-label='t("开始滚动截图")' @click="startScroll"><ScrollCaptureIcon :size="16" /></button>
          <button ref="recordingTrigger" class="recording-mode-trigger" :disabled="busy || editor.pending || editor.interacting || !recordingCapabilities?.available" :title="recordingCapabilities?.reason ?? (recordingCapabilityError || (recordingCapabilities ? t(recordingMode ? '返回截图' : '切换到录屏') : t('正在检查录屏能力…')))" :aria-label="t(recordingMode ? '返回截图' : '展开录屏工具栏')" :aria-pressed="recordingMode" :aria-expanded="recordingMode" @click="recordingMode ? leaveRecordingMode() : enterRecordingMode()"><Video :size="16" /><span>{{ t("录屏") }}</span></button>
        <template v-if="!recordingMode">
        <button :disabled="busy || editor.pending || editor.interacting" :title='t("贴图 Ctrl/⌘+P")' :aria-label='t("贴图")' @click="finish('pin')"><Pin :size="16" /></button>
        <button ref="ocrTrigger" :disabled="busy || editor.pending || editor.interacting" :title='t("识别文字")' :aria-label='t("识别文字")' aria-haspopup="dialog" :aria-expanded="ocrResult !== null && !translationMode" @click="recognizeText(false)"><ScanText :size="16" /></button>
        <button ref="translationTrigger" :disabled="busy || editor.pending || editor.interacting" :title='t("截图翻译")' :aria-label='t("截图翻译")' aria-haspopup="dialog" :aria-expanded="ocrResult !== null && translationMode" @click="recognizeText(true)"><Languages :size="16" /></button>
        <button :disabled="busy || editor.pending || editor.interacting" :title='t("识别二维码")' :aria-label='t("识别二维码")' aria-haspopup="dialog" :aria-expanded="!!qrResult" @click="recognizeQr"><QrCode :size="16" /></button>
        <button :disabled="busy || editor.pending || editor.interacting" :title='t("保存图片 Ctrl/⌘+S")' :aria-label='t("保存图片")' @click="finish('save')"><Save :size="16" /></button>
        <button class="copy-button" :disabled="busy || editor.pending || editor.interacting" :title="busy ? t('处理中…') : t('复制图片 Enter')" :aria-label="busy ? t('处理中') : t('复制图片')" :aria-busy="busy" @click="finish('copy')"><Copy :size="16" /></button>
        </template>
        <button :disabled="busy || editor.pending" :title="recordingMode ? t('取消录屏') : t('取消截图')" :aria-label='t("取消截图")' @click="cancel"><X :size="16" /></button>
        </template>
      <template #footer>
      <div v-if="recordingMode" class="recording-ready-row" role="toolbar" :aria-label='t("录屏准备工具栏")'>
        <button class="recording-grip" :disabled="busy" :title='t("拖动工具栏；双击恢复位置")' :aria-label='t("拖动录屏工具栏")' @pointerdown="floatToolbar" @dblclick="resetToolbarPosition"><GripVertical :size="16" /></button>
        <button :disabled="busy" :title='t("返回截图")' :aria-label='t("返回截图")' @click="leaveRecordingMode"><ArrowLeft :size="16" /></button>
        <div class="recording-control-group" role="group" :aria-label="t('录制范围')">
        <RecordingSourceSelect :value="recordingSource" :disabled="busy || marked || editor.interacting" @select="chooseRecordingSource" />
        <select v-model="recordingFormat" :disabled="busy || !recordingCapabilities?.available" :aria-label='t("录屏格式")' :title='t("录屏格式")'><option v-for="format in recordingCapabilities?.formats ?? []" :key="format" :value="format">{{ format.toUpperCase() }}</option></select>
        <select v-model="recordingFps" :disabled="busy" :aria-label='t("录屏帧率")' :title='t("录屏帧率")'><option v-for="fps in (recordingFormat === 'gif' ? [10, 20] : [30, 60])" :key="fps" :value="fps">{{ fps }} FPS</option></select>
        </div>
        <div class="recording-control-group recording-toggles" role="group" :aria-label="t('录屏选项')">
        <button :disabled="busy" :aria-pressed="captureCursor" :title="`${t('录制鼠标指针')} · ${t(captureCursor ? '已启用' : '已关闭')}`" :aria-label='t("录制鼠标指针")' @click="captureCursor = !captureCursor"><MousePointer2 :size="16" /></button>
        <template v-if="recordingFormat === 'mp4'">
          <button :disabled="busy || !recordingCapabilities?.systemAudio" :aria-pressed="recordingSystemAudio" :title="recordingCapabilities?.systemAudio ? `${t('录制系统声音')} · ${t(recordingSystemAudio ? '已启用' : '已关闭')}` : t('系统声音录制不可用')" :aria-label='t("录制系统声音")' @click="recordingSystemAudio = !recordingSystemAudio"><component :is="recordingSystemAudio ? Volume2 : VolumeX" :size="16" /></button>
          <button :disabled="busy || !recordingCapabilities?.microphone" :aria-pressed="recordingMicrophone" :title="recordingCapabilities?.microphone ? `${t('录制麦克风')} · ${t(recordingMicrophone ? '已启用' : '已关闭')}` : t('麦克风录制不可用')" :aria-label='t("录制麦克风")' @click="recordingMicrophone = !recordingMicrophone"><component :is="recordingMicrophone ? Mic : MicOff" :size="16" /></button>
        </template>
        </div>
        <RecordingSetupMenu id="recording-inline-options" :format="recordingFormat" v-model:encoding="recordingEncoding" v-model:quality="recordingQuality" v-model:mbps="recordingMbps" v-model:countdown="recordingCountdown" v-model:effects="recordingEffects" :busy="busy" />
        <div class="recording-final-actions">
        <button :disabled="busy" :title='t("重新选择录屏范围")' :aria-label='t("重新选择录屏范围")' @click="reselect"><RotateCcw :size="16" /></button>
        <button ref="recordingConfirmTrigger" class="recording-confirm" :disabled="!valid || busy || editor.pending || editor.interacting || !recordingCapabilities?.available" :title="recordingCapabilities?.reason ?? (recordingCapabilityError || (recordingCapabilities ? t('确认范围，进入录制控制条') : t('正在检查录屏能力…')))" @click="startRecording"><Check :size="15" />{{ busy ? t("准备中…") : t("确认范围") }}</button>
        <button :disabled="busy" :title='t("取消录屏")' :aria-label='t("取消录屏")' @click="cancel"><X :size="16" /></button>
        </div>
      </div>
      </template>
    </AnnotationWorkspace>
    <CaptureMagnifier v-if="showMagnifier && screenImage && pointer && frame" :image="screenImage" :point="pointer" :viewport="viewport" :image-size="frame" :format="colorFormat" :relative="relativeCoordinates" :origin="pixels" :copied="colorCopied" :bounds="pointerDisplay" />
    <div v-if="ocrResult !== null" class="qr-shade" @pointerdown.stop @pointermove.stop @pointerup.stop @contextmenu.stop.prevent>
      <div :class="['qr-dialog', 'ocr-dialog', { 'translation-dialog': translationMode }]" :style="dialogStyle" role="dialog" aria-modal="true" :aria-label="translationMode ? t('文字翻译') : t('文字识别结果')">
        <ImageTranslation v-if="translationMode && frame && pixels" :key="ocrResult.requestId" :recognition="ocrResult" :source-text="ocrDraft" :image="{ dataUrl: imageUrl, width: frame.width, height: frame.height, crop: pixels }" @back="translationMode = false" @close="closeOcr" />
        <OcrResultCard v-else v-model:text="ocrDraft" @translate="translationMode = true" @close="closeOcr" />
      </div>
    </div>
    <div v-if="qrResult" class="qr-shade" @pointerdown.stop @pointermove.stop @pointerup.stop @contextmenu.stop.prevent>
      <div class="qr-dialog" :style="dialogStyle" role="dialog" aria-modal="true" :aria-label='t("二维码识别结果")'>
        <QrResultCard :capture-result="qrResult" @close="qrResult = null" @opened="cancel" />
      </div>
    </div>
    <div v-if="error" class="overlay-error" :style="{ left: `${activeDisplay.x + activeDisplay.width / 2}px`, bottom: 'auto', top: `${activeDisplay.y + 16}px`, maxWidth: `${activeDisplay.width - 32}px` }" role="alert" @pointerdown.stop>{{ te(error) }}</div>
  </div>
</template>

<style scoped>
.capture-overlay .recording-mode-trigger[aria-pressed=true] { color:#2563eb; background:#dbeafe; }
/* 常用操作按用途分组；窄屏按组换行，确认和取消保持在同一组。 */
.recording-ready-row { border-top:1px solid #e1e4e8; padding-top:6px; display:flex; flex-wrap:wrap; align-items:center; gap:6px; min-height:38px; max-width:100%; font-size:13px; }
.recording-control-group { display:inline-flex; flex:none; align-items:center; gap:3px; padding:2px; border:1px solid #e1e4e8; border-radius:5px; background:#f6f7f8; }
.recording-ready-row select { height:28px; padding:2px 5px; min-width:0; background:transparent; color:inherit; border:0; border-radius:4px; font:inherit; cursor:pointer; }
.recording-ready-row select:hover:enabled { background:#e9edf2; }
.recording-ready-row :deep(.recording-source-trigger) { padding:3px 6px; }
.recording-toggles button[aria-pressed=true] { color:#2563eb; background:#dbeafe; box-shadow:inset 0 0 0 1px #bfdbfe; }
.recording-toggles button[aria-pressed=false] { color:#57606a; }
.recording-final-actions { display:inline-flex; align-items:center; gap:4px; margin-left:auto; }
.recording-ready-row .recording-confirm { min-width:88px; padding:3px 10px; background:#2563eb; color:white; font-weight:600; }
.recording-ready-row .recording-confirm:hover:enabled { background:#1d4ed8; }
.recording-grip { cursor:grab; touch-action:none; }
.recording-ready-row :focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
:global([data-theme=dark]) .recording-control-group { background:#272e37; border-color:#444c56; }
:global([data-theme=dark]) .recording-ready-row { border-color:#444c56; }
:global([data-theme=dark]) .recording-toggles button[aria-pressed=true] { color:#bfdbfe; background:#263f60; box-shadow:inset 0 0 0 1px #45658b; }
:global([data-theme=dark]) .recording-toggles button[aria-pressed=false] { color:#a8b3c2; }
:global([data-theme=dark]) .recording-ready-row select:hover:enabled { background:#303f56; }
.qr-shade { position: absolute; inset: 0; z-index: 10; display: grid; place-items: center; cursor: default; background: #0003; }
.qr-dialog { width: min(440px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow-y: auto; color: #20232b; }
.ocr-dialog { width: min(560px, calc(100vw - 32px)); }
.translation-dialog { width: min(920px, calc(100vw - 32px)); }

.capture-modebar { position: absolute; top: 8px; left: 8px; z-index: 4; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; max-width: calc(100vw - 16px); border-radius: 6px; padding: 6px 8px; background: #242330ed; color: white; font-size: 12px; cursor: default; }
.capture-modebar button { display: flex; align-items: center; gap: 5px; padding: 3px; }
.capture-modebar button:disabled { opacity: .5; }
.snap-target { position: absolute; outline: 2px solid #60a5fa; pointer-events: none; }
</style>
